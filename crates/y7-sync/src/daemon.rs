//! The `y7-syncd` daemon: runs the [`Engine`] and serves apps over a Unix socket.
//!
//! One daemon per user owns the device identity (enforced with a lock file in
//! the data directory), so two processes never run an endpoint with the same
//! key. It sleeps in `accept`/`read` when nothing happens, and only starts the
//! network once a device is paired.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use automerge::sync::{self, SyncDoc};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, watch};

use crate::doc::{Doc, write_atomic};
use crate::ipc::{Control, Frame, read_frame_async};
use crate::net::Shared;
use crate::{Engine, Hub, Listener, NOTES_DOC, valid_doc_id};

pub struct Daemon {
    engine: Arc<Engine>,
    socket: PathBuf,
    activated: bool,
    _lock: File,
}

/// Status changes are pushed to every connected app.
struct Events(watch::Sender<u64>);

impl Listener for Events {
    fn doc_changed(&self, _doc: &str) {}
    fn status_changed(&self) {
        self.0.send_modify(|v| *v += 1);
    }
}

impl Daemon {
    /// Takes ownership of `dir` (failing if another daemon has it), brings in
    /// Notes' data from `legacy` the first time, and starts serving `socket`.
    pub fn start(dir: PathBuf, socket: PathBuf, device_name: String, legacy: Option<&Path>) -> Result<Daemon> {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let lock = File::create(dir.join("daemon.lock"))?;
        if lock.try_lock().is_err() {
            bail!("another y7-syncd is already running for {}", dir.display());
        }
        if let Some(legacy) = legacy {
            if let Err(e) = migrate(&dir, legacy) {
                tracing::error!("migrating {} failed: {e:#}", legacy.display());
            }
        }
        let (listener, activated) = bind(&socket)?;
        let status = watch::channel(0u64).0;
        let engine = Arc::new(Engine::start(dir, device_name, Arc::new(Events(status.clone()))));
        let shared = engine.shared.clone();
        let handle = engine.runtime().clone();
        engine.runtime().spawn(async move {
            let listener = match tokio::net::UnixListener::from_std(listener) {
                Ok(l) => l,
                Err(e) => return tracing::error!("socket: {e}"),
            };
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        tokio::spawn(link(shared.clone(), handle.clone(), stream, status.subscribe()));
                    }
                    Err(e) => tracing::warn!("accept: {e}"),
                }
            }
        });
        Ok(Daemon { engine, socket, activated, _lock: lock })
    }

    /// The defaults: `data_dir()`, `socket_path()`, migrating `legacy_notes_dir()`.
    pub fn start_default() -> Result<Daemon> {
        Daemon::start(crate::data_dir(), crate::socket_path(), crate::device_name(), Some(&crate::legacy_notes_dir()))
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if !self.activated {
            let _ = std::fs::remove_file(&self.socket);
        }
        self.engine.shutdown();
    }
}

/// The listening socket: handed over by systemd (socket activation), or bound
/// at `path`, replacing a stale socket left by a daemon that died.
fn bind(path: &Path) -> Result<(std::os::unix::net::UnixListener, bool)> {
    use std::os::unix::net::{UnixListener, UnixStream};
    let activated = std::env::var("LISTEN_PID").ok().and_then(|p| p.parse::<u32>().ok()) == Some(std::process::id())
        && std::env::var("LISTEN_FDS").ok().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0) >= 1;
    if activated {
        use std::os::fd::FromRawFd;
        // SAFETY: systemd passes the listening socket as fd 3 (SD_LISTEN_FDS_START).
        let listener = unsafe { UnixListener::from_raw_fd(3) };
        listener.set_nonblocking(true)?;
        return Ok((listener, true));
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            bail!("{} is already being served", path.display());
        }
        let _ = std::fs::remove_file(path);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    listener.set_nonblocking(true)?;
    Ok((listener, false))
}

/// Serves one app until it disconnects.
async fn link(shared: Arc<Shared>, rt: tokio::runtime::Handle, stream: tokio::net::UnixStream, mut status: watch::Receiver<u64>) {
    let (mut rd, mut wr) = stream.into_split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let writer = tokio::spawn(async move {
        while let Some(bytes) = out_rx.recv().await {
            if wr.write_all(&bytes).await.is_err() {
                break;
            }
        }
    });
    // Reading a frame isn't cancel-safe, so it gets its own task.
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(16);
    let reader = tokio::spawn(async move {
        while let Ok(frame) = read_frame_async(&mut rd).await {
            if tx.send(frame).await.is_err() {
                break;
            }
        }
    });

    let send = |f: Frame| {
        let _ = out.send(f.encode());
    };
    send(Frame::Control(Control::Status { status: shared.status() }));
    let mut changes = shared.changes.subscribe();
    let mut open: BTreeMap<String, sync::State> = BTreeMap::new();
    let result: Result<()> = async {
        loop {
            for (id, state) in open.iter_mut() {
                let doc = shared.doc(id);
                while let Some(msg) = doc.lock().unwrap().am().sync().generate_sync_message(state) {
                    send(Frame::Sync { doc: id.clone(), msg: msg.encode() });
                }
            }
            tokio::select! {
                frame = rx.recv() => {
                    let Some(frame) = frame else { return Ok(()) };
                    match Frame::decode(&frame)? {
                        Frame::Sync { doc, msg } => {
                            let Some(state) = open.get_mut(&doc) else { continue };
                            if shared.receive(&doc, state, sync::Message::decode(&msg)?)? {
                                shared.local_changed();
                            }
                        }
                        Frame::Control(Control::Open { doc }) if valid_doc_id(&doc) => {
                            let bytes = shared.doc(&doc).lock().unwrap().save_bytes();
                            send(Frame::Load { doc: doc.clone(), bytes });
                            open.insert(doc, sync::State::new());
                        }
                        Frame::Control(Control::StartPairing) => shared.start_pairing(&rt),
                        Frame::Control(Control::CancelPairing) => shared.cancel_pairing(),
                        Frame::Control(Control::Unpair { id }) => shared.unpair(&id),
                        Frame::Control(Control::SyncNow) => shared.sync_now(),
                        Frame::Control(Control::Pair { req, code }) => {
                            let (shared, out) = (shared.clone(), out.clone());
                            tokio::spawn(async move {
                                let result = shared.pair(code.trim()).await.map_err(|e| e.to_string());
                                let _ = out.send(Frame::Control(Control::Paired { req, result }).encode());
                            });
                        }
                        _ => {}
                    }
                }
                _ = changes.changed() => {}
                _ = status.changed() => send(Frame::Control(Control::Status { status: shared.status() })),
            }
        }
    }
    .await;
    if let Err(e) = result {
        tracing::debug!("app link ended: {e:#}");
    }
    reader.abort();
    drop(out);
    let _ = writer.await;
}

/// Brings Notes' sync data from its own directory (`legacy`) into the daemon's
/// `dir`: the device key and paired devices (so the phone stays paired), and
/// the notebook. Migrated files are renamed `*.migrated`. Returns whether
/// anything was moved.
pub fn migrate(dir: &Path, legacy: &Path) -> Result<bool> {
    let mut moved = Vec::new();
    let key = legacy.join("device.key");
    // Peers only know us by our key, so they come along with it or not at all.
    if key.exists() && !dir.join("device.key").exists() {
        let bytes = std::fs::read(&key)?;
        write_atomic(&dir.join("device.key"), &bytes)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("device.key"), std::fs::Permissions::from_mode(0o600))?;
        moved.push(key);
        let peers = legacy.join("peers.json");
        if peers.exists() {
            write_atomic(&dir.join("peers.json"), &std::fs::read(&peers)?)?;
            moved.push(peers);
        }
    }
    let notebook = legacy.join("notes.automerge");
    if notebook.exists() {
        let bytes = std::fs::read(&notebook)?;
        let mut doc = Doc::open(dir.join(format!("{NOTES_DOC}.automerge")));
        // Merging (rather than copying) keeps anything the daemon already has.
        doc.merge_bytes(&bytes).context("reading the old notebook")?;
        doc.save();
        moved.push(notebook);
    }
    for path in &moved {
        let mut to = path.clone().into_os_string();
        to.push(".migrated");
        std::fs::rename(path, to)?;
    }
    if !moved.is_empty() {
        tracing::info!("migrated {} files from {}", moved.len(), legacy.display());
    }
    Ok(!moved.is_empty())
}
