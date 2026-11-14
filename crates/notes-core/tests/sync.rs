//! Two devices on this machine pair and sync over real iroh connections.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use notes_core::{Core, DEFAULT_FOLDER, Listener, Note, now};

#[derive(Default)]
struct Counter(AtomicUsize);

impl Listener for Counter {
    fn notes_changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn status_changed(&self) {}
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(40), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("{what}: {:?}", start.elapsed());
}

#[test]
fn pair_and_sync() {
    let root = std::env::temp_dir().join(format!("notes-sync-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (la, lb) = (Arc::new(Counter::default()), Arc::new(Counter::default()));
    let desktop = Core::open(root.join("desktop"), "Desktop".into(), la.clone());
    let phone = Core::open(root.join("phone"), "Phone".into(), lb.clone());

    desktop.sync().start_pairing();
    let mut code = None;
    wait_for("pairing code", || {
        code = desktop.sync().status().pairing_code;
        code.is_some()
    });
    let code = code.unwrap();
    println!("code ({} chars): {code}", code.len());
    assert!(phone.sync().pair("notes-pair:1;bogus").is_err());
    let name = phone.sync().pair(&code).expect("pairing");
    assert_eq!(name, "Desktop");
    assert!(desktop.sync().status().pairing_code.is_none(), "code is single-use");
    assert_eq!(desktop.sync().status().peers[0].name, "Phone");

    // A note written on the phone shows up on the desktop.
    let s = phone.snapshot();
    let t = now();
    let note = Note { id: 42, folder: DEFAULT_FOLDER, body: "From the phone".into(), created: t, modified: t, pinned: false, deleted_at: None };
    phone.apply(&s.heads, &s.folders, &[note]);
    wait_for("phone -> desktop", || desktop.snapshot().notes.iter().any(|n| n.body == "From the phone"));
    assert!(la.0.load(Ordering::SeqCst) > 0);

    // And an edit on the desktop flows back.
    let s = desktop.snapshot();
    let mut notes = s.notes.clone();
    notes[0].body.push_str(" and the desktop");
    desktop.apply(&s.heads, &s.folders, &notes);
    wait_for("desktop -> phone", || phone.snapshot().notes.iter().any(|n| n.body == "From the phone and the desktop"));
    wait_for("connected", || desktop.sync().status().peers[0].connected && phone.sync().status().peers[0].connected);

    // A device that wasn't paired is turned away.
    let stranger = Core::open(root.join("stranger"), "Stranger".into(), Arc::new(Counter::default()));
    assert!(stranger.sync().pair(&code).is_err(), "used code must not pair again");
    drop((desktop, phone, stranger));
    let _ = std::fs::remove_dir_all(&root);
}
