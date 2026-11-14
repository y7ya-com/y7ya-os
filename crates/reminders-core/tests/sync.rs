//! Two devices on this machine pair and sync reminders over real iroh
//! connections: a desktop (the daemon, with Reminders as a client of it) and
//! a phone (the engine in-process).

use std::sync::Arc;
use std::time::{Duration, Instant};

use reminders_core::{Core, DEFAULT_LIST, Due, List, Priority, Reminder};
use y7_sync::daemon::Daemon;
use y7_sync::{Client, Engine, Hub, Listener};

struct Quiet;

impl Listener for Quiet {
    fn doc_changed(&self, _doc: &str) {}
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

fn find(core: &Core, id: u64) -> Option<Reminder> {
    core.snapshot().reminders.into_iter().find(|r| r.id == id)
}

#[test]
fn pair_and_sync_reminders() {
    let root = std::env::temp_dir().join(format!("reminders-sync-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let daemon = Daemon::start(root.join("desktop"), root.join("sync.sock"), "Desktop".into(), None).unwrap();
    let client: Arc<dyn Hub> = Arc::new(Client::connect(daemon.socket().to_path_buf(), Arc::new(Quiet)).unwrap());
    let desktop = Core::new(client.clone());
    let phone = Core::new(Arc::new(Engine::start(root.join("phone"), "Phone".into(), Arc::new(Quiet))));

    // Something added before pairing still makes it across.
    let s = phone.snapshot();
    let milk = Reminder::new(DEFAULT_LIST, "Buy milk");
    phone.apply(&s.heads, &s.lists, &[milk.clone()]);

    desktop.sync().start_pairing();
    let mut code = None;
    wait_for("pairing code", || {
        code = desktop.sync().status().pairing_code;
        code.is_some()
    });
    assert_eq!(phone.sync().pair(&code.unwrap()).unwrap(), "Desktop");
    wait_for("phone -> desktop", || find(&desktop, milk.id).is_some());
    assert_eq!(desktop.snapshot().lists.len(), 1, "both default lists are the same list");

    // A list with a reminder and a subtask, made on the desktop.
    let s = desktop.snapshot();
    let mut lists = s.lists.clone();
    lists.push(List { id: 500, name: "Groceries".into(), color: "orange".into() });
    let mut trip = Reminder::new(500, "Trip");
    trip.due = Some(Due::Time(1_800_000_000));
    trip.priority = Priority::Medium;
    let mut passport = Reminder::new(500, "Passport");
    passport.parent = Some(trip.id);
    let mut reminders = s.reminders.clone();
    reminders.extend([trip.clone(), passport.clone()]);
    desktop.apply(&s.heads, &lists, &reminders);
    wait_for("desktop -> phone", || find(&phone, passport.id).is_some_and(|r| r.parent == Some(trip.id)));
    assert_eq!(find(&phone, trip.id).unwrap().due, Some(Due::Time(1_800_000_000)));

    // Concurrent edits against the same version: the phone checks "Buy milk"
    // off and flags it; the desktop renames it and adds notes. All of it stays.
    let (sd, sp) = (desktop.snapshot(), phone.snapshot());
    let mut rd = sd.reminders.clone();
    let m = rd.iter_mut().find(|r| r.id == milk.id).unwrap();
    m.title = "Buy oat milk".into();
    m.notes = "The barista one".into();
    let mut rp = sp.reminders.clone();
    let m = rp.iter_mut().find(|r| r.id == milk.id).unwrap();
    m.completed = Some(1_790_000_000);
    m.flagged = true;
    desktop.apply(&sd.heads, &sd.lists, &rd);
    phone.apply(&sp.heads, &sp.lists, &rp);
    let merged = |r: &Reminder| r.title == "Buy oat milk" && r.notes == "The barista one" && r.is_done() && r.flagged;
    wait_for("merged on desktop", || find(&desktop, milk.id).is_some_and(|r| merged(&r)));
    wait_for("merged on phone", || find(&phone, milk.id).is_some_and(|r| merged(&r)));

    // Deleting a list on one device moves nothing out from under the other:
    // its reminders show up in the first list.
    let s = desktop.snapshot();
    desktop.apply(&s.heads, &s.lists[..1], &s.reminders);
    wait_for("list deletion reaches phone", || phone.snapshot().lists.len() == 1);
    assert_eq!(find(&phone, trip.id).unwrap().list, DEFAULT_LIST);

    drop((desktop, client, phone, daemon));
    let _ = std::fs::remove_dir_all(&root);
}
