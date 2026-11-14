//! Reminders' data: lists and reminders in one Automerge document
//! (`reminders`), synced between devices with y7-sync.
//!
//! Layout: the root holds `list:<id>` and `reminder:<id>` maps (see
//! [`y7_sync::doc`] for why everything sits at the root). Titles and notes are
//! collaborative text; every other field is last-writer-wins on its own, so
//! completing a reminder on the phone while renaming it here keeps both.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use y7_sync::automerge::AutoCommit;
use y7_sync::doc::{get_bool, get_i64, get_str, put_bool, put_i64, put_opt_i64, put_opt_u64, put_str, put_text, record, records, retain_records};
pub use y7_sync::{Doc, Heads, Hub, new_id, now};

/// The document's id.
pub const DOC: &str = "reminders";

/// The list every device starts with. A fixed id, so devices that start
/// separately share it.
pub const DEFAULT_LIST: u64 = 0;

/// Colors lists can have, as in macOS Reminders.
pub const COLORS: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "brown"];

pub fn color_rgb(name: &str) -> u32 {
    match name {
        "red" => 0xff3b30,
        "orange" => 0xff9500,
        "yellow" => 0xffcc00,
        "green" => 0x34c759,
        "purple" => 0xaf52de,
        "brown" => 0xa2845e,
        _ => 0x007aff,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List {
    pub id: u64,
    pub name: String,
    /// One of [`COLORS`].
    pub color: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    #[default]
    None,
    Low,
    Medium,
    High,
}

impl Priority {
    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Priority::Low,
            2 => Priority::Medium,
            3.. => Priority::High,
            _ => Priority::None,
        }
    }

    pub fn as_i64(self) -> i64 {
        self as i64
    }

    /// `!`, `!!`, `!!!` as Reminders shows it.
    pub fn marks(self) -> &'static str {
        ["", "!", "!!", "!!!"][self as usize]
    }
}

/// When a reminder is due: a day, or a day and time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    /// Midnight UTC of the date (a floating date, the same in every zone).
    Date(i64),
    /// An instant (Unix seconds).
    Time(i64),
}

impl Due {
    pub fn date(date: NaiveDate) -> Due {
        Due::Date(date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
    }

    /// The local day it's due.
    pub fn day(self) -> NaiveDate {
        match self {
            Due::Date(k) => DateTime::<Utc>::from_timestamp(k, 0).map_or(NaiveDate::MIN, |t| t.date_naive()),
            Due::Time(t) => Local.timestamp_opt(t, 0).single().map_or(NaiveDate::MIN, |t| t.date_naive()),
        }
    }

    /// Sorting key: date-only reminders come first in their day.
    pub fn sort_key(self) -> (NaiveDate, i64) {
        match self {
            Due::Date(_) => (self.day(), i64::MIN),
            Due::Time(t) => (self.day(), t),
        }
    }

    /// Overdue: a past day, or a time that has passed.
    pub fn is_overdue(self, now: i64) -> bool {
        match self {
            Due::Date(_) => self.day() < Local::now().date_naive(),
            Due::Time(t) => t < now,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub id: u64,
    pub list: u64,
    pub title: String,
    pub notes: String,
    pub due: Option<Due>,
    pub priority: Priority,
    pub flagged: bool,
    /// When it was checked off.
    pub completed: Option<i64>,
    /// The reminder this is a subtask of (one level).
    pub parent: Option<u64>,
    pub created: i64,
    pub modified: i64,
}

impl Reminder {
    pub fn new(list: u64, title: impl Into<String>) -> Self {
        let t = now();
        Reminder {
            id: new_id(),
            list,
            title: title.into(),
            notes: String::new(),
            due: None,
            priority: Priority::None,
            flagged: false,
            completed: None,
            parent: None,
            created: t,
            modified: t,
        }
    }

    pub fn is_done(&self) -> bool {
        self.completed.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub heads: Heads,
    pub lists: Vec<List>,
    pub reminders: Vec<Reminder>,
}

pub fn snapshot(doc: &mut Doc) -> Snapshot {
    let heads = doc.heads();
    let (lists, reminders) = read(doc.read());
    Snapshot { heads, lists, reminders }
}

/// Makes the document match `lists` and `reminders`, edited starting from the
/// version `base` (anything that arrived since is kept).
pub fn apply(doc: &mut Doc, base: &Heads, lists: &[List], reminders: &[Reminder]) -> Heads {
    doc.edit(base, |am| write(am, lists, reminders))
}

/// Gives a document without lists the default "Reminders" list.
pub fn ensure_defaults(doc: &mut Doc) {
    let s = snapshot(doc);
    if s.lists.is_empty() {
        let list = List { id: DEFAULT_LIST, name: "Reminders".into(), color: "blue".into() };
        apply(doc, &s.heads, &[list], &s.reminders);
    }
}

fn read(doc: &AutoCommit) -> (Vec<List>, Vec<Reminder>) {
    let mut lists: Vec<(i64, List)> = records(doc, "list")
        .into_iter()
        .map(|(id, obj)| {
            let list = List {
                id,
                name: get_str(doc, &obj, "name").unwrap_or_else(|| "Reminders".into()),
                color: get_str(doc, &obj, "color").unwrap_or_else(|| "blue".into()),
            };
            (get_i64(doc, &obj, "created").unwrap_or(0), list)
        })
        .collect();
    lists.sort_by_key(|(created, l)| (l.id != DEFAULT_LIST, *created, l.id));
    let lists: Vec<List> = lists.into_iter().map(|(_, l)| l).collect();
    let mut reminders: Vec<Reminder> = records(doc, "reminder")
        .into_iter()
        .map(|(id, obj)| {
            let due = match (get_i64(doc, &obj, "due"), get_bool(doc, &obj, "due_time")) {
                (Some(t), Some(true)) => Some(Due::Time(t)),
                (Some(k), _) => Some(Due::Date(k)),
                _ => None,
            };
            Reminder {
                id,
                list: get_i64(doc, &obj, "list").unwrap_or(0) as u64,
                title: get_str(doc, &obj, "title").unwrap_or_default(),
                notes: get_str(doc, &obj, "notes").unwrap_or_default(),
                due,
                priority: Priority::from_i64(get_i64(doc, &obj, "priority").unwrap_or(0)),
                flagged: get_bool(doc, &obj, "flagged").unwrap_or(false),
                completed: get_i64(doc, &obj, "completed"),
                parent: get_i64(doc, &obj, "parent").map(|p| p as u64),
                created: get_i64(doc, &obj, "created").unwrap_or(0),
                modified: get_i64(doc, &obj, "modified").unwrap_or(0),
            }
        })
        .collect();
    // Reminders in a list deleted on another device go to the first list;
    // subtasks whose parent is gone stand on their own.
    let ids: Vec<u64> = reminders.iter().map(|r| r.id).collect();
    for r in &mut reminders {
        if let Some(first) = lists.first().filter(|_| !lists.iter().any(|l| l.id == r.list)) {
            r.list = first.id;
        }
        if r.parent.is_some_and(|p| !ids.contains(&p) || p == r.id) {
            r.parent = None;
        }
    }
    reminders.sort_by_key(|r| (r.created, r.id));
    (lists, reminders)
}

fn write(doc: &mut AutoCommit, lists: &[List], reminders: &[Reminder]) {
    let t = now();
    for l in lists {
        let obj = record(doc, "list", l.id);
        put_str(doc, &obj, "name", &l.name);
        put_str(doc, &obj, "color", &l.color);
        if get_i64(doc, &obj, "created").is_none() {
            put_i64(doc, &obj, "created", if l.id == DEFAULT_LIST { 0 } else { t });
        }
    }
    for r in reminders {
        let obj = record(doc, "reminder", r.id);
        put_i64(doc, &obj, "list", r.list as i64);
        put_text(doc, &obj, "title", &r.title);
        put_text(doc, &obj, "notes", &r.notes);
        match r.due {
            Some(Due::Date(k)) => {
                put_i64(doc, &obj, "due", k);
                put_bool(doc, &obj, "due_time", false);
            }
            Some(Due::Time(t)) => {
                put_i64(doc, &obj, "due", t);
                put_bool(doc, &obj, "due_time", true);
            }
            None => put_opt_i64(doc, &obj, "due", None),
        }
        put_i64(doc, &obj, "priority", r.priority.as_i64());
        put_bool(doc, &obj, "flagged", r.flagged);
        put_opt_i64(doc, &obj, "completed", r.completed);
        put_opt_u64(doc, &obj, "parent", r.parent);
        put_i64(doc, &obj, "created", r.created);
        put_i64(doc, &obj, "modified", r.modified);
    }
    retain_records(doc, "list", |id| lists.iter().any(|l| l.id == id));
    retain_records(doc, "reminder", |id| reminders.iter().any(|r| r.id == id));
}

/// The reminders document plus sync, as used by an app.
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
    pub fn apply(&self, base: &Heads, lists: &[List], reminders: &[Reminder]) -> Heads {
        let heads = apply(&mut self.doc.lock().unwrap(), base, lists, reminders);
        self.hub.local_changed(DOC);
        heads
    }

    pub fn sync(&self) -> &dyn Hub {
        &*self.hub
    }

    /// The hub, for the pairing sheet.
    pub fn hub(&self) -> Arc<dyn Hub> {
        self.hub.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_field() {
        let mut doc = Doc::in_memory();
        ensure_defaults(&mut doc);
        let s = snapshot(&mut doc);
        assert_eq!(s.lists[0].name, "Reminders");
        let mut parent = Reminder::new(DEFAULT_LIST, "Trip");
        parent.due = Some(Due::date(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()));
        parent.priority = Priority::High;
        parent.flagged = true;
        parent.notes = "Pack light".into();
        let mut child = Reminder::new(DEFAULT_LIST, "Passport");
        child.parent = Some(parent.id);
        child.due = Some(Due::Time(1_800_000_000));
        child.completed = Some(1_700_000_000);
        apply(&mut doc, &s.heads, &s.lists, &[parent.clone(), child.clone()]);
        let mut got = snapshot(&mut doc).reminders;
        got.sort_by_key(|r| r.parent.is_some());
        assert_eq!(got, [parent.clone(), child.clone()]);
        assert_eq!(got[0].due.unwrap().day(), NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());

        // Clearing optional fields removes them.
        let s = snapshot(&mut doc);
        let mut rs = s.reminders.clone();
        for r in &mut rs {
            r.due = None;
            r.completed = None;
            r.parent = None;
        }
        apply(&mut doc, &s.heads, &s.lists, &rs);
        assert!(snapshot(&mut doc).reminders.iter().all(|r| r.due.is_none() && r.completed.is_none() && r.parent.is_none()));
    }

    #[test]
    fn subtasks_of_deleted_reminders_stand_alone() {
        let mut doc = Doc::in_memory();
        ensure_defaults(&mut doc);
        let s = snapshot(&mut doc);
        let parent = Reminder::new(DEFAULT_LIST, "Trip");
        let mut child = Reminder::new(DEFAULT_LIST, "Passport");
        child.parent = Some(parent.id);
        apply(&mut doc, &s.heads, &s.lists, &[parent, child.clone()]);
        let s = snapshot(&mut doc);
        apply(&mut doc, &s.heads, &s.lists, &[child]);
        assert_eq!(snapshot(&mut doc).reminders[0].parent, None);
    }

    #[test]
    fn priorities() {
        assert_eq!(Priority::from_i64(2).marks(), "!!");
        assert!(Priority::High > Priority::Low);
    }
}
