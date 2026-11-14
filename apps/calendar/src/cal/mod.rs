mod grid;
mod icons;
pub mod layout;
mod month;
pub mod model;
mod popover;
mod sidebar;
mod theme;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use calendar_core::dates::first_weekday;
use calendar_core::model::{date_key, day_start, local};
use calendar_core::{Calendar, Event, Freq, Rule};
use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use sync_ui::{Pairing, PairingHost, SyncEvents};
use ui::*;

use layout::{DAY_MIN, View, at, minute_of, y_to_minutes};
use model::Store;
use theme::Theme;

const SIDEBAR_W: f32 = 220.0;
const TOOLBAR_H: f32 = 52.0;
const SAVE_DELAY: StdDuration = StdDuration::from_millis(600);
/// A press that moves less than this is a click, not a drag.
const DRAG_SLOP: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum DragKind {
    /// Sweeping out a new event on empty grid, from `anchor` minutes into `day`.
    Create { day: NaiveDate, anchor: i32 },
    /// Moving the occurrence `[start, end)` of `event`, grabbed `grab` minutes in.
    Move { event: u64, start: i64, end: i64, grab: i32 },
    /// Dragging the bottom edge of the occurrence `[start, end)`.
    Resize { event: u64, start: i64, end: i64 },
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    kind: DragKind,
    from: Point,
    moved: bool,
    /// Where the event would land if dropped now.
    preview: (i64, i64),
}

/// The event popover.
pub struct Editor {
    event: u64,
    /// The occurrence being edited (repeating events move as a whole).
    start: i64,
    end: i64,
    /// What the popover points at, in window coordinates.
    anchor: Bounds,
}

#[derive(Clone)]
enum MenuAction {
    SetCalendar(u64),
    SetRepeat(Option<Freq>),
    SetColor(u64, &'static str),
    NewCalendar,
    RenameCalendar(u64),
    DeleteCalendar(u64),
}

struct MenuItem {
    label: String,
    color: Option<Color>,
    checked: bool,
    /// `None` draws a separator.
    action: Option<MenuAction>,
}

struct Menu {
    pos: Point,
    items: Vec<MenuItem>,
}

/// The popover's text fields.
struct Fields {
    title: Rc<RefCell<TextInput>>,
    location: Rc<RefCell<TextInput>>,
    notes: Rc<RefCell<TextInput>>,
    start_date: Rc<RefCell<TextInput>>,
    start_time: Rc<RefCell<TextInput>>,
    end_date: Rc<RefCell<TextInput>>,
    end_time: Rc<RefCell<TextInput>>,
    rename: Rc<RefCell<TextInput>>,
}

pub struct CalendarApp {
    store: Store,
    view: View,
    /// The day the views are built around.
    date: NaiveDate,
    /// The month the sidebar's mini calendar shows.
    mini_month: NaiveDate,
    first: Weekday,
    hidden: Vec<u64>,
    selected: Option<u64>,
    sidebar_open: bool,
    /// The hour grid's day columns, as last painted (window coordinates, scrolled).
    grid_bounds: Rc<Cell<Bounds>>,
    drag: Option<Drag>,
    editor: Option<Editor>,
    menu: Option<Menu>,
    renaming: Option<u64>,
    fields: Fields,
    dirty_since: Option<Instant>,
    events: Option<Arc<SyncEvents>>,
    pairing: Pairing,
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

impl CalendarApp {
    pub fn new(store: Store, events: Option<Arc<SyncEvents>>, cx: &mut Cx) -> Self {
        if let Some(events) = &events {
            events.set_waker(cx.waker());
        }
        let hub = store.core.as_ref().map(|c| c.hub());
        let fonts = &mut cx.text.fonts;
        let mut make = |placeholder: &str, single: bool| {
            let mut i = TextInput::new(fonts, TextStyle::default(), single);
            i.placeholder = placeholder.into();
            Rc::new(RefCell::new(i))
        };
        let fields = Fields {
            title: make("New Event", true),
            location: make("Add Location", true),
            notes: make("Add Notes", false),
            start_date: make("", true),
            start_time: make("", true),
            end_date: make("", true),
            end_time: make("", true),
            rename: make("", true),
        };
        cx.window_radius = 10.0;
        let today = today();
        let mut app = Self {
            store,
            view: View::Week,
            date: today,
            mini_month: today.with_day(1).unwrap(),
            first: first_weekday(),
            hidden: Vec::new(),
            selected: None,
            sidebar_open: true,
            grid_bounds: Rc::new(Cell::new(Bounds::default())),
            drag: None,
            editor: None,
            menu: None,
            renaming: None,
            fields,
            dirty_since: None,
            events,
            pairing: Pairing::new(hub),
        };
        app.scroll_to_now(cx);
        app
    }

    fn theme(cx: &Cx) -> Theme {
        if cx.dark { Theme::dark() } else { Theme::light() }
    }

    fn mark_dirty(&mut self) {
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    fn save(&mut self, cx: &mut Cx) {
        if self.dirty_since.take().is_some() && self.store.save() {
            self.after_reload(cx);
        }
    }

    /// The store picked up edits from elsewhere: drop what no longer exists.
    fn after_reload(&mut self, _cx: &mut Cx) {
        if self.selected.is_some_and(|id| self.store.event(id).is_none()) {
            self.selected = None;
        }
        if self.editor.as_ref().is_some_and(|e| self.store.event(e.event).is_none()) {
            self.editor = None;
        }
        if self.renaming.is_some_and(|id| self.store.calendar(id).is_none()) {
            self.renaming = None;
        }
    }

    /// Scrolls the hour grid to the morning, or to now if that's later.
    fn scroll_to_now(&mut self, cx: &mut Cx) {
        let visible = (cx.window_size.1 - 200.0).max(240.0) / layout::HOUR_H;
        let now = minute_of(y7_sync::now(), today());
        cx.scroll_to("grid-scroll", layout::initial_scroll(now, visible));
    }

    // -- navigation -----------------------------------------------------------

    pub fn set_view(&mut self, view: View, cx: &mut Cx) {
        self.menu = None;
        if self.view != view {
            self.view = view;
            self.close_editor(cx);
        }
    }

    pub fn go_to(&mut self, date: NaiveDate, cx: &mut Cx) {
        self.date = date;
        self.mini_month = date.with_day(1).unwrap();
        self.close_editor(cx);
    }

    fn step(&mut self, dir: i32, cx: &mut Cx) {
        let date = layout::step(self.view, self.date, dir);
        self.go_to(date, cx);
    }

    fn go_today(&mut self, cx: &mut Cx) {
        self.go_to(today(), cx);
        if matches!(self.view, View::Day | View::Week) {
            self.scroll_to_now(cx);
        }
    }

    /// The days shown as columns in the day/week grid.
    fn grid_days(&self) -> Vec<NaiveDate> {
        let (first, last) = layout::range(self.view, self.date, self.first);
        (0..=(last - first).num_days()).map(|i| first + Duration::days(i)).collect()
    }

    // -- editing --------------------------------------------------------------

    /// The calendar new events go into: the selected event's, else the first visible.
    fn default_calendar(&self) -> u64 {
        let visible = |c: &&Calendar| !self.hidden.contains(&c.id);
        self.selected
            .and_then(|id| self.store.event(id))
            .map(|e| e.calendar)
            .or_else(|| self.store.calendars.iter().find(visible).map(|c| c.id))
            .or_else(|| self.store.calendars.first().map(|c| c.id))
            .unwrap_or(0)
    }

    /// Adds an event and opens the popover on it.
    pub fn create_event(&mut self, start: i64, end: i64, all_day: bool, anchor: Bounds, cx: &mut Cx) -> u64 {
        let id = y7_sync::new_id();
        let t = y7_sync::now();
        self.store.events.push(Event {
            id,
            calendar: self.default_calendar(),
            title: "New Event".into(),
            start,
            end,
            all_day,
            location: String::new(),
            notes: String::new(),
            rrule: String::new(),
            created: t,
            modified: t,
        });
        self.mark_dirty();
        self.selected = Some(id);
        self.open_editor(id, start, end, anchor, cx);
        id
    }

    /// Ctrl+N and the toolbar's +: an hour-long event at the next full hour.
    pub fn new_event(&mut self, cx: &mut Cx) {
        let day = if self.date == today() { today() } else { self.date };
        let hour = if day == today() { (minute_of(y7_sync::now(), day) / 60 + 1).min(23) } else { 9 };
        let (w, h) = cx.window_size;
        let anchor = Bounds::new(w / 2.0 - 40.0, h / 3.0, 80.0, 40.0);
        self.create_event(at(day, hour * 60), at(day, hour * 60 + 60), false, anchor, cx);
    }

    /// Moves occurrence `[start, end)` of `id` to `[new_start, new_end)`; a
    /// repeating event moves as a whole.
    pub fn reschedule(&mut self, id: u64, (start, end): (i64, i64), (new_start, new_end): (i64, i64)) {
        if let Some(e) = self.store.event_mut(id) {
            e.start += new_start - start;
            e.end += new_end - end;
            e.end = e.end.max(e.start);
            e.modified = y7_sync::now();
            self.mark_dirty();
        }
    }

    pub fn delete_event(&mut self, id: u64, cx: &mut Cx) {
        self.store.events.retain(|e| e.id != id);
        if self.selected == Some(id) {
            self.selected = None;
        }
        if self.editor.as_ref().is_some_and(|e| e.event == id) {
            self.editor = None;
            cx.blur();
        }
        self.mark_dirty();
    }

    fn toggle_calendar(&mut self, id: u64) {
        if let Some(i) = self.hidden.iter().position(|&h| h == id) {
            self.hidden.remove(i);
        } else {
            self.hidden.push(id);
        }
    }

    fn run_menu(&mut self, action: MenuAction, cx: &mut Cx) {
        self.menu = None;
        match action {
            MenuAction::SetCalendar(cal) => {
                if let Some(e) = self.editor.as_ref().and_then(|ed| self.store.event_mut(ed.event)) {
                    e.calendar = cal;
                    self.mark_dirty();
                }
            }
            MenuAction::SetRepeat(freq) => {
                if let Some(e) = self.editor.as_ref().and_then(|ed| self.store.event_mut(ed.event)) {
                    e.rrule = freq.map(|f| Rule::new(f).to_rrule()).unwrap_or_default();
                    self.mark_dirty();
                }
            }
            MenuAction::SetColor(cal, color) => {
                if let Some(c) = self.store.calendars.iter_mut().find(|c| c.id == cal) {
                    c.color = color.into();
                    self.mark_dirty();
                }
            }
            MenuAction::NewCalendar => {
                let id = y7_sync::new_id();
                let color = calendar_core::COLORS[(self.store.calendars.len() + 3) % calendar_core::COLORS.len()];
                self.store.calendars.push(Calendar { id, name: "Untitled".into(), color: color.into() });
                self.mark_dirty();
                self.start_rename(id, cx);
            }
            MenuAction::RenameCalendar(id) => self.start_rename(id, cx),
            MenuAction::DeleteCalendar(id) => {
                if self.store.calendars.len() > 1 {
                    self.store.calendars.retain(|c| c.id != id);
                    self.store.events.retain(|e| e.calendar != id);
                    self.hidden.retain(|&h| h != id);
                    self.mark_dirty();
                    self.after_reload(cx);
                }
            }
        }
    }

    fn start_rename(&mut self, id: u64, cx: &mut Cx) {
        let Some(name) = self.store.calendar(id).map(|c| c.name.clone()) else { return };
        let mut r = self.fields.rename.borrow_mut();
        r.set_text(&mut cx.text.fonts, &name);
        r.select_all();
        drop(r);
        self.renaming = Some(id);
        cx.focus("rename");
    }

    fn finish_rename(&mut self, commit: bool, cx: &mut Cx) {
        let Some(id) = self.renaming.take() else { return };
        let name = self.fields.rename.borrow().text().trim().to_string();
        if commit && !name.is_empty() {
            if let Some(c) = self.store.calendars.iter_mut().find(|c| c.id == id) {
                c.name = name;
                self.mark_dirty();
            }
        }
        cx.blur();
    }

    // -- dragging on the hour grid ----------------------------------------------

    /// The day column and minute under `p` (clamped to the grid).
    fn grid_hit(&self, p: Point) -> Option<(NaiveDate, i32)> {
        let b = self.grid_bounds.get();
        let days = self.grid_days();
        if b.w <= 0.0 || days.is_empty() {
            return None;
        }
        let col = ((p.x - b.x) / (b.w / days.len() as f32)).floor().clamp(0.0, days.len() as f32 - 1.0) as usize;
        Some((days[col], y_to_minutes(p.y - b.y)))
    }

    /// A press on empty grid: maybe the start of sweeping out an event.
    fn grid_down(&mut self, cx: &mut Cx) {
        self.menu = None;
        if self.editor.is_some() {
            self.close_editor(cx);
            return;
        }
        self.selected = None;
        let Some((day, min)) = self.grid_hit(cx.mouse) else { return };
        let preview = layout::created(day, min, min);
        self.drag = Some(Drag { kind: DragKind::Create { day, anchor: min }, from: cx.mouse, moved: false, preview });
    }

    /// Double-click on empty grid: an hour-long event at that half hour.
    fn grid_double_click(&mut self, cx: &mut Cx) {
        self.drag = None;
        let Some((day, min)) = self.grid_hit(cx.mouse) else { return };
        let start = layout::snap_down(min, 30).min(DAY_MIN - 60);
        let anchor = self.block_anchor(day, start, start + 60);
        self.create_event(at(day, start), at(day, start + 60), false, anchor, cx);
    }

    /// A press on an event block in the grid: select it, and get ready to move
    /// it (or resize it, near its bottom edge).
    fn event_down(&mut self, event: u64, occ: (i64, i64), day: NaiveDate, cx: &mut Cx) {
        self.menu = None;
        if self.editor.as_ref().is_some_and(|e| e.event != event) {
            self.close_editor(cx);
        }
        self.selected = Some(event);
        let Some((_, min)) = self.grid_hit(cx.mouse) else { return };
        let b = self.grid_bounds.get();
        let bottom_min = minute_of(occ.1, day).min(DAY_MIN);
        let bottom_y = b.y + layout::minutes_to_y(bottom_min as f32);
        let ends_today = occ.1 <= day_start(day + Duration::days(1));
        let kind = if ends_today && cx.mouse.y > bottom_y - 6.0 {
            DragKind::Resize { event, start: occ.0, end: occ.1 }
        } else {
            DragKind::Move { event, start: occ.0, end: occ.1, grab: min - minute_of(occ.0, day) }
        };
        self.drag = Some(Drag { kind, from: cx.mouse, moved: false, preview: occ });
    }

    /// Where the drag would leave things with the pointer at `p`.
    fn drag_preview(&self, kind: DragKind, p: Point) -> Option<(i64, i64)> {
        let (day, min) = self.grid_hit(p)?;
        Some(match kind {
            DragKind::Create { day: d0, anchor } => {
                // Sweeping stays within the day it started on.
                let min = if day == d0 { min } else if day > d0 { DAY_MIN } else { 0 };
                layout::created(d0, anchor, min)
            }
            DragKind::Move { start, end, grab, .. } => layout::moved(start, end, grab, day, min),
            DragKind::Resize { start, end, .. } => {
                let start_day = local(start).date();
                let day = day.max(start_day);
                let _ = end;
                (start, layout::resized(start, day, min))
            }
        })
    }

    pub fn drag_to(&mut self, p: Point, cx: &mut Cx) {
        let Some(mut drag) = self.drag else { return };
        if !drag.moved && (p.x - drag.from.x).abs() < DRAG_SLOP && (p.y - drag.from.y).abs() < DRAG_SLOP {
            return;
        }
        drag.moved = true;
        if let Some(preview) = self.drag_preview(drag.kind, p) {
            drag.preview = preview;
        }
        self.drag = Some(drag);
        cx.request_redraw();
    }

    pub fn drop_drag(&mut self, cx: &mut Cx) {
        let Some(drag) = self.drag.take() else { return };
        if !drag.moved {
            return;
        }
        match drag.kind {
            DragKind::Create { day, .. } => {
                let (s, e) = drag.preview;
                let anchor = self.block_anchor(day, minute_of(s, day), minute_of(e, day));
                self.create_event(s, e, false, anchor, cx);
            }
            DragKind::Move { event, start, end, .. } | DragKind::Resize { event, start, end } => {
                self.reschedule(event, (start, end), drag.preview);
                self.save(cx);
            }
        }
        cx.request_redraw();
    }

    /// Window rect of a block from `s` to `e` minutes on `day` in the grid.
    fn block_anchor(&self, day: NaiveDate, s: i32, e: i32) -> Bounds {
        let b = self.grid_bounds.get();
        let days = self.grid_days();
        let col = days.iter().position(|d| *d == day).unwrap_or(0);
        let w = b.w / days.len().max(1) as f32;
        let y = b.y + layout::minutes_to_y(s as f32);
        Bounds::new(b.x + col as f32 * w, y, w - 8.0, layout::minutes_to_y((e - s).max(15) as f32))
    }

    // -- the popover -------------------------------------------------------------

    pub fn open_editor(&mut self, id: u64, start: i64, end: i64, anchor: Bounds, cx: &mut Cx) {
        self.menu = None;
        self.selected = Some(id);
        self.editor = Some(Editor { event: id, start, end, anchor });
        self.load_fields(cx);
        self.fields.title.borrow_mut().select_all();
        cx.focus("ed-title");
    }

    /// For previews: the popover open on today's "Design Review".
    pub fn preview_popover(&mut self, cx: &mut Cx) {
        let day = today();
        let Some(e) = self.store.events.iter().find(|e| e.title == "Design Review") else { return };
        let (id, s, end) = (e.id, e.start, e.end);
        self.go_to(day, cx);
        self.open_editor(id, s, end, Bounds::new(500.0, 300.0, 100.0, 60.0), cx);
        cx.blur();
    }

    /// Fills the popover's fields from the event.
    fn load_fields(&mut self, cx: &mut Cx) {
        let Some(ed) = &self.editor else { return };
        let Some(e) = self.store.event(ed.event).cloned() else { return };
        let (start, end) = (ed.start, ed.end);
        let f = &self.fields;
        let fonts = &mut cx.text.fonts;
        f.title.borrow_mut().set_text(fonts, &e.title);
        f.location.borrow_mut().set_text(fonts, &e.location);
        f.notes.borrow_mut().set_text(fonts, &e.notes);
        let (sd, st, ed_, et) = if e.all_day {
            let last = calendar_core::model::date_of((end - 1).max(start));
            (calendar_core::model::date_of(start).format("%-m/%-d/%Y").to_string(), String::new(), last.format("%-m/%-d/%Y").to_string(), String::new())
        } else {
            let (s, e) = (local(start), local(end));
            (s.format("%-m/%-d/%Y").to_string(), s.format("%-I:%M %p").to_string(), e.format("%-m/%-d/%Y").to_string(), e.format("%-I:%M %p").to_string())
        };
        f.start_date.borrow_mut().set_text(fonts, &sd);
        f.start_time.borrow_mut().set_text(fonts, &st);
        f.end_date.borrow_mut().set_text(fonts, &ed_);
        f.end_time.borrow_mut().set_text(fonts, &et);
    }

    pub fn close_editor(&mut self, cx: &mut Cx) {
        let Some(ed) = self.editor.take() else { return };
        self.menu = None;
        if let Some(e) = self.store.event_mut(ed.event) {
            if e.title.trim().is_empty() {
                e.title = "New Event".into();
                self.mark_dirty();
            }
        }
        cx.blur();
        self.save(cx);
    }

    /// A date or time field changed: apply it if it parses. A new start
    /// keeps the duration (moving the end along); a new end before the start
    /// is ignored.
    fn apply_times(&mut self, start_changed: bool, cx: &mut Cx) {
        let Some(ed) = &self.editor else { return };
        let Some(e) = self.store.event(ed.event) else { return };
        let f = &self.fields;
        let (date, time) = if start_changed { (&f.start_date, &f.start_time) } else { (&f.end_date, &f.end_time) };
        let Some(d) = parse_date(&date.borrow().text()) else { return };
        let (start, end) = if e.all_day {
            let key = date_key(d);
            if start_changed { (key, key + (ed.end - ed.start)) } else { (ed.start, date_key(d + Duration::days(1))) }
        } else {
            let Some(t) = parse_time(&time.borrow().text()) else { return };
            let ts = calendar_core::model::instant(d.and_time(t));
            if start_changed { (ts, ts + (ed.end - ed.start)) } else { (ed.start, ts) }
        };
        if end < start || (start, end) == (ed.start, ed.end) {
            return;
        }
        let (id, old) = (ed.event, (ed.start, ed.end));
        self.reschedule(id, old, (start, end));
        let ed = self.editor.as_mut().unwrap();
        (ed.start, ed.end) = (start, end);
        if start_changed {
            // Show where the end moved to.
            let all_day = self.store.event(id).is_some_and(|e| e.all_day);
            let (d, t) = if all_day {
                (calendar_core::model::date_of(end - 1).format("%-m/%-d/%Y").to_string(), String::new())
            } else {
                (local(end).format("%-m/%-d/%Y").to_string(), local(end).format("%-I:%M %p").to_string())
            };
            self.fields.end_date.borrow_mut().set_text(&mut cx.text.fonts, &d);
            self.fields.end_time.borrow_mut().set_text(&mut cx.text.fonts, &t);
        }
    }

    fn set_all_day(&mut self, all_day: bool, cx: &mut Cx) {
        let Some(ed) = self.editor.as_mut() else { return };
        let Some(e) = self.store.events.iter_mut().find(|e| e.id == ed.event) else { return };
        if e.all_day == all_day {
            return;
        }
        let to_days = |s: i64, end: i64| {
            let first = local(s).date();
            let last = local((end - 1).max(s)).date().max(first);
            (date_key(first), date_key(last + Duration::days(1)))
        };
        let to_times = |s: i64| {
            let d = calendar_core::model::date_of(s);
            (at(d, 9 * 60), at(d, 10 * 60))
        };
        let (es, ee) = if all_day { to_days(e.start, e.end) } else { to_times(e.start) };
        (e.start, e.end, e.all_day) = (es, ee, all_day);
        e.modified = y7_sync::now();
        (ed.start, ed.end) = if all_day { to_days(ed.start, ed.end) } else { to_times(ed.start) };
        self.mark_dirty();
        self.load_fields(cx);
    }

    // -- menus ---------------------------------------------------------------

    fn calendar_menu(&mut self, cx: &Cx) {
        let t = Self::theme(cx);
        let current = self.editor.as_ref().and_then(|ed| self.store.event(ed.event)).map(|e| e.calendar);
        let items = self
            .store
            .calendars
            .iter()
            .map(|c| MenuItem {
                label: c.name.clone(),
                color: Some(t.cal(&c.color)),
                checked: Some(c.id) == current,
                action: Some(MenuAction::SetCalendar(c.id)),
            })
            .collect();
        self.menu = Some(Menu { pos: cx.mouse, items });
    }

    fn repeat_menu(&mut self, cx: &Cx) {
        let current = self.editor.as_ref().and_then(|ed| self.store.event(ed.event)).and_then(|e| e.rule()).map(|r| r.freq);
        let item = |label: &str, f: Option<Freq>| MenuItem {
            label: label.into(),
            color: None,
            checked: current == f,
            action: Some(MenuAction::SetRepeat(f)),
        };
        let items = vec![
            item("None", None),
            MenuItem { label: String::new(), color: None, checked: false, action: None },
            item("Every Day", Some(Freq::Daily)),
            item("Every Week", Some(Freq::Weekly)),
            item("Every Month", Some(Freq::Monthly)),
            item("Every Year", Some(Freq::Yearly)),
        ];
        self.menu = Some(Menu { pos: cx.mouse, items });
    }

    fn sidebar_menu(&mut self, id: Option<u64>, cx: &Cx) {
        let t = Self::theme(cx);
        let mut items = vec![MenuItem { label: "New Calendar".into(), color: None, checked: false, action: Some(MenuAction::NewCalendar) }];
        if let Some(cal) = id.and_then(|id| self.store.calendar(id)) {
            items.push(MenuItem { label: String::new(), color: None, checked: false, action: None });
            for name in calendar_core::COLORS {
                let mut label = name.to_string();
                label[..1].make_ascii_uppercase();
                items.push(MenuItem {
                    label,
                    color: Some(t.cal(name)),
                    checked: cal.color == name,
                    action: Some(MenuAction::SetColor(cal.id, name)),
                });
            }
            items.push(MenuItem { label: String::new(), color: None, checked: false, action: None });
            items.push(MenuItem { label: "Rename".into(), color: None, checked: false, action: Some(MenuAction::RenameCalendar(cal.id)) });
            if self.store.calendars.len() > 1 {
                items.push(MenuItem { label: "Delete".into(), color: None, checked: false, action: Some(MenuAction::DeleteCalendar(cal.id)) });
            }
        }
        self.menu = Some(Menu { pos: cx.mouse, items });
    }

    // -- rendering -------------------------------------------------------------

    fn sync_input_styles(&self, t: &Theme) {
        let f = &self.fields;
        let style = |size: f32, weight: Weight, color: Color| TextStyle { size, line_height: 1.3, weight, color };
        let inputs = [
            (&f.title, style(15.0, Weight::Semibold, t.text)),
            (&f.location, style(12.0, Weight::Regular, t.text)),
            (&f.notes, style(12.0, Weight::Regular, t.text)),
            (&f.start_date, style(12.0, Weight::Regular, t.text)),
            (&f.start_time, style(12.0, Weight::Regular, t.text)),
            (&f.end_date, style(12.0, Weight::Regular, t.text)),
            (&f.end_time, style(12.0, Weight::Regular, t.text)),
            (&f.rename, style(13.0, Weight::Regular, t.text)),
        ];
        for (input, s) in inputs {
            let mut i = input.borrow_mut();
            i.set_styles(s, None);
            i.placeholder_color = t.text_tertiary;
            i.caret_color = t.accent;
            i.selection_color = t.text_selection;
        }
    }

    fn icon_button(
        id: &'static str,
        t: &Theme,
        icon: fn(&mut Scene, Bounds, Color),
        on_click: impl Fn(&mut Self, &mut Cx) + 'static,
    ) -> Div<Self> {
        let color = t.icon;
        div()
            .id(id)
            .w(32.0)
            .h(28.0)
            .rounded(6.0)
            .items_center()
            .justify_center()
            .hover_bg(t.icon_hover_bg)
            .on_click(on_click)
            .child(canvas(move |s, b| icon(s, b, color)).size(18.0))
    }

    fn toolbar(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut left = div().flex_1().flex_row().items_center().gap(4.0);
        if !self.sidebar_open {
            if let Some(buttons) = ui::chrome::window_buttons(true, cx, t.text) {
                left = left.child(buttons.mr(10.0));
            }
            left = left.child(Self::icon_button("show-sidebar", t, icons::sidebar, |s, _| s.sidebar_open = true));
        }
        left = left.child(Self::icon_button("add-event", t, icons::plus, |s: &mut Self, cx: &mut Cx| s.new_event(cx)));
        let labels: Vec<&str> = View::ALL.iter().map(|v| v.label()).collect();
        let seg = ui::widgets::segmented(cx, "view", &labels, self.view.index(), |s: &mut Self, i, cx| s.set_view(View::ALL[i], cx));
        let right = div().flex_1().flex_row().items_center().justify_end().children(ui::chrome::window_buttons(false, cx, t.text));
        div()
            .h(TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .px(12.0)
            .window_drag()
            .child(left)
            .child(seg)
            .child(right)
    }

    /// "November 2026" with Today and the arrows.
    fn title_row(&self, t: &Theme) -> Div<Self> {
        let (bold, rest, sub) = match self.view {
            View::Day => (self.date.format("%B %-d,").to_string(), self.date.format(" %Y").to_string(), Some(self.date.format("%A").to_string())),
            View::Week | View::Month => (self.date.format("%B").to_string(), self.date.format(" %Y").to_string(), None),
            View::Year => (self.date.format("%Y").to_string(), String::new(), None),
        };
        let title = div()
            .flex_col()
            .flex_1()
            .child(
                div()
                    .flex_row()
                    .items_end()
                    .child(text(bold).size(24.0).weight(Weight::Bold))
                    .child(text(rest).size(24.0).color(t.text)),
            )
            .child_if(sub.is_some(), || text(sub.clone().unwrap_or_default()).size(13.0).color(t.text_secondary).into());
        let arrow = |id: &'static str, icon: fn(&mut Scene, Bounds, Color), dir: i32| {
            let c = t.icon;
            div()
                .id(id)
                .size(24.0)
                .rounded(6.0)
                .items_center()
                .justify_center()
                .hover_bg(t.icon_hover_bg)
                .on_click(move |s: &mut Self, cx: &mut Cx| s.step(dir, cx))
                .child(canvas(move |s, b| icon(s, b, c)).size(14.0))
        };
        let nav = div()
            .flex_row()
            .items_center()
            .gap(2.0)
            .child(arrow("prev", icons::chevron_left, -1))
            .child(
                div()
                    .id("today")
                    .h(22.0)
                    .px(10.0)
                    .rounded(6.0)
                    .items_center()
                    .bg(t.button_bg)
                    .hover_bg(t.icon_hover_bg)
                    .on_click(|s: &mut Self, cx: &mut Cx| s.go_today(cx))
                    .child(text("Today").size(12.5)),
            )
            .child(arrow("next", icons::chevron_right, 1));
        div().h(if sub.is_some() { 62.0 } else { 50.0 }).shrink_0().flex_row().items_center().px(20.0).child(title).child(nav)
    }

    fn content(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let body = match self.view {
            View::Day | View::Week => self.grid_view(cx, t),
            View::Month => self.month_view(cx, t),
            View::Year => self.year_view(cx, t),
        };
        div().flex_1().h_full().flex_col().bg(t.content).child(self.toolbar(cx, t)).child(self.title_row(t)).child(body)
    }

    fn content_width(&self, cx: &Cx) -> f32 {
        cx.window_size.0 - if self.sidebar_open { SIDEBAR_W + 1.0 } else { 0.0 }
    }

    fn menu_overlay(&self, menu: &Menu, cx: &Cx, t: &Theme) -> Div<Self> {
        let (w, h) = cx.window_size;
        let width = 200.0;
        let est_h: f32 = 10.0 + menu.items.iter().map(|i| if i.action.is_some() { 24.0 } else { 9.0 }).sum::<f32>();
        let x = menu.pos.x.min(w - width - 6.0).max(6.0);
        let y = if menu.pos.y + est_h > h - 6.0 { (menu.pos.y - est_h).max(6.0) } else { menu.pos.y };
        let mut panel = div()
            .absolute(x, y)
            .w(width)
            .flex_col()
            .p(5.0)
            .rounded(8.0)
            .bg(t.menu_bg)
            .border(t.menu_border, 0.5)
            .shadow(t.shadow, 14.0, 6.0)
            .occlude();
        for (i, item) in menu.items.iter().enumerate() {
            let Some(action) = &item.action else {
                panel = panel.child(div().h(1.0).mt(4.0).mb(4.0).px(6.0).child(div().flex_1().h(1.0).bg(t.menu_border)));
                continue;
            };
            let hovered = cx.hovered(("menu-item", i));
            let ink = if hovered { Color::WHITE } else { t.text };
            let action = action.clone();
            let mut row = div()
                .id(("menu-item", i))
                .h(24.0)
                .px(6.0)
                .rounded(5.0)
                .flex_row()
                .items_center()
                .gap(6.0)
                .hover_bg(t.menu_highlight)
                .on_click(move |s: &mut Self, cx: &mut Cx| s.run_menu(action.clone(), cx));
            row = row.child(div().w(12.0).child_if(item.checked, || canvas(move |s, b| icons::check(s, b, ink, 1.6)).size(12.0).into()));
            if let Some(c) = item.color {
                row = row.child(div().size(10.0).rounded(5.0).bg(c));
            }
            panel = panel.child(row.child(text(item.label.clone()).color(ink).truncate()));
        }
        div()
            .inset_0()
            .deferred()
            .on_mouse_down(|s: &mut Self, _| s.menu = None)
            .on_right_click(|s: &mut Self, _| s.menu = None)
            .child(panel)
    }
}

/// "11/14/2026", "11/14/26", "2026-11-14".
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    ["%m/%d/%Y", "%m/%d/%y", "%Y-%m-%d"].iter().find_map(|f| NaiveDate::parse_from_str(s, f).ok()).filter(|d| d.year() > 1900)
}

/// "9:30 AM", "9:30am", "9 PM", "21:30".
pub fn parse_time(s: &str) -> Option<chrono::NaiveTime> {
    let s = s.trim().to_uppercase().replace('.', "");
    let s = if s.ends_with('A') || s.ends_with('P') { format!("{s}M") } else { s };
    // A bare hour ("9 PM") needs minutes to be a time.
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    let s = if !s.contains(':') && (1..=2).contains(&digits) { format!("{}:00{}", &s[..digits], &s[digits..]) } else { s };
    ["%I:%M %p", "%I:%M%p", "%H:%M", "%H%M"]
        .iter()
        .find_map(|f| chrono::NaiveTime::parse_from_str(&s, f).ok())
}

impl PairingHost for CalendarApp {
    fn pairing(&mut self) -> &mut Pairing {
        &mut self.pairing
    }
}

impl App for CalendarApp {
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Self::theme(cx).text }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        let t = Self::theme(cx);
        self.sync_input_styles(&t);
        let (w, h) = cx.window_size;
        let mut root = div().w(w).h(h).flex_row().bg(t.content).text_color(t.text);
        if self.sidebar_open {
            root = root.child(self.sidebar(cx, &t)).child(div().w(1.0).h_full().shrink_0().bg(t.divider));
        }
        root = root.child(self.content(cx, &t));
        if let Some(popover) = self.popover(cx, &t) {
            root = root.child(popover);
        }
        if let Some(menu) = &self.menu {
            root = root.child(self.menu_overlay(menu, cx, &t));
        }
        if let Some(sheet) = self.pairing.sheet(cx, &self.sync_colors(&t)) {
            root = root.child(sheet);
        }
        if cx.transparent && !cx.maximized && !ui::is_y7ya() {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(t.window_border, 1.0));
        }
        root
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if self.pairing.on_key(key) {
            return true;
        }
        if key.is_named(NamedKey::Escape) {
            if self.menu.take().is_some() {
            } else if self.renaming.is_some() {
                self.finish_rename(false, cx);
            } else if self.editor.is_some() {
                self.close_editor(cx);
            } else {
                self.selected = None;
            }
            return true;
        }
        if self.menu.is_some() {
            return true;
        }
        let editing = cx.focused.is_some();
        if key.is_named(NamedKey::Enter) {
            if self.renaming.is_some() {
                self.finish_rename(true, cx);
                return true;
            }
            if self.editor.is_some() && !cx.is_focused("ed-notes") {
                self.close_editor(cx);
                return true;
            }
            if let Some(id) = self.selected.filter(|_| !editing) {
                if let Some(e) = self.store.event(id) {
                    let (s, e2) = (e.start, e.end);
                    let (w, h) = cx.window_size;
                    self.open_editor(id, s, e2, Bounds::new(w / 2.0 - 40.0, h / 3.0, 80.0, 40.0), cx);
                }
                return true;
            }
        }
        if key.ctrl && !key.alt {
            let view = ['1', '2', '3', '4'].iter().position(|c| key.is_char(*c));
            if let Some(i) = view {
                self.set_view(View::ALL[i], cx);
                return true;
            }
            if key.is_char('n') {
                self.new_event(cx);
                return true;
            }
            if key.is_char('t') {
                self.go_today(cx);
                return true;
            }
            if key.is_char('q') || key.is_char('w') {
                self.close_editor(cx);
                self.save(cx);
                cx.close_window();
                return true;
            }
            if key.is_char('s') {
                self.save(cx);
                return true;
            }
        }
        if !editing {
            if key.is_named(NamedKey::ArrowLeft) {
                self.step(-1, cx);
                return true;
            }
            if key.is_named(NamedKey::ArrowRight) {
                self.step(1, cx);
                return true;
            }
            if key.is_named(NamedKey::Delete) || key.is_named(NamedKey::Backspace) {
                if let Some(id) = self.selected {
                    self.delete_event(id, cx);
                }
                return true;
            }
        }
        if self.renaming.is_some() && !cx.is_focused("rename") {
            self.finish_rename(true, cx);
        }
        false
    }

    fn on_input(&mut self, id: ElementId, cx: &mut Cx) {
        let Some(ed) = &self.editor else { return };
        let event = ed.event;
        let f = &self.fields;
        let text_field = [(eid("ed-title"), &f.title), (eid("ed-location"), &f.location), (eid("ed-notes"), &f.notes)];
        if let Some((_, input)) = text_field.iter().find(|(i, _)| *i == id) {
            let value = input.borrow().text();
            if let Some(e) = self.store.event_mut(event) {
                match id {
                    i if i == eid("ed-title") => e.title = value,
                    i if i == eid("ed-location") => e.location = value,
                    _ => e.notes = value,
                }
                e.modified = y7_sync::now();
                self.mark_dirty();
            }
        } else if id == eid("ed-sdate") || id == eid("ed-stime") {
            self.apply_times(true, cx);
        } else if id == eid("ed-edate") || id == eid("ed-etime") {
            self.apply_times(false, cx);
        }
    }

    fn on_drag(&mut self, cx: &mut Cx) {
        let p = cx.mouse;
        self.drag_to(p, cx);
    }

    fn on_mouse_up(&mut self, cx: &mut Cx) {
        self.drop_drag(cx);
    }

    fn next_wake(&self) -> Option<Instant> {
        let save = self.dirty_since.map(|t| t + SAVE_DELAY);
        // The current-time line moves once a minute (and "today" at midnight).
        let secs = 60 - (y7_sync::now().rem_euclid(60)) as u64;
        let minute = Instant::now() + StdDuration::from_secs(secs);
        save.into_iter().chain(self.pairing.next_wake()).chain([minute]).min()
    }

    fn on_wake(&mut self, cx: &mut Cx) {
        if self.dirty_since.is_some_and(|t| t.elapsed() >= SAVE_DELAY) && self.drag.is_none() {
            self.save(cx);
        }
    }

    fn on_external(&mut self, cx: &mut Cx) {
        if self.events.as_ref().is_some_and(|e| e.take_changed(calendar_core::DOC)) && self.drag.is_none() {
            // Merge any unsaved edits first, then show the combined result.
            self.dirty_since = None;
            self.store.save();
            self.after_reload(cx);
        }
    }

    fn on_exit(&mut self) {
        if let Some(e) = self.editor.take().and_then(|ed| self.store.event_mut(ed.event)) {
            if e.title.trim().is_empty() {
                e.title = "New Event".into();
            }
        }
        self.store.save();
        if let Some(core) = &self.store.core {
            core.sync().shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(app: &mut CalendarApp, cx: &mut Cx, field: &str, s: &str) {
        let input = match field {
            "ed-title" => app.fields.title.clone(),
            "ed-stime" => app.fields.start_time.clone(),
            _ => app.fields.location.clone(),
        };
        input.borrow_mut().select_all();
        for ch in s.chars() {
            let kp = KeyPress { key: Key::Character(ch.to_string().into()), text: Some(ch.to_string()), ctrl: false, shift: false, alt: false };
            let out = input.borrow_mut().handle_key(&mut cx.text.fonts, &kp, &mut None);
            if out.changed {
                app.on_input(eid(field), cx);
            }
        }
    }

    #[test]
    fn parses_typed_dates_and_times() {
        assert_eq!(parse_date("11/14/2026"), NaiveDate::from_ymd_opt(2026, 11, 14));
        assert_eq!(parse_date("2026-11-14"), NaiveDate::from_ymd_opt(2026, 11, 14));
        assert_eq!(parse_date("11/14"), None);
        let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 0);
        assert_eq!(parse_time("9:30 AM"), t(9, 30));
        assert_eq!(parse_time("9:30pm"), t(21, 30));
        assert_eq!(parse_time("9 p"), t(21, 0));
        assert_eq!(parse_time("21:05"), t(21, 5));
        assert_eq!(parse_time("lunch"), None);
    }

    #[test]
    fn create_edit_move_and_delete_events() {
        let path = std::env::temp_dir().join("calendar-selftest.png");
        let day = today();
        ui::snapshot((1280.0, 800.0), 2.0, false, &path, |cx| CalendarApp::new(model::demo_store(day), None, cx), |app, cx| {
            let before = app.store.events.len();
            // A new event opens the popover with its title selected.
            let id = app.create_event(at(day, 600), at(day, 660), false, Bounds::new(400.0, 300.0, 100.0, 48.0), cx);
            assert_eq!(app.store.events.len(), before + 1);
            assert!(cx.is_focused("ed-title"));
            type_text(app, cx, "ed-title", "Call the bank");
            type_text(app, cx, "ed-location", "Phone");
            assert_eq!(app.store.event(id).unwrap().title, "Call the bank");
            assert_eq!(app.store.event(id).unwrap().location, "Phone");

            // Typing a new start time keeps the duration; a new end changes it.
            type_text(app, cx, "ed-stime", "2:15 PM");
            let e = app.store.event(id).unwrap();
            assert_eq!((e.start, e.end), (at(day, 14 * 60 + 15), at(day, 15 * 60 + 15)));
            assert_eq!(app.fields.end_time.borrow().text(), "3:15 PM");
            app.fields.end_time.borrow_mut().set_text(&mut cx.text.fonts, "4:00 PM");
            app.on_input(eid("ed-etime"), cx);
            assert_eq!(app.store.event(id).unwrap().end, at(day, 16 * 60));
            app.fields.end_time.borrow_mut().set_text(&mut cx.text.fonts, "1:00 PM");
            app.on_input(eid("ed-etime"), cx);
            assert_eq!(app.store.event(id).unwrap().end, at(day, 16 * 60), "an end before the start is ignored");

            // Repeat and calendar from the menus.
            app.run_menu(MenuAction::SetRepeat(Some(Freq::Weekly)), cx);
            app.run_menu(MenuAction::SetCalendar(900), cx);
            let e = app.store.event(id).unwrap();
            assert_eq!((e.rrule.as_str(), e.calendar), ("FREQ=WEEKLY", 900));
            app.close_editor(cx);
            assert!(app.editor.is_none());

            // Moving a later occurrence moves the whole series.
            let next = (at(day + Duration::days(7), 855), at(day + Duration::days(7), 960));
            app.reschedule(id, next, (next.0 + 3600, next.1 + 3600));
            assert_eq!(app.store.event(id).unwrap().start, at(day, 15 * 60 + 15));

            // All-day and back.
            app.open_editor(id, at(day, 915), at(day, 975), Bounds::new(400.0, 300.0, 100.0, 48.0), cx);
            app.set_all_day(true, cx);
            let e = app.store.event(id).unwrap();
            assert!(e.all_day && e.start == date_key(day) && e.end == date_key(day + Duration::days(1)));
            app.set_all_day(false, cx);
            assert_eq!(app.store.event(id).unwrap().start, at(day, 9 * 60));

            app.delete_event(id, cx);
            assert!(app.store.event(id).is_none() && app.editor.is_none());
            assert_eq!(app.store.events.len(), before);

            // Navigation.
            app.set_view(View::Month, cx);
            app.step(1, cx);
            assert_eq!(app.date, calendar_core::dates::add_months(day, 1));
            app.go_today(cx);
            assert_eq!(app.date, day);
            app.set_view(View::Week, cx);
        });
        assert!(path.exists());
    }

    #[test]
    fn dragging_on_the_grid() {
        let path = std::env::temp_dir().join("calendar-selftest-drag.png");
        let day = today();
        ui::snapshot((1280.0, 800.0), 2.0, false, &path, |cx| CalendarApp::new(model::demo_store(day), None, cx), |app, cx| {
            app.set_view(View::Day, cx);
            app.go_to(day, cx);
            // As if painted: the day column spans x 300..1200, 9 AM at y 100.
            let top = 100.0 - layout::minutes_to_y(540.0);
            app.grid_bounds.set(Bounds::new(300.0, top, 900.0, layout::HOUR_H * 24.0));
            let y = |min: f32| top + layout::minutes_to_y(min);

            // Sweep from 9:05 to 10:20: a 9:00-10:30 event.
            let before = app.store.events.len();
            cx.mouse = pt(500.0, y(545.0));
            app.grid_down(cx);
            app.drag_to(pt(500.0, y(620.0)), cx);
            app.drop_drag(cx);
            assert_eq!(app.store.events.len(), before + 1);
            let id = app.selected.unwrap();
            let e = app.store.event(id).unwrap();
            assert_eq!((e.start, e.end), (at(day, 540), at(day, 630)));
            app.close_editor(cx);

            // Grab it 30 min in and drag down by an hour.
            cx.mouse = pt(500.0, y(570.0));
            app.event_down(id, (at(day, 540), at(day, 630)), day, cx);
            app.drag_to(pt(500.0, y(632.0)), cx);
            app.drop_drag(cx);
            let e = app.store.event(id).unwrap();
            assert_eq!((e.start, e.end), (at(day, 600), at(day, 690)));

            // Pull its bottom edge to 12:00.
            cx.mouse = pt(500.0, y(689.0));
            app.event_down(id, (at(day, 600), at(day, 690)), day, cx);
            app.drag_to(pt(500.0, y(721.0)), cx);
            app.drop_drag(cx);
            assert_eq!(app.store.event(id).unwrap().end, at(day, 720));

            // A click without moving only selects.
            cx.mouse = pt(500.0, y(650.0));
            app.event_down(id, (at(day, 600), at(day, 720)), day, cx);
            app.drop_drag(cx);
            assert_eq!(app.store.event(id).unwrap().start, at(day, 600));
            assert_eq!(app.selected, Some(id));
        });
    }
}
