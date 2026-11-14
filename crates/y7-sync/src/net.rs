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
//! protocol over it for every document, so edits show up on the other device
//! within a moment. Protocols, newest first:
//!
//! - `y7/sync/2`: one stream; each frame is `[id length: u8][doc id][sync message]`.
//!   Either side may mention a document the other doesn't have yet; it's
//!   created on arrival, so every device ends up holding (and relaying) all
//!   documents.
//! - `notes/sync/1`: one stream of bare sync messages for the `notes`
//!   document. Spoken by the first Notes releases; still accepted, and used
//!   when dialing a device that doesn't offer version 2.
//!
//! The network only starts once there is a paired device or pairing begins,
//! so a device that syncs with nothing stays completely idle.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use automerge::sync::{self, SyncDoc};
use iroh::endpoint::{ConnectOptions, Connection, RecvStream, SendStream, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, TransportAddr};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc, watch};

use crate::doc::{Doc, write_atomic};
use crate::{Hub, Listener, NOTES_DOC, PeerStatus, Status, now, valid_doc_id};

const SYNC_ALPN: &[u8] = b"y7/sync/2";
const SYNC_ALPN_V1: &[u8] = b"notes/sync/1";
const PAIR_ALPN: &[u8] = b"notes/pair/1";
/// Phones check for this prefix, so it stays even though more than notes sync.
const PAIR_PREFIX: &str = "notes-pair:1";
const PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
/// Unreachable paired devices are retried after this, backing off to
/// `REDIAL_MAX` while nothing changes. Edits and incoming connections retry
/// right away.
const REDIAL_MIN: Duration = Duration::from_secs(30);
const REDIAL_MAX: Duration = Duration::from_secs(15 * 60);
const MAX_FRAME: usize = 64 << 20;

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

pub(crate) struct Shared {
    dir: PathBuf,
    device_name: String,
    listener: Arc<dyn Listener>,
    router: watch::Sender<Option<Router>>,
    /// Set once something needs the network (a peer, or pairing).
    online: AtomicBool,
    peers: Mutex<Vec<Peer>>,
    live: Mutex<HashMap<EndpointId, Vec<Connection>>>,
    dialing: Mutex<HashSet<EndpointId>>,
    pairing: Mutex<Option<Pairing>>,
    docs: Mutex<BTreeMap<String, Arc<Mutex<Doc>>>>,
    /// Bumped whenever any document changes, so open sessions send it on.
    pub(crate) changes: watch::Sender<u64>,
    /// Makes the connector try paired devices right away.
    wake: Notify,
    /// Behave like the first Notes release (`notes/sync/1` only), for tests.
    v1_only: bool,
}

/// The sync engine: identity, pairing, peers and documents, all in `dir`
/// (`device.key`, `peers.json`, `<doc>.automerge`).
pub struct Engine {
    pub(crate) shared: Arc<Shared>,
    rt: tokio::runtime::Runtime,
}

impl Engine {
    pub fn start(dir: PathBuf, device_name: String, listener: Arc<dyn Listener>) -> Self {
        Self::start_with(dir, device_name, listener, false)
    }

    /// An engine that only speaks `notes/sync/1`, like phones running the
    /// first Notes release. For compatibility tests.
    #[doc(hidden)]
    pub fn start_legacy(dir: PathBuf, device_name: String, listener: Arc<dyn Listener>) -> Self {
        Self::start_with(dir, device_name, listener, true)
    }

    fn start_with(dir: PathBuf, device_name: String, listener: Arc<dyn Listener>, v1_only: bool) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("y7-sync")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let peers: Vec<Peer> = std::fs::read(dir.join("peers.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Every saved document syncs with peers, even ones no app opened.
        let mut docs = BTreeMap::new();
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            if path.extension().is_some_and(|e| e == "automerge") && valid_doc_id(&id) {
                docs.insert(id, Arc::new(Mutex::new(Doc::open(path))));
            }
        }
        let shared = Arc::new(Shared {
            online: AtomicBool::new(!peers.is_empty()),
            dir,
            device_name,
            listener,
            router: watch::channel(None).0,
            peers: Mutex::new(peers),
            live: Mutex::default(),
            dialing: Mutex::default(),
            pairing: Mutex::default(),
            docs: Mutex::new(docs),
            changes: watch::channel(0).0,
            wake: Notify::new(),
            v1_only,
        });
        rt.spawn(run(shared.clone()));
        Engine { shared, rt }
    }

    pub(crate) fn runtime(&self) -> &tokio::runtime::Handle {
        self.rt.handle()
    }
}

impl Hub for Engine {
    fn doc(&self, id: &str) -> Arc<Mutex<Doc>> {
        self.shared.doc(id)
    }

    fn local_changed(&self, _id: &str) {
        self.shared.local_changed();
    }

    fn status(&self) -> Status {
        self.shared.status()
    }

    fn start_pairing(&self) {
        self.shared.start_pairing(self.rt.handle());
    }

    fn cancel_pairing(&self) {
        self.shared.cancel_pairing();
    }

    fn pair(&self, code: &str) -> Result<String> {
        self.rt.block_on(self.shared.clone().pair(code.trim()))
    }

    fn unpair(&self, id: &str) {
        self.shared.unpair(id);
    }

    fn sync_now(&self) {
        self.shared.sync_now();
    }

    fn shutdown(&self) {
        let router = self.shared.router.borrow().clone();
        if let Some(router) = router {
            let _ = self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(1), router.shutdown()).await });
        }
    }
}

impl Shared {
    pub(crate) fn doc(&self, id: &str) -> Arc<Mutex<Doc>> {
        let mut docs = self.docs.lock().unwrap();
        docs.entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(Doc::open(self.dir.join(format!("{id}.automerge"))))))
            .clone()
    }

    fn doc_ids(&self) -> Vec<String> {
        self.docs.lock().unwrap().keys().cloned().collect()
    }

    pub(crate) fn local_changed(&self) {
        self.changes.send_modify(|v| *v += 1);
        self.wake.notify_one();
    }

    /// Applies a sync message to document `id`. Returns whether it changed.
    pub(crate) fn receive(&self, id: &str, state: &mut sync::State, msg: sync::Message) -> Result<bool> {
        let doc = self.doc(id);
        let changed = {
            let mut doc = doc.lock().unwrap();
            let before = doc.heads();
            doc.am().sync().receive_sync_message(state, msg)?;
            let changed = doc.heads() != before;
            if changed {
                doc.save();
            }
            changed
        };
        if changed {
            self.changes.send_modify(|v| *v += 1);
            self.listener.doc_changed(id);
        }
        Ok(changed)
    }

    pub(crate) fn start_pairing(self: &Arc<Self>, rt: &tokio::runtime::Handle) {
        let token = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>());
        *self.pairing.lock().unwrap() =
            Some(Pairing { token: token.clone(), expires: Instant::now() + PAIRING_TTL, code: None });
        let shared = self.clone();
        rt.spawn(async move {
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

    pub(crate) fn cancel_pairing(&self) {
        *self.pairing.lock().unwrap() = None;
        self.listener.status_changed();
    }

    pub(crate) fn sync_now(&self) {
        self.wake.notify_one();
    }

    pub(crate) fn status(&self) -> Status {
        let live = self.live.lock().unwrap();
        let peers = self.peers.lock().unwrap();
        let pairing = self.pairing.lock().unwrap();
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

    pub(crate) fn unpair(&self, id: &str) {
        let Ok(id) = id.parse::<EndpointId>() else { return };
        self.peers.lock().unwrap().retain(|p| p.id != id);
        self.save_peers();
        for conn in self.live.lock().unwrap().remove(&id).unwrap_or_default() {
            conn.close(0u32.into(), b"unpaired");
        }
        self.listener.status_changed();
    }

    /// The network endpoint, starting it if this is the first use.
    async fn endpoint(&self) -> Endpoint {
        if !self.online.swap(true, Ordering::SeqCst) {
            self.wake.notify_one();
        }
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

    pub(crate) async fn pair(self: Arc<Self>, code: &str) -> Result<String> {
        let parts: Vec<&str> = code.split(';').collect();
        let [PAIR_PREFIX, id, token, relay, ips] = parts[..] else {
            bail!("That isn't a pairing code.");
        };
        let id: EndpointId = id.parse().context("That isn't a pairing code.")?;
        let mut addrs: Vec<TransportAddr> =
            ips.split(',').filter_map(|a| a.parse().ok()).map(TransportAddr::Ip).collect();
        if let Ok(url) = relay.parse() {
            addrs.push(TransportAddr::Relay(url));
        }
        let ep = self.endpoint().await;
        let unreachable = "Couldn't reach the other device. Is it on and showing this code?";
        let conn = tokio::time::timeout(Duration::from_secs(30), ep.connect(EndpointAddr::from_parts(id, addrs), PAIR_ALPN))
            .await
            .context(unreachable)?
            .context(unreachable)?;
        let (mut send, mut recv) = conn.open_bi().await?;
        write_frame(&mut send, token.as_bytes()).await?;
        write_frame(&mut send, self.device_name.as_bytes()).await?;
        let reply = read_frame(&mut recv).await?;
        if reply != b"ok" {
            bail!("This code has expired. Show a new one on the other device.");
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
        let v2 = conn.alpn() == SYNC_ALPN;
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
        let mut states: HashMap<String, sync::State> = HashMap::new();
        let result: Result<()> = async {
            loop {
                let ids = if v2 { self.doc_ids() } else { vec![NOTES_DOC.to_string()] };
                for id in ids {
                    let doc = self.doc(&id);
                    let state = states.entry(id.clone()).or_insert_with(sync::State::new);
                    loop {
                        let msg = doc.lock().unwrap().am().sync().generate_sync_message(state);
                        let Some(msg) = msg else { break };
                        let frame = if v2 { tagged(&id, &msg.encode()) } else { msg.encode() };
                        write_frame(&mut send, &frame).await?;
                    }
                }
                tokio::select! {
                    frame = rx.recv() => {
                        let Some(frame) = frame else { return Ok(()) };
                        let (id, msg) = if v2 { untag(&frame)? } else { (NOTES_DOC, frame.as_slice()) };
                        let msg = sync::Message::decode(msg)?;
                        let state = states.entry(id.to_string()).or_insert_with(sync::State::new);
                        let changed = self.receive(id, state, msg)?;
                        self.mark_synced(&peer);
                        if changed {
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
        // Offer both versions; the other side picks the newest it speaks.
        let (alpn, more) = if self.v1_only { (SYNC_ALPN_V1, vec![]) } else { (SYNC_ALPN, vec![SYNC_ALPN_V1.to_vec()]) };
        let opts = ConnectOptions::new().with_additional_alpns(more);
        let conn = tokio::time::timeout(Duration::from_secs(20), async {
            ep.connect_with_opts(id, alpn, opts).await?.await.map_err(anyhow::Error::from)
        })
        .await;
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

/// `[id length][id][payload]`, the version 2 frame.
pub(crate) fn tagged(id: &str, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + id.len() + payload.len());
    frame.push(id.len() as u8);
    frame.extend_from_slice(id.as_bytes());
    frame.extend_from_slice(payload);
    frame
}

pub(crate) fn untag(frame: &[u8]) -> Result<(&str, &[u8])> {
    let n = *frame.first().context("empty frame")? as usize;
    let id = frame.get(1..1 + n).and_then(|b| std::str::from_utf8(b).ok()).filter(|id| valid_doc_id(id));
    let Some(id) = id else { bail!("bad document id") };
    Ok((id, &frame[1 + n..]))
}

async fn run(shared: Arc<Shared>) {
    // Stay off the network until something needs it.
    while !shared.online.load(Ordering::SeqCst) {
        shared.wake.notified().await;
    }
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
    let mut router = Router::builder(ep.clone());
    if !shared.v1_only {
        router = router.accept(SYNC_ALPN, SyncHandler(shared.clone()));
    }
    let router = router
        .accept(SYNC_ALPN_V1, SyncHandler(shared.clone()))
        .accept(PAIR_ALPN, PairHandler(shared.clone()))
        .spawn();
    shared.router.send_replace(Some(router));

    // Keep trying paired devices that aren't connected.
    let mut redial = REDIAL_MIN;
    loop {
        let peers: Vec<EndpointId> = shared.peers.lock().unwrap().iter().map(|p| p.id).collect();
        for id in peers {
            let connected = shared.live.lock().unwrap().get(&id).is_some_and(|c| !c.is_empty());
            if !connected && shared.dialing.lock().unwrap().insert(id) {
                tokio::spawn(shared.clone().dial(ep.clone(), id));
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(redial) => redial = (redial * 2).min(REDIAL_MAX),
            _ = shared.wake.notified() => redial = REDIAL_MIN,
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
