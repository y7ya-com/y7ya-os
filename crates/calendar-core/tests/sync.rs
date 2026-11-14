//! Two devices on this machine pair and sync calendars over real iroh
//! connections: a desktop (the daemon, with Calendar as a client of it) and a
//! phone (the engine in-process).

use std::sync::Arc;
use std::time::{Duration, Instant};

use calendar_core::{Calendar, Core, Event};
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

fn event(id: u64, title: &str, start: i64) -> Event {
    Event {
        id,
        calendar: 1,
        title: title.into(),
        start,
        end: start + 3600,
        all_day: false,
        location: String::new(),
        notes: String::new(),
        rrule: String::new(),
        created: start,
        modified: start,
    }
}

fn find(core: &Core, id: u64) -> Option<Event> {
    core.snapshot().events.into_iter().find(|e| e.id == id)
}

#[test]
fn pair_and_sync_calendars() {
    let root = std::env::temp_dir().join(format!("calendar-sync-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let daemon = Daemon::start(root.join("desktop"), root.join("sync.sock"), "Desktop".into(), None).unwrap();
    let client: Arc<dyn Hub> = Arc::new(Client::connect(daemon.socket().to_path_buf(), Arc::new(Quiet)).unwrap());
    let desktop = Core::new(client.clone());
    let phone = Core::new(Arc::new(Engine::start(root.join("phone"), "Phone".into(), Arc::new(Quiet))));

    desktop.sync().start_pairing();
    let mut code = None;
    wait_for("pairing code", || {
        code = desktop.sync().status().pairing_code;
        code.is_some()
    });
    assert_eq!(phone.sync().pair(&code.unwrap()).unwrap(), "Desktop");

    // Both started with the default calendars; they merge into one set.
    wait_for("calendars merge", || desktop.snapshot().calendars == phone.snapshot().calendars);
    assert_eq!(desktop.snapshot().calendars.len(), 2);

    // An event and a new calendar made on the desktop show up on the phone.
    let s = desktop.snapshot();
    let mut calendars = s.calendars.clone();
    calendars.push(Calendar { id: 77, name: "Family".into(), color: "green".into() });
    let mut dentist = event(10, "Dentist", 1_800_000_000);
    dentist.calendar = 77;
    desktop.apply(&s.heads, &calendars, &[dentist]);
    wait_for("desktop -> phone", || find(&phone, 10).is_some_and(|e| e.calendar == 77));
    assert!(phone.snapshot().calendars.iter().any(|c| c.name == "Family"));

    // Concurrent edits, both made against the same version: the desktop moves
    // the event and edits its title, the phone adds a location and edits the
    // start of the title. Everything survives; titles merge by character.
    let (sd, sp) = (desktop.snapshot(), phone.snapshot());
    let mut ed = sd.events.clone();
    ed[0].start += 1800;
    ed[0].end += 1800;
    ed[0].title = "Dentist (Dr. Lee)".into();
    let mut ep = sp.events.clone();
    ep[0].location = "12 Main St".into();
    ep[0].title = "Annual Dentist".into();
    ep.push(event(11, "Flight", 1_800_100_000));
    desktop.apply(&sd.heads, &sd.calendars, &ed);
    phone.apply(&sp.heads, &sp.calendars, &ep);
    let merged = |e: &Event| e.start == 1_800_001_800 && e.location == "12 Main St" && e.title == "Annual Dentist (Dr. Lee)";
    wait_for("concurrent edits merge on desktop", || find(&desktop, 10).is_some_and(|e| merged(&e)) && find(&desktop, 11).is_some());
    wait_for("concurrent edits merge on phone", || find(&phone, 10).is_some_and(|e| merged(&e)));

    // Deleting on the phone deletes on the desktop.
    let s = phone.snapshot();
    let events: Vec<Event> = s.events.iter().filter(|e| e.id != 11).cloned().collect();
    phone.apply(&s.heads, &s.calendars, &events);
    wait_for("delete reaches desktop", || find(&desktop, 11).is_none());

    drop((desktop, client, phone, daemon));
    let _ = std::fs::remove_dir_all(&root);
}
