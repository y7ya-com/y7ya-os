//! An app's connection to the `y7-syncd` daemon.
//!
//! The app gets in-memory replicas of the documents it opens. Edits are made
//! to the replica (instantly, on the UI thread) and flow to the daemon over
//! Automerge's sync protocol; changes from other apps and devices flow back
//! the same way. A background thread blocks reading the socket, so an idle
//! app costs nothing. If the daemon goes away the thread reconnects (starting
//! the daemon again) and replicas resync, keeping any edits made meanwhile.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use automerge::sync::{self, SyncDoc};

use crate::daemon::Daemon;
use crate::doc::Doc;
use crate::ipc::{Control, Frame, VERSION, read_frame, write_frame};
use crate::{Hub, Listener, Status};

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const FLUSH_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Client {
    inner: Arc<Inner>,
}

struct Replica {
    doc: Arc<Mutex<Doc>>,
    state: Mutex<sync::State>,
    loaded: AtomicBool,
}

struct Inner {
    socket: PathBuf,
    /// Start the daemon when it isn't running (apps), or not (tests).
    autostart: bool,
    listener: Arc<dyn Listener>,
    writer: Mutex<Option<UnixStream>>,
    replicas: Mutex<HashMap<String, Arc<Replica>>>,
    status: Mutex<Status>,
    pairs: Mutex<HashMap<u64, mpsc::Sender<Result<String, String>>>>,
    next_req: AtomicU64,
    /// Bumped (and notified) after every frame from the daemon.
    progress: (Mutex<u64>, Condvar),
    /// The daemon, when this process had to host it (see [`Client::connect_default`]).
    hosted: Mutex<Option<Daemon>>,
}

impl Client {
    /// Connects to the daemon listening on `socket`.
    pub fn connect(socket: PathBuf, listener: Arc<dyn Listener>) -> Result<Client> {
        Self::start(socket, false, listener)
    }

    /// Connects to the user's daemon, starting `y7-syncd` if it isn't running.
    /// Without the binary (e.g. `cargo run -p notes`), this process hosts the
    /// daemon itself until it exits.
    pub fn connect_default(listener: Arc<dyn Listener>) -> Result<Client> {
        Self::start(crate::socket_path(), true, listener)
    }

    fn start(socket: PathBuf, autostart: bool, listener: Arc<dyn Listener>) -> Result<Client> {
        let inner = Arc::new(Inner {
            socket,
            autostart,
            listener,
            writer: Mutex::default(),
            replicas: Mutex::default(),
            status: Mutex::default(),
            pairs: Mutex::default(),
            next_req: AtomicU64::new(1),
            progress: (Mutex::new(0), Condvar::new()),
            hosted: Mutex::default(),
        });
        let stream = inner.establish()?;
        inner.attach(&stream)?;
        let reader = inner.clone();
        std::thread::Builder::new().name("y7-sync-client".into()).spawn(move || reader.run(stream))?;
        Ok(Client { inner })
    }

    /// Waits until the daemon has every local edit, or `timeout` passes.
    pub fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (lock, cvar) = &self.inner.progress;
        let mut guard = lock.lock().unwrap();
        loop {
            if self.inner.settled() {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || self.inner.writer.lock().unwrap().is_none() {
                return false;
            }
            guard = cvar.wait_timeout(guard, left).unwrap().0;
        }
    }
}

impl Inner {
    /// A connection to the daemon, starting it if allowed.
    fn establish(&self) -> Result<UnixStream> {
        if let Ok(s) = UnixStream::connect(&self.socket) {
            return Ok(s);
        }
        if !self.autostart {
            return UnixStream::connect(&self.socket).with_context(|| format!("connecting to {}", self.socket.display()));
        }
        if spawn_daemon() {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
                if let Ok(s) = UnixStream::connect(&self.socket) {
                    return Ok(s);
                }
            }
        }
        let mut hosted = self.hosted.lock().unwrap();
        if hosted.is_none() {
            match Daemon::start_default() {
                Ok(d) => {
                    tracing::info!("y7-syncd isn't installed; syncing from this app");
                    *hosted = Some(d);
                }
                // Another process may have just won the race; try it.
                Err(e) => tracing::warn!("couldn't host sync: {e:#}"),
            }
        }
        UnixStream::connect(&self.socket).with_context(|| format!("connecting to {}", self.socket.display()))
    }

    /// Starts a session on a fresh connection: (re)opens every replica.
    fn attach(&self, stream: &UnixStream) -> Result<()> {
        let mut w = stream.try_clone()?;
        write_frame(&mut w, &Frame::Control(Control::Hello { version: VERSION }))?;
        let replicas: Vec<String> = self.replicas.lock().unwrap().keys().cloned().collect();
        for id in &replicas {
            if let Some(r) = self.replica(id) {
                *r.state.lock().unwrap() = sync::State::new();
            }
            write_frame(&mut w, &Frame::Control(Control::Open { doc: id.clone() }))?;
        }
        *self.writer.lock().unwrap() = Some(w);
        Ok(())
    }

    fn run(self: Arc<Self>, mut stream: UnixStream) {
        loop {
            while let Ok(frame) = read_frame(&mut stream) {
                match Frame::decode(&frame) {
                    Ok(f) => self.handle(f),
                    Err(e) => tracing::warn!("bad frame from y7-syncd: {e}"),
                }
                let (lock, cvar) = &self.progress;
                *lock.lock().unwrap() += 1;
                cvar.notify_all();
            }
            // Disconnected: the daemon quit or crashed. Reconnect, backing off.
            *self.writer.lock().unwrap() = None;
            self.progress.1.notify_all();
            self.listener.status_changed();
            let mut delay = Duration::from_millis(200);
            stream = loop {
                std::thread::sleep(delay);
                if let Ok(s) = self.establish()
                    && self.attach(&s).is_ok()
                {
                    break s;
                }
                delay = (delay * 2).min(Duration::from_secs(30));
            };
        }
    }

    fn handle(&self, frame: Frame) {
        match frame {
            Frame::Load { doc: id, bytes } => {
                let Some(r) = self.replica(&id) else { return };
                let changed = {
                    let _state = r.state.lock().unwrap();
                    let mut doc = r.doc.lock().unwrap();
                    let before = doc.heads();
                    if let Err(e) = doc.merge_bytes(&bytes) {
                        tracing::error!("loading {id}: {e}");
                    }
                    doc.heads() != before
                };
                // The first load is what `Hub::doc` is waiting for, not news.
                if r.loaded.swap(true, Ordering::SeqCst) && changed {
                    self.listener.doc_changed(&id);
                }
                self.send_sync(&id);
            }
            Frame::Sync { doc: id, msg } => {
                let Some(r) = self.replica(&id) else { return };
                let Ok(msg) = sync::Message::decode(&msg) else { return };
                let changed = {
                    let mut state = r.state.lock().unwrap();
                    let mut doc = r.doc.lock().unwrap();
                    let before = doc.heads();
                    if let Err(e) = doc.am().sync().receive_sync_message(&mut state, msg) {
                        tracing::error!("sync {id}: {e}");
                    }
                    doc.heads() != before
                };
                if changed {
                    self.listener.doc_changed(&id);
                }
                self.send_sync(&id);
            }
            Frame::Control(Control::Status { status }) => {
                *self.status.lock().unwrap() = status;
                self.listener.status_changed();
            }
            Frame::Control(Control::Paired { req, result }) => {
                if let Some(tx) = self.pairs.lock().unwrap().remove(&req) {
                    let _ = tx.send(result);
                }
            }
            Frame::Control(_) => {}
        }
    }

    fn replica(&self, id: &str) -> Option<Arc<Replica>> {
        self.replicas.lock().unwrap().get(id).cloned()
    }

    fn send(&self, frame: &Frame) {
        let mut writer = self.writer.lock().unwrap();
        if let Some(w) = writer.as_mut()
            && write_frame(w, frame).and_then(|_| w.flush()).is_err()
        {
            // Wakes the reader thread, which reconnects.
            let _ = w.shutdown(std::net::Shutdown::Both);
            *writer = None;
        }
    }

    /// Sends whatever the daemon is missing of document `id`.
    fn send_sync(&self, id: &str) {
        let Some(r) = self.replica(id) else { return };
        if !r.loaded.load(Ordering::SeqCst) {
            return;
        }
        let msgs: Vec<Vec<u8>> = {
            let mut state = r.state.lock().unwrap();
            let mut doc = r.doc.lock().unwrap();
            std::iter::from_fn(|| doc.am().sync().generate_sync_message(&mut state).map(|m| m.encode())).collect()
        };
        for msg in msgs {
            self.send(&Frame::Sync { doc: id.to_string(), msg });
        }
    }

    /// True when the daemon has acknowledged everything in every replica.
    fn settled(&self) -> bool {
        let replicas: Vec<Arc<Replica>> = self.replicas.lock().unwrap().values().cloned().collect();
        replicas.iter().all(|r| {
            let state = r.state.lock().unwrap();
            let heads = r.doc.lock().unwrap().heads();
            heads.0.iter().all(|h| state.shared_heads.contains(h))
        })
    }

    fn control(&self, c: Control) {
        self.send(&Frame::Control(c));
    }
}

impl Hub for Client {
    fn doc(&self, id: &str) -> Arc<Mutex<Doc>> {
        let inner = &self.inner;
        let r = {
            let mut replicas = inner.replicas.lock().unwrap();
            if let Some(r) = replicas.get(id) {
                return r.doc.clone();
            }
            let r = Arc::new(Replica {
                doc: Arc::new(Mutex::new(Doc::in_memory())),
                state: Mutex::new(sync::State::new()),
                loaded: AtomicBool::new(false),
            });
            replicas.insert(id.to_string(), r.clone());
            r
        };
        inner.control(Control::Open { doc: id.to_string() });
        // Wait for the daemon's copy, so the app starts with the real content.
        let deadline = Instant::now() + OPEN_TIMEOUT;
        let (lock, cvar) = &inner.progress;
        let mut guard = lock.lock().unwrap();
        while !r.loaded.load(Ordering::SeqCst) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                tracing::warn!("y7-syncd didn't send {id}; starting empty");
                break;
            }
            guard = cvar.wait_timeout(guard, left).unwrap().0;
        }
        r.doc.clone()
    }

    fn local_changed(&self, id: &str) {
        self.inner.send_sync(id);
    }

    fn status(&self) -> Status {
        self.inner.status.lock().unwrap().clone()
    }

    fn start_pairing(&self) {
        self.inner.control(Control::StartPairing);
    }

    fn cancel_pairing(&self) {
        self.inner.control(Control::CancelPairing);
    }

    fn pair(&self, code: &str) -> Result<String> {
        let req = self.inner.next_req.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.inner.pairs.lock().unwrap().insert(req, tx);
        self.inner.control(Control::Pair { req, code: code.to_string() });
        let result = rx.recv_timeout(Duration::from_secs(60)).map_err(|_| anyhow!("The sync service didn't answer."));
        self.inner.pairs.lock().unwrap().remove(&req);
        result?.map_err(|e| anyhow!(e))
    }

    fn unpair(&self, id: &str) {
        self.inner.control(Control::Unpair { id: id.to_string() });
    }

    fn sync_now(&self) {
        self.inner.control(Control::SyncNow);
    }

    fn shutdown(&self) {
        // Pairing belongs to the sheet that started it.
        self.flush(FLUSH_TIMEOUT);
        if let Some(daemon) = self.inner.hosted.lock().unwrap().take() {
            drop(daemon);
        }
    }
}

/// Starts `y7-syncd` in the background: `$Y7_SYNCD`, next to this executable,
/// or from `PATH`. Returns false if it couldn't be started.
fn spawn_daemon() -> bool {
    use std::os::unix::process::CommandExt;
    let exe = std::env::var_os("Y7_SYNCD").map(PathBuf::from).unwrap_or_else(|| {
        let sibling = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("y7-syncd")));
        sibling.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("y7-syncd"))
    });
    let child = std::process::Command::new(&exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .process_group(0)
        .spawn();
    match child {
        Ok(mut child) => {
            // Reap it if it exits, without polling.
            std::thread::spawn(move || child.wait());
            true
        }
        Err(_) => false,
    }
}
