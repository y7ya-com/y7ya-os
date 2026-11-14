//! The daemon, apps talking to it, and a phone running the engine in-process,
//! all on this machine over real sockets and iroh connections.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use y7_sync::daemon::{Daemon, migrate};
use y7_sync::doc::{get_str, put_str, record, records};
use y7_sync::{Client, Doc, Engine, Hub, Listener};

#[derive(Default)]
struct Events {
    changed: Mutex<Vec<String>>,
    status: AtomicUsize,
}

impl Listener for Events {
    fn doc_changed(&self, doc: &str) {
        self.changed.lock().unwrap().push(doc.to_string());
    }
    fn status_changed(&self) {
        self.status.fetch_add(1, Ordering::SeqCst);
    }
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(40), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("{what}: {:?}", start.elapsed());
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("y7-sync-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `item:<id>.title = title` into a document.
fn put_item(hub: &dyn Hub, doc_id: &str, id: u64, title: &str) {
    let doc = hub.doc(doc_id);
    {
        let mut doc = doc.lock().unwrap();
        let base = doc.heads();
        doc.edit(&base, |am| {
            let obj = record(am, "item", id);
            put_str(am, &obj, "title", title);
        });
    }
    hub.local_changed(doc_id);
}

fn titles(doc: &Arc<Mutex<Doc>>) -> Vec<String> {
    let doc = doc.lock().unwrap();
    let am = doc.read();
    let mut v: Vec<String> = records(am, "item").iter().filter_map(|(_, o)| get_str(am, o, "title")).collect();
    v.sort();
    v
}

/// Pairs the device behind `desktop` with `phone`, the way the sheet does.
fn pair(desktop: &dyn Hub, phone: &dyn Hub) {
    desktop.start_pairing();
    let mut code = None;
    wait_for("pairing code", || {
        code = desktop.status().pairing_code;
        code.is_some()
    });
    assert_eq!(phone.pair(&code.unwrap()).expect("pairing"), "Desktop");
}

#[test]
fn apps_share_one_identity_and_sync_many_documents() {
    let root = temp("daemon");
    let daemon = Daemon::start(root.join("desktop"), root.join("sync.sock"), "Desktop".into(), None).unwrap();
    assert!(Daemon::start(root.join("desktop"), root.join("other.sock"), "Desktop".into(), None).is_err(), "one daemon per identity");

    // Two apps, each with its own connection and document.
    let calendar_events = Arc::new(Events::default());
    let calendar = Client::connect(daemon.socket().to_path_buf(), calendar_events.clone()).unwrap();
    let reminders = Client::connect(daemon.socket().to_path_buf(), Arc::new(Events::default())).unwrap();
    put_item(&calendar, "calendar", 1, "Dentist");
    put_item(&reminders, "reminders", 2, "Buy milk");

    // Pairing from one app pairs the whole computer.
    let phone = Engine::start(root.join("phone"), "Phone".into(), Arc::new(Events::default()));
    pair(&calendar, &phone);
    wait_for("other app sees the pairing", || reminders.status().peers.iter().any(|p| p.name == "Phone"));

    // Both documents reach the phone over the one connection.
    wait_for("calendar -> phone", || titles(&phone.doc("calendar")) == ["Dentist"]);
    wait_for("reminders -> phone", || titles(&phone.doc("reminders")) == ["Buy milk"]);

    // And phone edits reach the app's replica, with a notification.
    put_item(&phone, "calendar", 3, "Flight");
    wait_for("phone -> calendar app", || titles(&calendar.doc("calendar")) == ["Dentist", "Flight"]);
    assert!(calendar_events.changed.lock().unwrap().iter().any(|d| d == "calendar"));
    wait_for("connected", || calendar.status().peers.first().is_some_and(|p| p.connected));

    // The daemon saved what it got.
    assert!(root.join("desktop/calendar.automerge").exists());
    assert!(root.join("desktop/reminders.automerge").exists());
    drop((calendar, reminders, daemon, phone));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn old_phones_still_sync_notes() {
    let root = temp("legacy");
    let daemon = Daemon::start(root.join("desktop"), root.join("sync.sock"), "Desktop".into(), None).unwrap();
    let notes = Client::connect(daemon.socket().to_path_buf(), Arc::new(Events::default())).unwrap();
    let phone = Engine::start_legacy(root.join("phone"), "Old Phone".into(), Arc::new(Events::default()));
    pair(&notes, &phone);

    put_item(&notes, "notes", 1, "From the desktop");
    put_item(&notes, "calendar", 2, "Not for old phones");
    wait_for("desktop -> old phone", || titles(&phone.doc("notes")) == ["From the desktop"]);
    put_item(&phone, "notes", 3, "From the old phone");
    wait_for("old phone -> desktop", || titles(&notes.doc("notes")).len() == 2);
    assert!(titles(&phone.doc("calendar")).is_empty(), "notes/sync/1 carries only notes");
    drop((notes, daemon, phone));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn apps_reconnect_and_keep_offline_edits() {
    let root = temp("reconnect");
    let socket = root.join("sync.sock");
    let daemon = Daemon::start(root.join("desktop"), socket.clone(), "Desktop".into(), None).unwrap();
    let app = Client::connect(socket.clone(), Arc::new(Events::default())).unwrap();
    put_item(&app, "reminders", 1, "Before");
    assert!(app.flush(Duration::from_secs(5)));
    drop(daemon);

    // Edits made while the daemon is away stay in the replica...
    std::thread::sleep(Duration::from_millis(100));
    put_item(&app, "reminders", 2, "While away");
    // ...and reach it once it's back.
    let daemon = Daemon::start(root.join("desktop"), socket, "Desktop".into(), None).unwrap();
    wait_for("resynced", || app.flush(Duration::from_millis(200)));
    assert_eq!(titles(&daemon.engine().doc("reminders")), ["Before", "While away"]);
    drop((app, daemon));
    let _ = std::fs::remove_dir_all(&root);
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn migrates_notes_identity_peers_and_notebook() {
    let root = temp("migrate");
    let legacy = root.join("notes");
    let dir = root.join("y7-sync");

    // What Notes left behind: its key, a paired phone, and a notebook.
    let key = [7u8; 32];
    write(&legacy.join("device.key"), &key);
    let phone_id = iroh::SecretKey::from_bytes(&[8u8; 32]).public();
    let peers = format!(r#"[{{"id":"{phone_id}","name":"Pixel","last_sync":1700000000}}]"#);
    write(&legacy.join("peers.json"), peers.as_bytes());
    let mut notebook = Doc::open(legacy.join("notes.automerge"));
    let base = notebook.heads();
    notebook.edit(&base, |am| {
        let obj = record(am, "item", 9);
        put_str(am, &obj, "title", "Old note");
    });

    let daemon = Daemon::start(dir.clone(), root.join("sync.sock"), "Desktop".into(), Some(&legacy)).unwrap();
    assert_eq!(std::fs::read(dir.join("device.key")).unwrap(), key, "same identity, so the phone stays paired");
    let status = daemon.engine().status();
    assert_eq!(status.peers.len(), 1);
    assert_eq!(status.peers[0].name, "Pixel");
    assert_eq!(titles(&daemon.engine().doc("notes")), ["Old note"]);
    for f in ["device.key", "peers.json", "notes.automerge"] {
        assert!(!legacy.join(f).exists(), "{f} left behind");
        assert!(legacy.join(format!("{f}.migrated")).exists());
    }
    drop(daemon);

    // Once is enough: a second run changes nothing.
    assert!(!migrate(&dir, &legacy).unwrap());
    let _ = std::fs::remove_dir_all(&root);
}
