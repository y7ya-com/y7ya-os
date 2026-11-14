//! `y7-syncd`: owns this computer's sync identity, paired devices and synced
//! documents, and serves apps over `$XDG_RUNTIME_DIR/y7-sync.sock`.
//!
//! Started by systemd (socket activation, see `y7-syncd.socket`), the XDG
//! autostart entry, or on demand by the first app that needs it. Only one
//! runs per user; extra instances exit quietly.
//!
//! `y7-syncd status` prints paired devices of the running daemon;
//! `y7-syncd pair <code>` pairs it with a device showing a pairing code.
//!
//! Environment: `Y7_SYNC_DATA_DIR` (default `~/.local/share/y7-sync`),
//! `Y7_SYNC_SOCKET`, and `NOTES_DATA_DIR` for where to migrate Notes from.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use y7_sync::{Client, Hub, Listener};

/// Signals when the daemon's status arrives.
struct StatusArrived(Mutex<mpsc::Sender<()>>);

impl Listener for StatusArrived {
    fn doc_changed(&self, _doc: &str) {}
    fn status_changed(&self) {
        let _ = self.0.lock().unwrap().send(());
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("status") => return status(),
        Some("pair") if std::env::args().nth(2).is_some() => return pair(&std::env::args().nth(2).unwrap()),
        Some(_) => {
            println!("usage: y7-syncd [status | pair <code>]\nDevice-to-device sync for y7ya-os apps.");
            return;
        }
    }
    let _daemon = match y7_sync::daemon::Daemon::start_default() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("y7-syncd: {e:#}");
            return;
        }
    };
    // Everything happens on the engine's threads; every change is saved as
    // it happens, so being killed at any point is safe.
    loop {
        std::thread::park();
    }
}

/// A connection to the running daemon, once it has sent its status.
fn connect() -> Client {
    let socket = y7_sync::socket_path();
    let (tx, rx) = mpsc::channel();
    let client = match Client::connect(socket.clone(), Arc::new(StatusArrived(Mutex::new(tx)))) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("y7-syncd isn't running ({})", socket.display());
            std::process::exit(1);
        }
    };
    // The daemon sends its status as soon as an app connects.
    let _ = rx.recv_timeout(Duration::from_secs(2));
    client
}

fn pair(code: &str) {
    match connect().pair(code) {
        Ok(name) => println!("Paired with {name}."),
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }
}

fn status() {
    let status = connect().status();
    if status.peers.is_empty() {
        println!("No paired devices.");
    }
    for p in status.peers {
        let state = if p.connected { "connected".to_string() } else { p.last_sync.map_or("never synced".into(), |t| format!("last synced at {t}")) };
        println!("{}  {}  ({state})", p.name, p.id);
    }
}
