//! Device-to-device sync over iroh: no server, no account.
//!
//! Every device has an Ed25519 key; its public half is the device's address
//! (`EndpointId`). Connections are QUIC + TLS 1.3 authenticated by those keys,
//! so a device only ever talks to the exact peers it paired with. iroh finds
//! peers on the local network (mDNS) or over the internet (hole punching, with
//! an encrypted relay as a fallback); either way only the two devices can read
//! the traffic.
//!
//! Pairing: the computer shows a QR code holding its address and a one-time
//! secret. The phone connects to that address, proves it saw the code by
//! sending the secret, and both sides remember each other's key.
//!
//! Sync: paired devices keep a connection open and run Automerge's sync
//! protocol over it, so edits show up on the other device within a moment.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use automerge::sync::{self, SyncDoc};
use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, TransportAddr};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc, watch};

use crate::model::now;
use crate::notebook::{Notebook, write_atomic};

const SYNC_ALPN: &[u8] = b"notes/sync/1";
const PAIR_ALPN: &[u8] = b"notes/pair/1";
const PAIR_PREFIX: &str = "notes-pair:1";
const PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
const REDIAL_EVERY: Duration = Duration::from_secs(30);
const MAX_FRAME: usize = 64 << 20;

/// Receives sync events. Called from background threads.
pub trait Listener: Send + std::marker::Sync + 'static {
    /// The notes changed because of another device.
    fn notes_changed(&self);
    /// Paired devices, their connection state, or pairing changed.
    fn status_changed(&self);
}

#[derive(Clone, Debug)]
pub struct PeerStatus {
    pub id: String,
    pub name: String,
    pub connected: bool,
    /// When notes were last exchanged with this device (Unix seconds).
    pub last_sync: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub peers: Vec<PeerStatus>,
    /// Shown as a QR code while this device is waiting to be paired.
    pub pairing_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Peer {
    id: EndpointId,
    name: String,
    #[serde(default)]
    last_sync: Option<i64>,
}

struct Pairing {
    token: String,
    expires: Instant,
    code: Option<String>,
}

struct Shared {
    notebook: Arc<Mutex<Notebook>>,
    dir: PathBuf,
    device_name: String,
    listener: Arc<dyn Listener>,
    router: watch::Sender<Option<Router>>,
    peers: Mutex<Vec<Peer>>,
    live: Mutex<HashMap<EndpointId, Vec<Connection>>>,
    dialing: Mutex<HashSet<EndpointId>>,
    pairing: Mutex<Option<Pairing>>,
    /// Bumped whenever the notebook changes, so open sessions send it on.
    changes: watch::Sender<u64>,
    /// Makes the connector try paired devices right away.
    wake: Notify,
}

pub struct Sync {
    shared: Arc<Shared>,
    rt: tokio::runtime::Runtime,
}

impl Sync {
    pub fn start(notebook: Arc<Mutex<Notebook>>, dir: PathBuf, device_name: String, listener: Arc<dyn Listener>) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("notes-sync")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let peers = std::fs::read(dir.join("peers.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let shared = Arc::new(Shared {
            notebook,
            dir,
            device_name,
            listener,
            router: watch::channel(None).0,
            peers: Mutex::new(peers),
            live: Mutex::default(),
            dialing: Mutex::default(),
            pairing: Mutex::default(),
            changes: watch::channel(0).0,
            wake: Notify::new(),
        });
        rt.spawn(run(shared.clone()));
        Sync { shared, rt }
    }

    /// Call after every local edit; paired devices get it within a moment.
    pub fn local_changed(&self) {
        self.shared.changes.send_modify(|v| *v += 1);
        self.shared.wake.notify_one();
    }

    /// Try to reach paired devices now (e.g. when the app comes to the front).
    pub fn sync_now(&self) {
        self.shared.wake.notify_one();
    }

    pub fn status(&self) -> Status {
        let live = self.shared.live.lock().unwrap();
        let peers = self.shared.peers.lock().unwrap();
        let pairing = self.shared.pairing.lock().unwrap();
        Status {
            peers: peers
                .iter()
                .map(|p| PeerStatus {
                    id: p.id.to_string(),
                    name: p.name.clone(),
                    connected: live.get(&p.id).is_some_and(|c| !c.is_empty()),
                    last_sync: p.last_sync,
                })
                .collect(),
            pairing_code: pairing.as_ref().filter(|p| p.expires > Instant::now()).and_then(|p| p.code.clone()),
        }
    }

    /// Starts accepting one new device. The code to show appears in
    /// `status().pairing_code` once the network is up (normally instantly).
    pub fn start_pairing(&self) {
        let token = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>());
        *self.shared.pairing.lock().unwrap() =
            Some(Pairing { token: token.clone(), expires: Instant::now() + PAIRING_TTL, code: None });
        let shared = self.shared.clone();
        self.rt.spawn(async move {
            let ep = shared.endpoint().await;
            // Wait briefly for a relay, so the code also works off this network.
            let _ = tokio::time::timeout(Duration::from_secs(4), ep.online()).await;
            let addr = ep.addr();
            let relay = addr.relay_urls().next().map_or("-".to_string(), |u| u.to_string());
            let ips: Vec<String> = addr.ip_addrs().map(|a| a.to_string()).collect();
            let code = format!("{PAIR_PREFIX};{};{token};{relay};{}", ep.id(), ips.join(","));
            if let Some(p) = shared.pairing.lock().unwrap().as_mut().filter(|p| p.token == token) {
                p.code = Some(code);
            }
            shared.listener.status_changed();
        });
    }

    pub fn cancel_pairing(&self) {
        *self.shared.pairing.lock().unwrap() = None;
        self.shared.listener.status_changed();
    }

    /// Pairs with the device showing `code`. Blocks until done; returns the
    /// other device's name.
    pub fn pair(&self, code: &str) -> Result<String> {
        self.rt.block_on(self.shared.clone().pair(code.trim()))
    }

    pub fn unpair(&self, id: &str) {
        let Ok(id) = id.parse::<EndpointId>() else { return };
        self.shared.peers.lock().unwrap().retain(|p| p.id != id);
        self.shared.save_peers();
        for conn in self.shared.live.lock().unwrap().remove(&id).unwrap_or_default() {
            conn.close(0u32.into(), b"unpaired");
        }
        self.shared.listener.status_changed();
    }

    /// Closes connections so peers notice right away.
    pub fn shutdown(&self) {
        let router = self.shared.router.borrow().clone();
        if let Some(router) = router {
            let _ = self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(1), router.shutdown()).await });
        }
    }
}

impl Shared {
    async fn endpoint(&self) -> Endpoint {
        let mut rx = self.router.subscribe();
        let router = rx.wait_for(|r| r.is_some()).await.expect("sync stopped");
        router.as_ref().unwrap().endpoint().clone()
    }

    fn is_paired(&self, id: &EndpointId) -> bool {
        self.peers.lock().unwrap().iter().any(|p| p.id == *id)
    }

    fn add_peer(&self, id: EndpointId, name: String) {
        let mut peers = self.peers.lock().unwrap();
        peers.retain(|p| p.id != id);
        peers.push(Peer { id, name, last_sync: None });
        drop(peers);
        self.save_peers();
    }

    fn save_peers(&self) {
        let peers = self.peers.lock().unwrap().clone();
        if let Err(e) = write_atomic(&self.dir.join("peers.json"), &serde_json::to_vec_pretty(&peers).unwrap()) {
            tracing::error!("saving peers failed: {e}");
        }
    }

    fn mark_synced(&self, id: &EndpointId) {
        if let Some(p) = self.peers.lock().unwrap().iter_mut().find(|p| p.id == *id) {
            p.last_sync = Some(now());
        }
    }

    async fn pair(self: Arc<Self>, code: &str) -> Result<String> {
        let parts: Vec<&str> = code.split(';').collect();
        let [PAIR_PREFIX, id, token, relay, ips] = parts[..] else {
            bail!("That isn't a Notes pairing code.");
        };
        let id: EndpointId = id.parse().context("That isn't a Notes pairing code.")?;
        let mut addrs: Vec<TransportAddr> =
            ips.split(',').filter_map(|a| a.parse().ok()).map(TransportAddr::Ip).collect();
        if let Ok(url) = relay.parse() {
            addrs.push(TransportAddr::Relay(url));
        }
        let ep = self.endpoint().await;
        let conn = tokio::time::timeout(Duration::from_secs(30), ep.connect(EndpointAddr::from_parts(id, addrs), PAIR_ALPN))
            .await
            .context("Couldn't reach your computer. Is Notes open on it?")?
            .context("Couldn't reach your computer. Is Notes open on it?")?;
        let (mut send, mut recv) = conn.open_bi().await?;
        write_frame(&mut send, token.as_bytes()).await?;
        write_frame(&mut send, self.device_name.as_bytes()).await?;
        let reply = read_frame(&mut recv).await?;
        if reply != b"ok" {
            bail!("This code has expired. Show a new one on your computer.");
        }
        let name = String::from_utf8_lossy(&read_frame(&mut recv).await?).into_owned();
        conn.close(0u32.into(), b"paired");
        self.add_peer(id, name.clone());
        self.listener.status_changed();
        self.wake.notify_one();
        Ok(name)
    }

    /// Exchanges changes with a paired device until the connection drops.
    async fn session(self: Arc<Self>, conn: Connection, initiator: bool) -> Result<()> {
        let peer = conn.remote_id();
        let (mut send, recv) = if initiator { conn.open_bi().await? } else { conn.accept_bi().await? };
        self.live.lock().unwrap().entry(peer).or_default().push(conn.clone());
        self.listener.status_changed();

        // Reading a frame isn't cancel-safe, so it gets its own task.
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(16);
        let reader = tokio::spawn(async move {
            let mut recv = recv;
            while let Ok(frame) = read_frame(&mut recv).await {
                if tx.send(frame).await.is_err() {
                    break;
                }
            }
        });

        let mut changes = self.changes.subscribe();
        let mut state = sync::State::new();
        let result: Result<()> = async {
            loop {
                loop {
                    let msg = self.notebook.lock().unwrap().doc().sync().generate_sync_message(&mut state);
                    let Some(msg) = msg else { break };
                    write_frame(&mut send, &msg.encode()).await?;
                }
                tokio::select! {
                    frame = rx.recv() => {
                        let Some(frame) = frame else { return Ok(()) };
                        let msg = sync::Message::decode(&frame)?;
                        let changed = {
                            let mut nb = self.notebook.lock().unwrap();
                            let before = nb.heads();
                            nb.doc().sync().receive_sync_message(&mut state, msg)?;
                            let changed = nb.heads() != before;
                            if changed {
                                nb.save();
                            }
                            changed
                        };
                        self.mark_synced(&peer);
                        if changed {
                            self.changes.send_modify(|v| *v += 1);
                            self.listener.notes_changed();
                            self.listener.status_changed();
                        }
                    }
                    _ = changes.changed() => {}
                }
            }
        }
        .await;

        reader.abort();
        if let Some(conns) = self.live.lock().unwrap().get_mut(&peer) {
            conns.retain(|c| c.stable_id() != conn.stable_id());
        }
        self.save_peers();
        self.listener.status_changed();
        result
    }

    async fn dial(self: Arc<Self>, ep: Endpoint, id: EndpointId) {
        let conn = tokio::time::timeout(Duration::from_secs(20), ep.connect(id, SYNC_ALPN)).await;
        self.dialing.lock().unwrap().remove(&id);
        match conn {
            Ok(Ok(conn)) => {
                if let Err(e) = self.session(conn, true).await {
                    tracing::debug!("sync with {id} ended: {e:#}");
                }
            }
            Ok(Err(e)) => tracing::debug!("couldn't reach {id}: {e}"),
            Err(_) => tracing::debug!("couldn't reach {id}: timed out"),
        }
    }
}

async fn run(shared: Arc<Shared>) {
    let key = load_or_create_key(&shared.dir);
    let ep = loop {
        let built = Endpoint::builder(presets::N0)
            .secret_key(key.clone())
            .address_lookup(MdnsAddressLookup::builder())
            .bind()
            .await;
        match built {
            Ok(ep) => break ep,
            Err(e) => {
                tracing::warn!("network unavailable ({e}); retrying");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    };
    let router = Router::builder(ep.clone())
        .accept(SYNC_ALPN, SyncHandler(shared.clone()))
        .accept(PAIR_ALPN, PairHandler(shared.clone()))
        .spawn();
    shared.router.send_replace(Some(router));

    // Keep trying paired devices that aren't connected.
    loop {
        let peers: Vec<EndpointId> = shared.peers.lock().unwrap().iter().map(|p| p.id).collect();
        for id in peers {
            let connected = shared.live.lock().unwrap().get(&id).is_some_and(|c| !c.is_empty());
            if !connected && shared.dialing.lock().unwrap().insert(id) {
                tokio::spawn(shared.clone().dial(ep.clone(), id));
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(REDIAL_EVERY) => {}
            _ = shared.wake.notified() => {}
        }
    }
}

#[derive(Clone)]
struct SyncHandler(Arc<Shared>);

impl std::fmt::Debug for SyncHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncHandler")
    }
}

impl ProtocolHandler for SyncHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if !self.0.is_paired(&conn.remote_id()) {
            conn.close(1u32.into(), b"not paired");
            return Ok(());
        }
        if let Err(e) = self.0.clone().session(conn, false).await {
            tracing::debug!("sync session ended: {e:#}");
        }
        Ok(())
    }
}

#[derive(Clone)]
struct PairHandler(Arc<Shared>);

impl std::fmt::Debug for PairHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairHandler")
    }
}

impl ProtocolHandler for PairHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(e) = self.pair(&conn).await {
            tracing::debug!("pairing failed: {e:#}");
        }
        Ok(())
    }
}

impl PairHandler {
    async fn pair(&self, conn: &Connection) -> Result<()> {
        let shared = &self.0;
        let (mut send, mut recv) = conn.accept_bi().await?;
        let (token, name) = tokio::time::timeout(Duration::from_secs(15), async {
            Ok::<_, anyhow::Error>((read_frame(&mut recv).await?, read_frame(&mut recv).await?))
        })
        .await??;
        let accepted = {
            let mut pairing = shared.pairing.lock().unwrap();
            let ok = pairing
                .as_ref()
                .is_some_and(|p| p.expires > Instant::now() && constant_time_eq(p.token.as_bytes(), &token));
            if ok {
                *pairing = None; // One code, one device.
            }
            ok
        };
        if !accepted {
            write_frame(&mut send, b"no").await?;
            send.finish()?;
            let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
            return Ok(());
        }
        let name = String::from_utf8_lossy(&name).chars().take(64).collect::<String>();
        shared.add_peer(conn.remote_id(), if name.trim().is_empty() { "Phone".into() } else { name });
        write_frame(&mut send, b"ok").await?;
        write_frame(&mut send, shared.device_name.as_bytes()).await?;
        send.finish()?;
        let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
        shared.listener.status_changed();
        shared.wake.notify_one();
        Ok(())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn load_or_create_key(dir: &std::path::Path) -> SecretKey {
    let path = dir.join("device.key");
    if let Ok(bytes) = std::fs::read(&path)
        && let Ok(bytes) = <[u8; 32]>::try_from(bytes.as_slice())
    {
        return SecretKey::from_bytes(&bytes);
    }
    let key = SecretKey::generate();
    let _ = std::fs::create_dir_all(dir);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path);
        if let Ok(mut f) = file {
            let _ = f.write_all(&key.to_bytes());
        }
    }
    key
}

async fn write_frame(send: &mut SendStream, data: &[u8]) -> Result<()> {
    send.write_all(&(data.len() as u32).to_be_bytes()).await?;
    send.write_all(data).await?;
    Ok(())
}

async fn read_frame(recv: &mut RecvStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        bail!("frame too large");
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;
    Ok(buf)
}
