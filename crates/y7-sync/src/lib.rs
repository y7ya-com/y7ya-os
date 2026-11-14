//! Device-to-device sync for every y7ya-os app.
//!
//! A device has one identity and one list of paired devices, and syncs any
//! number of Automerge documents ("notes", "calendar", "reminders", ...) with
//! them over a single connection per peer.
//!
//! - [`Engine`] does the syncing in-process. The Android app embeds it.
//! - On the desktop, the `y7-syncd` daemon runs the engine ([`daemon`]) and
//!   apps talk to it over a Unix socket with a [`Client`]. Each app keeps an
//!   in-memory replica of the documents it opened and syncs it with the
//!   daemon using Automerge's sync protocol, so the daemon needs no knowledge
//!   of any app's schema, and pairing happens once for the whole system.
//!
//! Both implement [`Hub`], which is all an app needs.

pub mod client;
pub mod daemon;
pub mod doc;
mod ipc;
mod net;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

pub use automerge;
pub use client::Client;
pub use doc::{Doc, Heads};
pub use net::Engine;

/// The Notes document: the only one the first phone app knew about.
pub const NOTES_DOC: &str = "notes";

/// Receives sync events. Called from background threads.
pub trait Listener: Send + std::marker::Sync + 'static {
    /// Document `doc` changed because of another device or app.
    fn doc_changed(&self, doc: &str);
    /// Paired devices, their connection state, or pairing changed.
    fn status_changed(&self);
}

/// What apps use to reach documents and pairing, whether the engine runs
/// in-process ([`Engine`]) or in the daemon ([`Client`]).
pub trait Hub: Send + std::marker::Sync {
    /// The document `id`, created empty if it doesn't exist yet.
    fn doc(&self, id: &str) -> Arc<Mutex<Doc>>;
    /// Call after every local edit of `id`; paired devices get it within a moment.
    fn local_changed(&self, id: &str);
    fn status(&self) -> Status;
    /// Starts accepting one new device. The code to show appears in
    /// `status().pairing_code` once the network is up (normally instantly).
    fn start_pairing(&self);
    fn cancel_pairing(&self);
    /// Pairs with the device showing `code`. Blocks until done; returns the
    /// other device's name.
    fn pair(&self, code: &str) -> anyhow::Result<String>;
    fn unpair(&self, id: &str);
    /// Try to reach paired devices now (e.g. when an app comes to the front).
    fn sync_now(&self);
    /// Makes sure edits have left this process (quitting apps), and closes
    /// connections so peers notice right away.
    fn shutdown(&self);
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeerStatus {
    pub id: String,
    pub name: String,
    pub connected: bool,
    /// When documents were last exchanged with this device (Unix seconds).
    pub last_sync: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub peers: Vec<PeerStatus>,
    /// Shown as a QR code while this device is waiting to be paired.
    pub pairing_code: Option<String>,
}

/// A document id: short, lowercase, safe as a file name.
pub fn valid_doc_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A fresh record id that won't collide with ones made on other devices. Ids
/// stay below 2^53 so they survive any language's number type, and are never 0.
pub fn new_id() -> u64 {
    loop {
        let id = rand::random::<u64>() >> 11;
        if id != 0 {
            return id;
        }
    }
}

/// Seconds since the Unix epoch.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// This computer's name as shown on paired devices.
pub fn device_name() -> String {
    let pretty = std::fs::read_to_string("/etc/machine-info").ok().and_then(|s| {
        s.lines().find_map(|l| l.strip_prefix("PRETTY_HOSTNAME=").map(|v| v.trim_matches('"').to_string()))
    });
    pretty
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Computer".into())
}

/// Where the daemon keeps the identity, peers and documents:
/// `~/.local/share/y7-sync`, or `$Y7_SYNC_DATA_DIR`.
pub fn data_dir() -> PathBuf {
    std::env::var_os("Y7_SYNC_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| local_share().join("y7-sync"))
}

/// The daemon's socket: `$XDG_RUNTIME_DIR/y7-sync.sock`, or `$Y7_SYNC_SOCKET`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("Y7_SYNC_SOCKET") {
        return PathBuf::from(p);
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("y7-sync.sock"),
        None => std::env::temp_dir().join(format!("y7-sync-{}.sock", std::env::var("USER").unwrap_or_default())),
    }
}

/// Where Notes kept its identity before sync moved into the daemon:
/// `~/.local/share/notes`, or `$NOTES_DATA_DIR`.
pub fn legacy_notes_dir() -> PathBuf {
    std::env::var_os("NOTES_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| local_share().join("notes"))
}

fn local_share() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
}
