//! Calendar's data: calendars and events in one Automerge document
//! (`calendar`), synced between devices with y7-sync.
//!
//! Layout: the root holds `calendar:<id>` and `event:<id>` maps (see
//! [`y7_sync::doc`] for why everything sits at the root). Titles and notes are
//! collaborative text, so concurrent edits merge by character; other fields
//! are last-writer-wins per field, so moving an event on one device and
//! renaming it on another keeps both changes.

pub mod dates;
pub mod ics;
pub mod model;
pub mod recur;

use std::sync::{Arc, Mutex};

use y7_sync::automerge::AutoCommit;
use y7_sync::doc::{get_bool, get_i64, get_str, put_bool, put_i64, put_str, put_text, record, records, retain_records};
pub use y7_sync::{Doc, Heads, Hub};

pub use model::{COLORS, Calendar, Event, color_rgb, default_calendars};
pub use recur::{Freq, Occurrence, Rule, occurrences};

/// The document's id.
pub const DOC: &str = "calendar";

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub heads: Heads,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
}

pub fn snapshot(doc: &mut Doc) -> Snapshot {
    let heads = doc.heads();
    let (calendars, events) = read(doc.read());
    Snapshot { heads, calendars, events }
}

/// Makes the document match `calendars` and `events`, edited starting from
/// the version `base` (anything that arrived since is kept). Returns the new
/// version.
pub fn apply(doc: &mut Doc, base: &Heads, calendars: &[Calendar], events: &[Event]) -> Heads {
    doc.edit(base, |am| write(am, calendars, events))
}

/// Gives a document without calendars the default ones.
pub fn ensure_defaults(doc: &mut Doc) {
    let s = snapshot(doc);
    if s.calendars.is_empty() {
        apply(doc, &s.heads, &default_calendars(), &s.events);
    }
}

fn read(doc: &AutoCommit) -> (Vec<Calendar>, Vec<Event>) {
    let mut calendars: Vec<(i64, Calendar)> = records(doc, "calendar")
        .into_iter()
        .map(|(id, obj)| {
            let cal = Calendar {
                id,
                name: get_str(doc, &obj, "name").unwrap_or_else(|| "Calendar".into()),
                color: get_str(doc, &obj, "color").unwrap_or_else(|| "blue".into()),
            };
            (get_i64(doc, &obj, "created").unwrap_or(0), cal)
        })
        .collect();
    calendars.sort_by_key(|(created, c)| (*created, c.id));
    let calendars: Vec<Calendar> = calendars.into_iter().map(|(_, c)| c).collect();
    let mut events: Vec<Event> = records(doc, "event")
        .into_iter()
        .map(|(id, obj)| Event {
            id,
            calendar: get_i64(doc, &obj, "calendar").unwrap_or(0) as u64,
            title: get_str(doc, &obj, "title").unwrap_or_default(),
            start: get_i64(doc, &obj, "start").unwrap_or(0),
            end: get_i64(doc, &obj, "end").unwrap_or(0),
            all_day: get_bool(doc, &obj, "all_day").unwrap_or(false),
            location: get_str(doc, &obj, "location").unwrap_or_default(),
            notes: get_str(doc, &obj, "notes").unwrap_or_default(),
            rrule: get_str(doc, &obj, "rrule").unwrap_or_default(),
            created: get_i64(doc, &obj, "created").unwrap_or(0),
            modified: get_i64(doc, &obj, "modified").unwrap_or(0),
        })
        .collect();
    // An event whose calendar was deleted on another device joins the first one.
    if let Some(first) = calendars.first() {
        for e in &mut events {
            if !calendars.iter().any(|c| c.id == e.calendar) {
                e.calendar = first.id;
            }
        }
    }
    events.sort_by_key(|e| (e.start, e.id));
    (calendars, events)
}

fn write(doc: &mut AutoCommit, calendars: &[Calendar], events: &[Event]) {
    let t = y7_sync::now();
    for (i, c) in calendars.iter().enumerate() {
        let obj = record(doc, "calendar", c.id);
        put_str(doc, &obj, "name", &c.name);
        put_str(doc, &obj, "color", &c.color);
        if get_i64(doc, &obj, "created").is_none() {
            // Defaults sort first and the same everywhere.
            put_i64(doc, &obj, "created", if c.id <= 2 { i as i64 } else { t });
        }
    }
    for e in events {
        let obj = record(doc, "event", e.id);
        put_i64(doc, &obj, "calendar", e.calendar as i64);
        put_text(doc, &obj, "title", &e.title);
        put_i64(doc, &obj, "start", e.start);
        put_i64(doc, &obj, "end", e.end);
        put_bool(doc, &obj, "all_day", e.all_day);
        put_str(doc, &obj, "location", &e.location);
        put_text(doc, &obj, "notes", &e.notes);
        put_str(doc, &obj, "rrule", &e.rrule);
        put_i64(doc, &obj, "created", e.created);
        put_i64(doc, &obj, "modified", e.modified);
    }
    retain_records(doc, "calendar", |id| calendars.iter().any(|c| c.id == id));
    retain_records(doc, "event", |id| events.iter().any(|e| e.id == id));
}

/// The calendar document plus sync, as used by an app.
pub struct Core {
    hub: Arc<dyn Hub>,
    doc: Arc<Mutex<Doc>>,
}

impl Core {
    pub fn new(hub: Arc<dyn Hub>) -> Self {
        let doc = hub.doc(DOC);
        ensure_defaults(&mut doc.lock().unwrap());
        hub.local_changed(DOC);
        Core { hub, doc }
    }

    pub fn snapshot(&self) -> Snapshot {
        snapshot(&mut self.doc.lock().unwrap())
    }

    /// Saves an edit made starting from `base` and sends it on. See [`apply`].
    pub fn apply(&self, base: &Heads, calendars: &[Calendar], events: &[Event]) -> Heads {
        let heads = apply(&mut self.doc.lock().unwrap(), base, calendars, events);
        self.hub.local_changed(DOC);
        heads
    }

    pub fn sync(&self) -> &dyn Hub {
        &*self.hub
    }

    pub fn hub(&self) -> Arc<dyn Hub> {
        self.hub.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            created: 0,
            modified: 0,
        }
    }

    #[test]
    fn round_trips_and_merges_field_edits() {
        let mut a = Doc::in_memory();
        ensure_defaults(&mut a);
        let s = snapshot(&mut a);
        assert_eq!(s.calendars.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Home", "Work"]);
        apply(&mut a, &s.heads, &s.calendars, &[event(5, "Lunch", 1000)]);
        let mut b = Doc::in_memory();
        b.merge_bytes(&a.save_bytes()).unwrap();

        // One device moves the event, the other renames it.
        let sa = snapshot(&mut a);
        let mut ea = sa.events.clone();
        ea[0].start += 7200;
        ea[0].end += 7200;
        apply(&mut a, &sa.heads, &sa.calendars, &ea);
        let sb = snapshot(&mut b);
        let mut eb = sb.events.clone();
        eb[0].title = "Lunch with Sam".into();
        eb[0].rrule = "FREQ=WEEKLY".into();
        apply(&mut b, &sb.heads, &sb.calendars, &eb);

        let (ab, bb) = (a.save_bytes(), b.save_bytes());
        a.merge_bytes(&bb).unwrap();
        b.merge_bytes(&ab).unwrap();
        let (ea, eb) = (snapshot(&mut a).events, snapshot(&mut b).events);
        assert_eq!(ea, eb);
        assert_eq!((ea[0].title.as_str(), ea[0].start, ea[0].rrule.as_str()), ("Lunch with Sam", 8200, "FREQ=WEEKLY"));
    }

    #[test]
    fn orphaned_events_join_the_first_calendar() {
        let mut a = Doc::in_memory();
        ensure_defaults(&mut a);
        let s = snapshot(&mut a);
        let mut e = event(5, "Gym", 0);
        e.calendar = 2;
        apply(&mut a, &s.heads, &s.calendars, &[e]);
        let s = snapshot(&mut a);
        apply(&mut a, &s.heads, &s.calendars[..1], &s.events);
        assert_eq!(snapshot(&mut a).events[0].calendar, 1);
    }
}
