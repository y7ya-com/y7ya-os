use std::sync::Arc;

use calendar_core::model::{date_key, local};
use calendar_core::{Calendar, Core, Event, Heads, Occurrence, default_calendars, occurrences};
use chrono::{Duration, NaiveDate};

use super::layout::at;

/// The calendar as the UI sees it: a working copy of the shared document.
/// Edit `calendars` and `events` freely, then `save()` to merge them into the
/// document (and on to paired devices).
#[derive(Default)]
pub struct Store {
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    /// The document version this copy was loaded from.
    heads: Heads,
    pub core: Option<Arc<Core>>,
}

impl Store {
    pub fn new(core: Arc<Core>) -> Self {
        let mut store = Store { core: Some(core), ..Default::default() };
        store.reload();
        store
    }

    /// An in-memory store that isn't saved (previews and tests).
    pub fn detached() -> Self {
        Store { calendars: default_calendars(), ..Default::default() }
    }

    /// Takes the document's current state, including edits from elsewhere.
    pub fn reload(&mut self) {
        let Some(core) = &self.core else { return };
        let s = core.snapshot();
        self.calendars = s.calendars;
        self.events = s.events;
        self.heads = s.heads;
    }

    /// Merges local edits into the document. Returns true if edits from
    /// another device arrived meanwhile, so the store now has new content.
    pub fn save(&mut self) -> bool {
        let Some(core) = &self.core else { return false };
        let heads = core.apply(&self.heads, &self.calendars, &self.events);
        let s = core.snapshot();
        let changed = s.calendars != self.calendars || s.events != self.events;
        self.calendars = s.calendars;
        self.events = s.events;
        self.heads = if changed { s.heads } else { heads };
        changed
    }

    pub fn event(&self, id: u64) -> Option<&Event> {
        self.events.iter().find(|e| e.id == id)
    }

    pub fn event_mut(&mut self, id: u64) -> Option<&mut Event> {
        self.events.iter_mut().find(|e| e.id == id)
    }

    pub fn calendar(&self, id: u64) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == id)
    }

    pub fn color_of(&self, event: &Event) -> &str {
        self.calendar(event.calendar).map_or("blue", |c| c.color.as_str())
    }

    /// Every occurrence touching `first..=last` of events in calendars not
    /// in `hidden`, in start order (all-day first on the same day).
    pub fn occurrences(&self, first: NaiveDate, last: NaiveDate, hidden: &[u64]) -> Vec<Occurrence> {
        let mut out: Vec<Occurrence> = self
            .events
            .iter()
            .filter(|e| !hidden.contains(&e.calendar))
            .flat_map(|e| occurrences(e, first, last))
            .collect();
        out.sort_by_key(|o| (!o.all_day, o.start, std::cmp::Reverse(o.end), o.event));
        out
    }

    /// Adds the events of an `.ics` file to a calendar named after it (or the
    /// first calendar). Returns how many were added.
    pub fn import(&mut self, text: &str) -> usize {
        let import = calendar_core::ics::parse(text);
        let calendar = match import.name.filter(|n| !n.trim().is_empty()) {
            Some(name) => match self.calendars.iter().find(|c| c.name == name) {
                Some(c) => c.id,
                None => {
                    let id = y7_sync::new_id();
                    let color = calendar_core::COLORS[self.calendars.len() % calendar_core::COLORS.len()];
                    self.calendars.push(Calendar { id, name, color: color.into() });
                    id
                }
            },
            None => self.calendars.first().map_or(0, |c| c.id),
        };
        let n = import.events.len();
        for mut e in import.events {
            e.id = y7_sync::new_id();
            e.calendar = calendar;
            if e.title.trim().is_empty() {
                e.title = "New Event".into();
            }
            self.events.push(e);
        }
        n
    }
}

/// "9 AM", "Noon", "2:30 PM".
pub fn hour_label(h: u32) -> String {
    match h {
        0 | 24 => "12 AM".into(),
        12 => "Noon".into(),
        1..=11 => format!("{h} AM"),
        _ => format!("{} PM", h - 12),
    }
}

/// "9:30 AM" for an instant.
pub fn time_label(ts: i64) -> String {
    local(ts).format("%-I:%M %p").to_string()
}

/// Sample content for `--snapshot` renders and tests, around `today`.
pub fn demo_store(today: NaiveDate) -> Store {
    let mut s = Store::detached();
    s.calendars.push(Calendar { id: 900, name: "Family".into(), color: "green".into() });
    s.calendars.push(Calendar { id: 901, name: "Fitness".into(), color: "orange".into() });
    s.calendars.push(Calendar { id: 902, name: "Holidays".into(), color: "purple".into() });
    let mut next = 1000;
    let mut add = |cal: u64, title: &str, day: i64, from: (i32, i32), to: (i32, i32), rrule: &str, location: &str| {
        let d = today + Duration::days(day);
        let start = at(d, from.0 * 60 + from.1);
        let end = at(d, to.0 * 60 + to.1);
        next += 1;
        s.events.push(Event {
            id: next,
            calendar: cal,
            title: title.into(),
            start,
            end,
            all_day: false,
            location: location.into(),
            notes: String::new(),
            rrule: rrule.into(),
            created: start,
            modified: start,
        });
        next
    };
    add(2, "Team Standup", -28, (9, 0), (9, 30), "FREQ=WEEKLY", "");
    add(2, "Weekly Sync", -29, (9, 30), (10, 0), "FREQ=WEEKLY", "Zoom");
    add(2, "Design Review", 0, (10, 0), (11, 30), "", "Studio B");
    add(1, "Coffee with Sam", 0, (10, 30), (11, 15), "", "Blue Bottle");
    add(1, "Lunch with Priya", 0, (12, 30), (13, 30), "", "Tartine");
    add(901, "Climbing", 0, (18, 0), (19, 30), "", "Mission Cliffs");
    add(2, "1:1 with Alex", -1, (14, 0), (14, 30), "", "");
    add(2, "Sprint Planning", -2, (13, 0), (15, 0), "", "Room 4");
    add(901, "Yoga", -3, (7, 30), (8, 30), "FREQ=WEEKLY", "");
    add(900, "Piano Lesson", -4, (16, 0), (17, 0), "FREQ=WEEKLY", "");
    add(1, "Dentist", 1, (8, 30), (9, 15), "", "12 Main St");
    add(2, "Product Demo", 2, (15, 0), (16, 30), "", "");
    add(900, "Dinner at Mom's", 3, (19, 0), (21, 0), "", "");
    add(2, "Quarterly Review", -5, (11, 0), (12, 30), "", "");
    add(1, "Haircut", 5, (17, 30), (18, 15), "", "");
    add(900, "Movie Night", -6, (20, 0), (22, 30), "", "");
    add(2, "Offsite Prep", 9, (10, 0), (12, 0), "", "");
    add(1, "Book Club", 12, (19, 0), (20, 30), "", "");
    add(900, "Soccer Practice", -9, (17, 0), (18, 30), "", "");
    add(2, "Interview", -11, (14, 0), (15, 0), "", "");
    add(1, "Farmers Market", -13, (9, 0), (11, 0), "", "");
    add(901, "Run Club", -8, (7, 0), (8, 0), "FREQ=WEEKLY;COUNT=6", "");
    add(2, "Budget Review", 6, (11, 0), (12, 0), "", "");
    let mut all_day = |cal: u64, title: &str, day: i64, days: i64, rrule: &str| {
        let d = today + Duration::days(day);
        next += 1;
        s.events.push(Event {
            id: next,
            calendar: cal,
            title: title.into(),
            start: date_key(d),
            end: date_key(d + Duration::days(days)),
            all_day: true,
            location: String::new(),
            notes: String::new(),
            rrule: rrule.into(),
            created: 0,
            modified: 0,
        });
    };
    all_day(2, "Web Summit", -1, 3, "");
    all_day(900, "Mom's Birthday", 2, 1, "FREQ=YEARLY");
    all_day(902, "Day Off", 8, 1, "");
    all_day(1, "Trip to Tahoe", 15, 3, "");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(hour_label(0), "12 AM");
        assert_eq!(hour_label(12), "Noon");
        assert_eq!(hour_label(15), "3 PM");
    }

    #[test]
    fn imports_into_a_named_calendar() {
        let mut s = Store::detached();
        let ics = "BEGIN:VCALENDAR\nX-WR-CALNAME:Holidays\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:20261225\nSUMMARY:Christmas\nEND:VEVENT\nEND:VCALENDAR\n";
        assert_eq!(s.import(ics), 1);
        let cal = s.calendars.iter().find(|c| c.name == "Holidays").unwrap().id;
        assert_eq!(s.events[0].calendar, cal);
        assert!(s.events[0].all_day);
        // A second import reuses the calendar.
        s.import(ics);
        assert_eq!(s.calendars.iter().filter(|c| c.name == "Holidays").count(), 1);
    }

    #[test]
    fn occurrences_skip_hidden_calendars() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();
        let s = demo_store(today);
        let all = s.occurrences(today, today, &[]);
        let some = s.occurrences(today, today, &[2]);
        assert!(all.len() > some.len());
        assert!(all.iter().any(|o| s.event(o.event).unwrap().title == "Team Standup"), "weekly repeats show up");
        assert!(all[0].all_day, "all-day events come first");
    }
}
