use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
pub use reminders_core::{Core, DEFAULT_LIST, Due, List, Priority, Reminder, color_rgb, now};
use reminders_core::Heads;

/// The reminders as the UI sees them: a working copy of the shared document.
/// Edit `lists` and `reminders` freely, then `save()` to merge them into the
/// document (and on to the other apps and paired devices).
#[derive(Default)]
pub struct Store {
    pub lists: Vec<List>,
    pub reminders: Vec<Reminder>,
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
        Store {
            lists: vec![List { id: DEFAULT_LIST, name: "Reminders".into(), color: "blue".into() }],
            ..Default::default()
        }
    }

    /// Takes the document's current state, including edits from elsewhere.
    pub fn reload(&mut self) {
        let Some(core) = &self.core else { return };
        let s = core.snapshot();
        self.lists = s.lists;
        self.reminders = s.reminders;
        self.heads = s.heads;
    }

    /// Merges local edits into the document. Returns true if edits from
    /// elsewhere arrived meanwhile, so the store now has new content.
    pub fn save(&mut self) -> bool {
        let Some(core) = &self.core else { return false };
        let heads = core.apply(&self.heads, &self.lists, &self.reminders);
        let s = core.snapshot();
        let changed = s.lists != self.lists || s.reminders != self.reminders;
        self.lists = s.lists;
        self.reminders = s.reminders;
        self.heads = if changed { s.heads } else { heads };
        changed
    }

    pub fn get(&self, id: u64) -> Option<&Reminder> {
        self.reminders.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Reminder> {
        self.reminders.iter_mut().find(|r| r.id == id)
    }

    pub fn list(&self, id: u64) -> Option<&List> {
        self.lists.iter().find(|l| l.id == id)
    }

    /// The list's color, as `0xrrggbb`.
    pub fn list_color(&self, id: u64) -> u32 {
        color_rgb(self.list(id).map_or("blue", |l| l.color.as_str()))
    }

    pub fn children(&self, id: u64) -> impl Iterator<Item = &Reminder> {
        self.reminders.iter().filter(move |r| r.parent == Some(id))
    }

    /// Incomplete reminders in a list.
    pub fn count_in(&self, list: u64) -> usize {
        self.reminders.iter().filter(|r| r.list == list && !r.is_done()).count()
    }

    pub fn unique_list_name(&self, base: &str) -> String {
        let taken = |n: &str| self.lists.iter().any(|l| l.name.eq_ignore_ascii_case(n));
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap()
    }

    /// Where a reminder added right after `after` sorts: between it and the
    /// next sibling (`created` orders reminders within a list).
    pub fn created_after(&self, after: &Reminder) -> i64 {
        let next = self
            .reminders
            .iter()
            .filter(|r| r.list == after.list && r.parent == after.parent && (r.created, r.id) > (after.created, after.id))
            .map(|r| r.created)
            .min();
        match next {
            Some(n) if n - after.created > 1 => after.created + (n - after.created) / 2,
            Some(_) => after.created,
            None => now().max(after.created + 1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Smart {
    Today,
    Scheduled,
    All,
    Flagged,
    Completed,
}

impl Smart {
    #[cfg(test)]
    pub const ALL: [Smart; 5] = [Smart::Today, Smart::Scheduled, Smart::All, Smart::Flagged, Smart::Completed];

    pub fn name(self) -> &'static str {
        match self {
            Smart::Today => "Today",
            Smart::Scheduled => "Scheduled",
            Smart::All => "All",
            Smart::Flagged => "Flagged",
            Smart::Completed => "Completed",
        }
    }

    /// The tile color in light and dark appearance.
    pub fn color(self, dark: bool) -> u32 {
        match (self, dark) {
            (Smart::Today, false) => 0x007aff,
            (Smart::Today, true) => 0x0a84ff,
            (Smart::Scheduled, false) => 0xff3b30,
            (Smart::Scheduled, true) => 0xff453a,
            (Smart::All, false) => 0x5b5b60,
            (Smart::All, true) => 0x8e8e93,
            (Smart::Flagged, _) => 0xff9500,
            (Smart::Completed, false) => 0x8e8e93,
            (Smart::Completed, true) => 0x7c7c80,
        }
    }

    /// Whether `r` belongs in this smart list.
    pub fn matches(self, r: &Reminder, today: NaiveDate) -> bool {
        match self {
            Smart::Today => !r.is_done() && r.due.is_some_and(|d| d.day() <= today),
            Smart::Scheduled => !r.is_done() && r.due.is_some(),
            Smart::All => !r.is_done(),
            Smart::Flagged => !r.is_done() && r.flagged,
            Smart::Completed => r.is_done(),
        }
    }

    pub fn count(self, store: &Store, today: NaiveDate) -> usize {
        store.reminders.iter().filter(|r| self.matches(r, today)).count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Smart(Smart),
    List(u64),
}

/// A run of rows under an optional heading.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: Option<String>,
    /// Heading color (the list's color in All), `None` for the label color.
    pub color: Option<u32>,
    pub rows: Vec<Row>,
    /// Reminders added from this section's empty space go to this list.
    pub list: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    /// 1 for a subtask shown under its parent.
    pub depth: u8,
}

/// A list's reminders in order: each top-level reminder followed by its
/// subtasks. `visible` decides which reminders show (completed ones only when
/// asked for, or ones checked off just now).
fn tree(store: &Store, list: u64, visible: &dyn Fn(&Reminder) -> bool) -> Vec<Row> {
    let order = |rs: &mut Vec<&Reminder>| rs.sort_by_key(|r| (r.created, r.id));
    let mut top: Vec<&Reminder> = store.reminders.iter().filter(|r| r.list == list && r.parent.is_none()).collect();
    order(&mut top);
    let mut rows = Vec::new();
    for r in top {
        let mut kids: Vec<&Reminder> = store.children(r.id).filter(|c| visible(c)).collect();
        order(&mut kids);
        let kids: Vec<Row> = kids.iter().map(|c| Row { id: c.id, depth: 1 }).collect();
        if visible(r) || !kids.is_empty() {
            rows.push(Row { id: r.id, depth: 0 });
            rows.extend(kids);
        }
    }
    rows
}

/// What a view shows: its sections of rows.
///
/// `keep` holds reminders checked off since the view opened; they stay put
/// (struck through) until the user moves on, as in Reminders.
pub fn sections(store: &Store, view: View, show_completed: bool, keep: &HashSet<u64>, query: &str, today: NaiveDate) -> Vec<Section> {
    let query = query.trim().to_lowercase();
    if !query.is_empty() {
        let hit = |r: &Reminder| r.title.to_lowercase().contains(&query) || r.notes.to_lowercase().contains(&query);
        return by_list(store, &|r| hit(r) && (show_completed || !r.is_done() || keep.contains(&r.id)));
    }
    let open = |r: &Reminder| !r.is_done() || keep.contains(&r.id);
    match view {
        View::List(list) => {
            let rows = tree(store, list, &|r| show_completed || open(r));
            vec![Section { title: None, color: None, rows, list: Some(list) }]
        }
        View::Smart(Smart::All) => by_list(store, &|r| show_completed || open(r)),
        View::Smart(Smart::Today) => {
            let mut rs: Vec<&Reminder> =
                store.reminders.iter().filter(|r| Smart::Today.matches(r, today) || keep.contains(&r.id) && r.due.is_some()).collect();
            rs.sort_by_key(|r| (r.due.map(|d| d.sort_key()), r.created));
            let rows = rs.iter().map(|r| Row { id: r.id, depth: 0 }).collect();
            vec![Section { title: None, color: None, rows, list: None }]
        }
        View::Smart(Smart::Flagged) => {
            let rows = store
                .reminders
                .iter()
                .filter(|r| r.flagged && open(r))
                .map(|r| Row { id: r.id, depth: 0 })
                .collect();
            vec![Section { title: None, color: None, rows, list: None }]
        }
        View::Smart(Smart::Scheduled) => {
            let mut rs: Vec<&Reminder> = store.reminders.iter().filter(|r| r.due.is_some() && open(r)).collect();
            rs.sort_by_key(|r| (r.due.map(|d| d.sort_key()), r.created));
            group(rs, |r| scheduled_label(r.due.unwrap().day(), today))
        }
        View::Smart(Smart::Completed) => {
            let mut rs: Vec<&Reminder> = store.reminders.iter().filter(|r| r.is_done()).collect();
            rs.sort_by_key(|r| std::cmp::Reverse((r.completed, r.id)));
            group(rs, |r| completed_label(r.completed.unwrap_or(0), today))
        }
    }
}

/// One section per list, in the list's color (All and search results).
fn by_list(store: &Store, visible: &dyn Fn(&Reminder) -> bool) -> Vec<Section> {
    store
        .lists
        .iter()
        .map(|l| Section {
            title: Some(l.name.clone()),
            color: Some(color_rgb(&l.color)),
            rows: tree(store, l.id, visible),
            list: Some(l.id),
        })
        .filter(|s| !s.rows.is_empty())
        .collect()
}

/// Consecutive reminders with the same label share a section.
fn group(rs: Vec<&Reminder>, label: impl Fn(&Reminder) -> String) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for r in rs {
        let l = label(r);
        match out.last_mut() {
            Some(s) if s.title.as_deref() == Some(l.as_str()) => s.rows.push(Row { id: r.id, depth: 0 }),
            _ => out.push(Section { title: Some(l), color: None, rows: vec![Row { id: r.id, depth: 0 }], list: None }),
        }
    }
    out
}

/// Scheduled's headings: "Past Due", "Today", "Tomorrow", the weekday for the
/// rest of the coming week, then dates.
pub fn scheduled_label(day: NaiveDate, today: NaiveDate) -> String {
    match (day - today).num_days() {
        d if d < 0 => "Past Due".into(),
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        2..=6 => day.format("%A").to_string(),
        _ if day.year() == today.year() => day.format("%A, %B %-d").to_string(),
        _ => day.format("%A, %B %-d, %Y").to_string(),
    }
}

/// Completed's headings, newest first.
pub fn completed_label(ts: i64, today: NaiveDate) -> String {
    let day = Local.timestamp_opt(ts, 0).single().map_or(today, |t| t.date_naive());
    match (today - day).num_days() {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => "Previous 7 Days".into(),
        _ if day.year() == today.year() => day.format("%B").to_string(),
        _ => day.year().to_string(),
    }
}

/// The due line under a reminder: "Today", "Tomorrow, 9:00 AM", "11/30/26".
pub fn due_text(due: Due, today: NaiveDate) -> String {
    let day = due.day();
    let date = match (day - today).num_days() {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        -1 => "Yesterday".to_string(),
        _ => day.format("%-m/%-d/%y").to_string(),
    };
    match due {
        Due::Date(_) => date,
        Due::Time(t) => format!("{date}, {}", time_text(t)),
    }
}

pub fn time_text(t: i64) -> String {
    Local.timestamp_opt(t, 0).single().map_or(String::new(), |t| t.format("%-I:%M %p").to_string())
}

/// The long date shown in the details popover: "Saturday, November 14, 2026".
pub fn long_date(day: NaiveDate) -> String {
    day.format("%a, %b %-d, %Y").to_string()
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Local instant for `day` at `minutes` past midnight.
pub fn at(day: NaiveDate, minutes: i64) -> i64 {
    let t = day.and_hms_opt(0, 0, 0).unwrap() + Duration::minutes(minutes);
    Local.from_local_datetime(&t).earliest().map_or_else(|| t.and_utc().timestamp(), |t| t.timestamp())
}

/// Minutes past local midnight of an instant.
pub fn minutes_of(t: i64) -> i64 {
    Local.timestamp_opt(t, 0).single().map_or(9 * 60, |t| {
        use chrono::Timelike;
        (t.hour() * 60 + t.minute()) as i64
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn scheduled_labels() {
        let today = d(2026, 11, 14); // A Saturday.
        assert_eq!(scheduled_label(d(2026, 11, 10), today), "Past Due");
        assert_eq!(scheduled_label(today, today), "Today");
        assert_eq!(scheduled_label(d(2026, 11, 15), today), "Tomorrow");
        assert_eq!(scheduled_label(d(2026, 11, 16), today), "Monday");
        assert_eq!(scheduled_label(d(2026, 11, 20), today), "Friday");
        assert_eq!(scheduled_label(d(2026, 11, 21), today), "Saturday, November 21");
        assert_eq!(scheduled_label(d(2027, 1, 4), today), "Monday, January 4, 2027");
    }

    #[test]
    fn due_texts() {
        let today = d(2026, 11, 14);
        assert_eq!(due_text(Due::date(today), today), "Today");
        assert_eq!(due_text(Due::date(d(2026, 11, 15)), today), "Tomorrow");
        assert_eq!(due_text(Due::date(d(2026, 11, 30)), today), "11/30/26");
        assert_eq!(due_text(Due::Time(at(today, 9 * 60 + 30)), today), "Today, 9:30 AM");
        assert_eq!(minutes_of(at(today, 17 * 60 + 15)), 17 * 60 + 15);
    }

    fn store(today: NaiveDate) -> Store {
        let mut s = Store::detached();
        s.lists.push(List { id: 7, name: "Groceries".into(), color: "orange".into() });
        let mut add = |list: u64, title: &str, f: &dyn Fn(&mut Reminder)| {
            let mut r = Reminder::new(list, title);
            r.created = s.reminders.len() as i64 + 10;
            f(&mut r);
            s.reminders.push(r);
        };
        add(DEFAULT_LIST, "Overdue", &|r| r.due = Some(Due::date(today - Duration::days(2))));
        add(DEFAULT_LIST, "Today", &|r| r.due = Some(Due::Time(at(today, 600))));
        add(DEFAULT_LIST, "Next week", &|r| r.due = Some(Due::date(today + Duration::days(9))));
        add(7, "Milk", &|r| r.flagged = true);
        add(7, "Eggs", &|r| r.completed = Some(1));
        add(7, "Done & flagged", &|r| {
            r.flagged = true;
            r.completed = Some(2);
        });
        s
    }

    fn titles(s: &Store, sections: &[Section]) -> Vec<Vec<String>> {
        sections.iter().map(|sec| sec.rows.iter().map(|r| s.get(r.id).unwrap().title.clone()).collect()).collect()
    }

    #[test]
    fn smart_lists_filter_and_count() {
        let today = d(2026, 11, 14);
        let s = store(today);
        let counts: Vec<usize> = Smart::ALL.iter().map(|m| m.count(&s, today)).collect();
        assert_eq!(counts, [2, 3, 4, 1, 2]); // Today, Scheduled, All, Flagged, Completed
        assert_eq!(s.count_in(7), 1);
        let none = HashSet::new();
        let today_view = sections(&s, View::Smart(Smart::Today), false, &none, "", today);
        assert_eq!(titles(&s, &today_view), [["Overdue", "Today"]]);
        let sched = sections(&s, View::Smart(Smart::Scheduled), false, &none, "", today);
        let heads: Vec<_> = sched.iter().map(|x| x.title.clone().unwrap()).collect();
        assert_eq!(heads, ["Past Due", "Today", "Monday, November 23"]);
        let all = sections(&s, View::Smart(Smart::All), false, &none, "", today);
        assert_eq!(titles(&s, &all), vec![vec!["Overdue", "Today", "Next week"], vec!["Milk"]]);
        assert_eq!(all[1].color, Some(color_rgb("orange")));
        let flagged = sections(&s, View::Smart(Smart::Flagged), false, &none, "", today);
        assert_eq!(titles(&s, &flagged), [["Milk"]]);
        let done = sections(&s, View::Smart(Smart::Completed), false, &none, "", today);
        assert_eq!(done.iter().map(|x| x.rows.len()).sum::<usize>(), 2);
        // Search looks through every list, titles and notes.
        let found = sections(&s, View::List(DEFAULT_LIST), false, &none, "MILK", today);
        assert_eq!(titles(&s, &found), [["Milk"]]);
    }

    #[test]
    fn lists_show_subtasks_under_parents_and_hide_completed() {
        let today = d(2026, 11, 14);
        let mut s = store(today);
        let parent = s.reminders.iter().find(|r| r.title == "Milk").unwrap().id;
        let mut kid = Reminder::new(7, "Oat milk");
        kid.parent = Some(parent);
        kid.created = 0; // Older than its parent: still listed under it.
        s.reminders.push(kid);
        let none = HashSet::new();
        let v = sections(&s, View::List(7), false, &none, "", today);
        assert_eq!(v[0].rows.iter().map(|r| r.depth).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(titles(&s, &v), [["Milk", "Oat milk"]]);
        let with_done = sections(&s, View::List(7), true, &none, "", today);
        assert_eq!(with_done[0].rows.len(), 4);
        // Just checked off: stays until the view changes.
        let eggs = s.reminders.iter().find(|r| r.title == "Eggs").unwrap().id;
        let keep = HashSet::from([eggs]);
        assert_eq!(sections(&s, View::List(7), false, &keep, "", today)[0].rows.len(), 3);
    }
}
