mod icons;
pub mod model;
mod theme;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Duration as Days;
use model::{DEFAULT_LIST, Due, List, Priority, Reminder, Section, Smart, Store, View, at, due_text, long_date, minutes_of, now, sections, time_text, today};
use reminders_core::{COLORS, DOC, color_rgb};
use sync_ui::{Pairing, PairingHost, SyncEvents};
use theme::Theme;
use ui::widgets::{Menu, MenuItem, MenuKey};
use ui::*;

/// Wider than the shared sidebar, as in Reminders, to fit two smart-list
/// tiles side by side.
const SIDEBAR_W: f32 = 240.0;
/// My Lists' colored circles (bigger than a plain sidebar icon).
const LIST_ICON: f32 = 20.0;
const SAVE_DELAY: Duration = Duration::from_millis(600);
/// Subtasks are indented by the width of a completion circle and its gap.
const INDENT: f32 = 30.0;
const POPOVER_W: f32 = 320.0;
const POPOVER_H: f32 = 396.0;

#[derive(Clone, Debug, PartialEq)]
enum MenuAction {
    NewReminder,
    NewList,
    RenameList(u64),
    DeleteList(u64),
    ListColor(u64, &'static str),
    Flag(u64, bool),
    Priority(u64, Priority),
    MoveTo(u64, u64),
    Indent(u64),
    Outdent(u64),
    ShowInfo(u64),
    Delete(u64),
}

/// Which line of the reminder being edited has the caret.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Title,
    Notes,
}

pub struct RemindersApp {
    store: Store,
    view: View,
    selected: Option<u64>,
    /// The reminder whose title and notes are live text fields.
    editing: Option<u64>,
    title: Rc<RefCell<TextInput>>,
    notes: Rc<RefCell<TextInput>>,
    search: Rc<RefCell<TextInput>>,
    query: String,
    rename: Rc<RefCell<TextInput>>,
    renaming: Option<u64>,
    /// The reminder whose details popover is open.
    details: Option<u64>,
    details_notes: Rc<RefCell<TextInput>>,
    show_completed: bool,
    /// Checked off since this view opened: they stay until the user moves on.
    keep: HashSet<u64>,
    menu: Option<Menu<MenuAction>>,
    dirty_since: Option<Instant>,
    events: Option<Arc<SyncEvents>>,
    pairing: Pairing,
    /// Where each row was drawn last frame (to anchor the details popover).
    row_bounds: Rc<RefCell<HashMap<u64, Bounds>>>,
}

impl RemindersApp {
    pub fn new(store: Store, events: Option<Arc<SyncEvents>>, cx: &mut Cx) -> Self {
        if let Some(events) = &events {
            events.set_waker(cx.waker());
        }
        let fonts = &mut cx.text.fonts;
        let mut title = TextInput::new(fonts, TextStyle::default(), true);
        title.placeholder = "New Reminder".into();
        let mut notes = TextInput::new(fonts, TextStyle::default(), false);
        notes.placeholder = "Notes".into();
        let mut search = TextInput::new(fonts, TextStyle::default(), true);
        search.placeholder = "Search".into();
        let rename = TextInput::new(fonts, TextStyle::default(), true);
        let mut details_notes = TextInput::new(fonts, TextStyle::default(), false);
        details_notes.placeholder = "Add Note".into();
        cx.window_radius = 10.0;
        let hub = store.core.as_ref().map(|c| c.hub());
        let first = store.lists.first().map_or(DEFAULT_LIST, |l| l.id);
        cx.focus("list");
        Self {
            store,
            view: View::List(first),
            selected: None,
            editing: None,
            title: Rc::new(RefCell::new(title)),
            notes: Rc::new(RefCell::new(notes)),
            search: Rc::new(RefCell::new(search)),
            query: String::new(),
            rename: Rc::new(RefCell::new(rename)),
            renaming: None,
            details: None,
            details_notes: Rc::new(RefCell::new(details_notes)),
            show_completed: false,
            keep: HashSet::new(),
            menu: None,
            dirty_since: None,
            events,
            pairing: Pairing::new(hub),
            row_bounds: Rc::default(),
        }
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

    /// The store picked up edits from elsewhere: bring the UI in line.
    fn after_reload(&mut self, cx: &mut Cx) {
        match self.editing.and_then(|id| self.store.get(id)) {
            Some(r) => {
                let (title, notes) = (r.title.clone(), r.notes.clone());
                if self.title.borrow().text() != title {
                    self.title.borrow_mut().replace_text(&mut cx.text.fonts, &title);
                }
                if self.notes.borrow().text() != notes {
                    self.notes.borrow_mut().replace_text(&mut cx.text.fonts, &notes);
                }
            }
            None => self.editing = None,
        }
        match self.details.and_then(|id| self.store.get(id)).map(|r| r.notes.clone()) {
            Some(notes) if self.details_notes.borrow().text() != notes => {
                self.details_notes.borrow_mut().replace_text(&mut cx.text.fonts, &notes);
            }
            Some(_) => {}
            None => self.details = None,
        }
        if self.selected.is_some_and(|id| self.store.get(id).is_none()) {
            self.selected = None;
        }
        if self.renaming.is_some_and(|id| self.store.list(id).is_none()) {
            self.renaming = None;
        }
        if let View::List(l) = self.view
            && self.store.list(l).is_none()
        {
            self.view = View::List(self.store.lists.first().map_or(DEFAULT_LIST, |l| l.id));
        }
    }

    pub fn open_sync(&mut self) {
        self.menu = None;
        self.pairing.open();
    }

    // -- previews -----------------------------------------------------------

    pub fn search_for(&mut self, q: &str, cx: &mut Cx) {
        self.search.borrow_mut().set_text(&mut cx.text.fonts, q);
        self.query = q.to_string();
        cx.focus("search");
    }

    /// Opens the details of the first reminder in view that has notes.
    pub fn show_details_preview(&mut self, cx: &mut Cx) {
        let ids = self.visible_ids();
        let id = ids.iter().copied().find(|&id| self.store.get(id).is_some_and(|r| !r.notes.is_empty())).or(ids.first().copied());
        if let Some(id) = id {
            self.open_details(id, cx);
        }
    }

    /// Opens the second reminder's context menu (or the current list's),
    /// with the pointer over a row.
    pub fn menu_preview(&mut self, list: bool, cx: &mut Cx) {
        if list {
            let id = match self.view {
                View::List(l) => l,
                View::Smart(_) => self.target_list(),
            };
            cx.mouse = Point { x: 150.0, y: 350.0 };
            self.list_menu(id, cx);
        } else if let Some(&id) = self.visible_ids().get(1) {
            cx.mouse = Point { x: 420.0, y: 190.0 };
            self.reminder_menu(id, cx);
        }
        cx.mouse.x += 30.0;
        cx.mouse.y += 5.0 + 24.0 * 3.0 + 11.0 + 12.0;
    }

    /// Edits the second reminder in view.
    pub fn edit_preview(&mut self, cx: &mut Cx) {
        if let Some(&id) = self.visible_ids().get(1) {
            self.start_edit(id, Field::Title, cx);
        }
    }

    // -- queries ------------------------------------------------------------

    fn searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    fn sections(&self) -> Vec<Section> {
        sections(&self.store, self.view, self.show_completed, &self.keep, &self.query, today())
    }

    fn visible_ids(&self) -> Vec<u64> {
        self.sections().iter().flat_map(|s| s.rows.iter().map(|r| r.id)).collect()
    }

    fn view_name(&self) -> String {
        if self.searching() {
            return format!("Results for \u{201c}{}\u{201d}", self.query.trim());
        }
        match self.view {
            View::Smart(s) => s.name().to_string(),
            View::List(l) => self.store.list(l).map_or("Reminders".into(), |l| l.name.clone()),
        }
    }

    fn view_color(&self, t: &Theme) -> Color {
        if self.searching() {
            return t.text;
        }
        match self.view {
            View::Smart(s) => rgb(s.color(t.dark)),
            View::List(l) => rgb(self.store.list_color(l)),
        }
    }

    // -- selection and editing ----------------------------------------------

    pub fn set_view(&mut self, view: View, cx: &mut Cx) {
        self.menu = None;
        self.details = None;
        self.finish_edit(cx);
        if self.searching() {
            self.query.clear();
            self.search.borrow_mut().set_text(&mut cx.text.fonts, "");
        }
        if self.view != view {
            self.keep.clear();
            self.selected = None;
        }
        self.view = view;
        cx.scroll_to_top("content-scroll");
        cx.focus("list");
    }

    fn select(&mut self, id: Option<u64>) {
        self.selected = id;
    }

    /// Makes `id`'s title (or notes) editable, with the caret at the end.
    fn start_edit(&mut self, id: u64, field: Field, cx: &mut Cx) {
        if self.editing != Some(id) {
            self.finish_edit(cx);
        }
        let Some(r) = self.store.get(id) else { return };
        let (title, notes) = (r.title.clone(), r.notes.clone());
        if self.editing != Some(id) {
            self.title.borrow_mut().set_text(&mut cx.text.fonts, &title);
            self.notes.borrow_mut().set_text(&mut cx.text.fonts, &notes);
        }
        self.editing = Some(id);
        self.selected = Some(id);
        let input = if field == Field::Title { &self.title } else { &self.notes };
        input.borrow_mut().move_to_end();
        cx.focus(if field == Field::Title { "title" } else { "notes" });
    }

    /// Ends editing. Like Reminders, a reminder left without a title (and
    /// without notes or subtasks) disappears.
    fn finish_edit(&mut self, cx: &mut Cx) {
        let Some(id) = self.editing.take() else { return };
        let blank = self
            .store
            .get(id)
            .is_some_and(|r| r.title.trim().is_empty() && r.notes.trim().is_empty() && self.store.children(id).next().is_none());
        if blank {
            self.store.reminders.retain(|r| r.id != id);
            if self.selected == Some(id) {
                self.selected = None;
            }
            self.mark_dirty();
        }
        if cx.is_focused("title") || cx.is_focused("notes") {
            cx.focus("list");
        }
    }

    fn move_selection(&mut self, delta: i32, cx: &mut Cx) {
        let ids = self.visible_ids();
        if ids.is_empty() {
            return;
        }
        let cur = self.editing.or(self.selected);
        let i = cur.and_then(|s| ids.iter().position(|&x| x == s)).map_or(0, |i| i as i32 + delta);
        let id = ids[i.clamp(0, ids.len() as i32 - 1) as usize];
        if self.editing.is_some() {
            self.start_edit(id, Field::Title, cx);
        } else {
            self.select(Some(id));
        }
    }

    // -- reminder actions ---------------------------------------------------

    /// The list new reminders go to in the current view.
    fn target_list(&self) -> u64 {
        match self.view {
            View::List(l) if !self.searching() => l,
            _ => self.store.list(DEFAULT_LIST).or(self.store.lists.first()).map_or(DEFAULT_LIST, |l| l.id),
        }
    }

    /// Adds a reminder (right after `after` when given, else at the end of
    /// the view's list) and starts editing its title.
    pub fn new_reminder(&mut self, after: Option<u64>, cx: &mut Cx) {
        self.menu = None;
        self.details = None;
        if self.view == View::Smart(Smart::Completed) && after.is_none() {
            return;
        }
        let after = after.and_then(|id| self.store.get(id)).cloned();
        let mut r = Reminder::new(after.as_ref().map_or_else(|| self.target_list(), |a| a.list), "");
        match &after {
            Some(a) => {
                r.created = self.store.created_after(a);
                r.parent = a.parent;
                if self.view == View::Smart(Smart::Today) || self.view == View::Smart(Smart::Scheduled) {
                    r.due = a.due;
                }
                r.flagged = self.view == View::Smart(Smart::Flagged);
            }
            None => match self.view {
                View::Smart(Smart::Today | Smart::Scheduled) => r.due = Some(Due::date(today())),
                View::Smart(Smart::Flagged) => r.flagged = true,
                _ => {}
            },
        }
        if self.searching() {
            self.query.clear();
            self.search.borrow_mut().set_text(&mut cx.text.fonts, "");
        }
        let id = r.id;
        self.finish_edit(cx);
        self.store.reminders.push(r);
        self.mark_dirty();
        self.start_edit(id, Field::Title, cx);
    }

    fn toggle_done(&mut self, id: u64) {
        let Some(r) = self.store.get_mut(id) else { return };
        r.completed = if r.is_done() { None } else { Some(now()) };
        r.modified = now();
        self.keep.insert(id);
        self.mark_dirty();
    }

    /// Makes `id` a subtask of the reminder above it (one level deep).
    pub fn indent(&mut self, id: u64) {
        let Some(r) = self.store.get(id) else { return };
        if r.parent.is_some() || self.store.children(id).next().is_some() {
            return;
        }
        let ids = self.visible_ids();
        let Some(pos) = ids.iter().position(|&x| x == id) else { return };
        let list = r.list;
        let parent = ids[..pos]
            .iter()
            .rev()
            .filter_map(|&x| self.store.get(x))
            .find(|p| p.list == list)
            .map(|p| p.parent.unwrap_or(p.id));
        if let (Some(parent), Some(r)) = (parent, self.store.get_mut(id)) {
            r.parent = Some(parent);
            r.modified = now();
            self.mark_dirty();
        }
    }

    /// Makes subtask `id` a reminder of its own, right after its parent.
    pub fn outdent(&mut self, id: u64) {
        let Some(parent) = self.store.get(id).and_then(|r| r.parent).and_then(|p| self.store.get(p)).cloned() else { return };
        let created = self.store.created_after(&parent);
        if let Some(r) = self.store.get_mut(id) {
            r.parent = None;
            r.created = created;
            r.modified = now();
            self.mark_dirty();
        }
    }

    fn delete_reminder(&mut self, id: u64, cx: &mut Cx) {
        if self.editing == Some(id) {
            self.editing = None;
            cx.focus("list");
        }
        if self.details == Some(id) {
            self.details = None;
        }
        let ids = self.visible_ids();
        let next = ids.iter().position(|&x| x == id).and_then(|i| ids.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| ids.get(j))));
        let next = next.copied().filter(|&n| n != id && self.store.get(n).is_some_and(|r| r.parent != Some(id)));
        self.store.reminders.retain(|r| r.id != id && r.parent != Some(id));
        if self.selected == Some(id) {
            self.selected = next;
        }
        self.mark_dirty();
    }

    /// Changes reminder `id` in place.
    fn update(&mut self, id: u64, f: impl FnOnce(&mut Reminder)) {
        if let Some(r) = self.store.get_mut(id) {
            f(r);
            r.modified = now();
            self.mark_dirty();
        }
    }

    pub fn open_details(&mut self, id: u64, cx: &mut Cx) {
        self.menu = None;
        if self.editing == Some(id) {
            // Keep a fresh reminder that only has a title so far.
            self.editing = None;
        } else {
            self.finish_edit(cx);
        }
        let Some(r) = self.store.get(id) else { return };
        let notes = r.notes.clone();
        self.details_notes.borrow_mut().set_text(&mut cx.text.fonts, &notes);
        self.details = Some(id);
        self.selected = Some(id);
        cx.focus("list");
    }

    // -- list actions -------------------------------------------------------

    pub fn new_list(&mut self, cx: &mut Cx) {
        self.menu = None;
        let id = reminders_core::new_id();
        let name = self.store.unique_list_name("New List");
        let color = COLORS[(self.store.lists.len() + 4) % COLORS.len()].to_string();
        self.store.lists.push(List { id, name, color });
        self.mark_dirty();
        self.set_view(View::List(id), cx);
        self.start_rename(id, cx);
    }

    fn start_rename(&mut self, id: u64, cx: &mut Cx) {
        self.menu = None;
        let Some(name) = self.store.list(id).map(|l| l.name.clone()) else { return };
        let mut r = self.rename.borrow_mut();
        r.set_text(&mut cx.text.fonts, &name);
        r.select_all();
        drop(r);
        self.renaming = Some(id);
        cx.focus("rename");
    }

    fn finish_rename(&mut self, commit: bool, cx: &mut Cx) {
        let Some(id) = self.renaming.take() else { return };
        let name = self.rename.borrow().text().trim().to_string();
        if commit && !name.is_empty() {
            if let Some(l) = self.store.lists.iter_mut().find(|l| l.id == id) {
                l.name = name;
                self.mark_dirty();
            }
        }
        cx.focus("list");
    }

    fn delete_list(&mut self, id: u64, cx: &mut Cx) {
        if self.store.lists.len() <= 1 {
            return;
        }
        self.store.lists.retain(|l| l.id != id);
        self.store.reminders.retain(|r| r.list != id);
        self.mark_dirty();
        if self.view == View::List(id) {
            let first = self.store.lists[0].id;
            self.set_view(View::List(first), cx);
        }
        self.after_reload(cx);
    }

    fn run_menu(&mut self, action: MenuAction, cx: &mut Cx) {
        self.menu = None;
        match action {
            MenuAction::NewReminder => self.new_reminder(None, cx),
            MenuAction::NewList => self.new_list(cx),
            MenuAction::RenameList(id) => self.start_rename(id, cx),
            MenuAction::DeleteList(id) => self.delete_list(id, cx),
            MenuAction::ListColor(id, color) => {
                if let Some(l) = self.store.lists.iter_mut().find(|l| l.id == id) {
                    l.color = color.to_string();
                    self.mark_dirty();
                }
            }
            MenuAction::Flag(id, on) => self.update(id, |r| r.flagged = on),
            MenuAction::Priority(id, p) => self.update(id, |r| r.priority = p),
            MenuAction::MoveTo(id, list) => {
                self.update(id, |r| {
                    r.list = list;
                    r.parent = None;
                });
                for kid in self.store.reminders.iter_mut().filter(|r| r.parent == Some(id)) {
                    kid.list = list;
                }
            }
            MenuAction::Indent(id) => self.indent(id),
            MenuAction::Outdent(id) => self.outdent(id),
            MenuAction::ShowInfo(id) => self.open_details(id, cx),
            MenuAction::Delete(id) => self.delete_reminder(id, cx),
        }
    }

    fn reminder_menu(&mut self, id: u64, cx: &mut Cx) {
        if self.editing != Some(id) {
            self.finish_edit(cx);
        }
        self.selected = Some(id);
        let Some(r) = self.store.get(id).cloned() else { return };
        let mut items = vec![
            MenuItem::new("Show Info", MenuAction::ShowInfo(id)).shortcut("Ctrl+I"),
            MenuItem::new(if r.flagged { "Unflag" } else { "Flag" }, MenuAction::Flag(id, !r.flagged)),
            MenuItem::separator(),
        ];
        for (p, label) in [(Priority::None, "No Priority"), (Priority::Low, "Low Priority"), (Priority::Medium, "Medium Priority"), (Priority::High, "High Priority")] {
            items.push(MenuItem::new(label, MenuAction::Priority(id, p)).checked(r.priority == p));
        }
        items.push(MenuItem::separator());
        if r.parent.is_some() {
            items.push(MenuItem::new("Outdent Reminder", MenuAction::Outdent(id)).shortcut("Shift+Tab"));
        } else if self.store.children(id).next().is_none() {
            items.push(MenuItem::new("Indent Reminder", MenuAction::Indent(id)).shortcut("Tab"));
        }
        items.push(MenuItem::new("Delete", MenuAction::Delete(id)).shortcut("Delete"));
        self.menu = Some(Menu::new(cx.mouse, items));
    }

    fn list_menu(&mut self, id: u64, cx: &mut Cx) {
        let current = self.store.list(id).map(|l| l.color.clone()).unwrap_or_default();
        let mut items = vec![
            MenuItem::new("New Reminder", MenuAction::NewReminder).shortcut("Ctrl+N"),
            MenuItem::new("New List", MenuAction::NewList).shortcut("Ctrl+Shift+N"),
            MenuItem::separator(),
            MenuItem::new("Rename List", MenuAction::RenameList(id)),
        ];
        if self.store.lists.len() > 1 {
            items.push(MenuItem::new("Delete List", MenuAction::DeleteList(id)));
        }
        items.push(MenuItem::separator());
        for c in COLORS {
            let mut label = c.to_string();
            label[..1].make_ascii_uppercase();
            items.push(MenuItem::new(label, MenuAction::ListColor(id, c)).checked(current == c).swatch(rgb(color_rgb(c))));
        }
        self.menu = Some(Menu::new(cx.mouse, items));
    }

    fn move_menu(&mut self, id: u64, cx: &mut Cx) {
        let current = self.store.get(id).map_or(0, |r| r.list);
        let items = self
            .store
            .lists
            .iter()
            .map(|l| {
                let swatch = rgb(self.store.list_color(l.id));
                MenuItem::new(l.name.clone(), MenuAction::MoveTo(id, l.id)).checked(l.id == current).swatch(swatch)
            })
            .collect();
        self.menu = Some(Menu::new(cx.mouse, items));
    }

    // -- rendering ----------------------------------------------------------

    fn sync_input_styles(&self, t: &Theme) {
        let style = |size: f32, color: Color| TextStyle { size, line_height: 1.3, weight: Weight::Regular, color };
        for (input, s) in [
            (&self.title, style(13.0, t.text)),
            (&self.notes, style(12.0, t.text_secondary)),
            (&self.search, style(13.0, t.text)),
            (&self.rename, style(13.0, t.text)),
            (&self.details_notes, style(13.0, t.text)),
        ] {
            let mut i = input.borrow_mut();
            i.set_styles(s, None);
            i.placeholder_color = t.text_tertiary;
            i.caret_color = t.accent;
            i.selection_color = t.text_selection;
        }
    }

    fn sync_colors(t: &Theme) -> sync_ui::Colors {
        sync_ui::Colors {
            accent: t.accent,
            on_accent: Color::WHITE,
            content: t.content,
            field: t.field,
            text_secondary: t.text_secondary,
            hover: t.hover,
            border: t.menu_border,
            shadow: t.shadow,
        }
    }

    /// A 32×28 toolbar button: hover fill, darker while pressed, lighter
    /// in an inactive window.
    fn icon_button(
        id: &'static str,
        cx: &Cx,
        t: &Theme,
        color: Color,
        icon: fn(&mut Scene, Bounds, Color),
        on_click: impl Fn(&mut Self, &mut Cx) + 'static,
    ) -> Div<Self> {
        let color = native::toolbar_ink(cx, color);
        let pressed = cx.hovered(id) && cx.mouse_down;
        div()
            .id(id)
            .w(32.0)
            .h(28.0)
            .rounded(6.0)
            .items_center()
            .justify_center()
            .bg_opt(pressed.then_some(t.icon_pressed_bg))
            .hover_bg(if pressed { t.icon_pressed_bg } else { t.icon_hover_bg })
            .on_click(on_click)
            .child(canvas(move |s, b| icon(s, b, color)).size(20.0))
    }

    fn sidebar(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut header = div().h(native::TOOLBAR_H).shrink_0().flex_row().items_center().px(native::TRAFFIC_LIGHTS_X).window_drag();
        if let Some(buttons) = ui::chrome::window_buttons(true, cx, t.text) {
            header = header.child(buttons);
        }

        let icon_c = t.text_secondary;
        let query_set = !self.query.is_empty();
        let search = div()
            .id("search-box")
            .mx(native::SIDEBAR_INSET)
            .h(28.0)
            .shrink_0()
            .rounded(7.0)
            .bg(t.field)
            .focus_ring(cx.is_focused("search"), cx)
            .flex_row()
            .items_center()
            .gap(6.0)
            .px(8.0)
            .cursor(CursorIcon::Text)
            .on_mouse_down(|s: &mut Self, cx: &mut Cx| {
                s.finish_edit(cx);
                cx.focus("search");
            })
            .child(canvas(move |s, b| icons::magnifier(s, b, icon_c)).size(14.0).shrink_0())
            .child(input("search", &self.search).flex_1())
            .child_if(query_set, || {
                let pressed = cx.hovered("search-clear") && cx.mouse_down;
                div()
                    .id("search-clear")
                    .size(16.0)
                    .rounded(8.0)
                    .bg(icon_c.alpha(if pressed { 0.8 } else { 0.5 }))
                    .shrink_0()
                    .on_click(|s: &mut Self, cx: &mut Cx| {
                        s.query.clear();
                        s.search.borrow_mut().set_text(&mut cx.text.fonts, "");
                        cx.focus("list");
                    })
                    .child(canvas(|s, b| icons::xmark(s, b, Color::WHITE, 6.0, 1.4)).size(16.0))
                    .into()
            });

        let today = today();
        let tile = |smart: Smart| self.tile(smart, smart.count(&self.store, today), t);
        let grid = div()
            .flex_col()
            .gap(8.0)
            .px(native::SIDEBAR_INSET)
            .pt(12.0)
            .shrink_0()
            .child(div().flex_row().gap(8.0).child(tile(Smart::Today)).child(tile(Smart::Scheduled)))
            .child(div().flex_row().gap(8.0).child(tile(Smart::All)).child(tile(Smart::Flagged)))
            .child(div().flex_row().gap(8.0).child(tile(Smart::Completed)).child(div().flex_1()));

        let mut rows: Vec<AnyElement<Self>> = vec![native::sidebar_header("My Lists", cx).pt(20.0).into()];
        for l in &self.store.lists {
            rows.push(self.list_row(l, cx, t).into());
        }

        // "⊕ Add List" at the bottom, lined up with the sidebar's icons.
        let pressed = cx.hovered("add-list") && cx.mouse_down;
        let ink = if pressed { t.text } else { t.text_secondary };
        let add_list = div()
            .id("add-list")
            .h(40.0)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(8.0)
            .px(native::SIDEBAR_INSET + native::SIDEBAR_ROW_PAD)
            .on_click(|s: &mut Self, cx: &mut Cx| s.new_list(cx))
            .child(canvas(move |s, b| icons::plus_circle(s, b, ink)).size(native::SIDEBAR_ICON).shrink_0())
            .child(text("Add List").color(ink).truncate());

        div()
            .w(SIDEBAR_W)
            .h_full()
            .shrink_0()
            .flex_col()
            .bg(t.sidebar)
            .child(header)
            .child(search)
            .child(
                div()
                    .id("sidebar-scroll")
                    .scroll_y()
                    .flex_1()
                    .flex_col()
                    .child(grid)
                    .child(div().flex_col().gap(1.0).pb(8.0).children(rows)),
            )
            .child(self.pairing.sidebar_row(&Self::sync_colors(t)))
            .child(add_list)
    }

    /// A smart list's tile: icon badge and count on top, name below.
    fn tile(&self, smart: Smart, count: usize, t: &Theme) -> Div<Self> {
        let selected = self.view == View::Smart(smart) && !self.searching();
        let c = rgb(smart.color(t.dark));
        let (badge, glyph, ink, name_ink) = if selected {
            (Color::WHITE, c, Color::WHITE, Color::WHITE)
        } else {
            (c, Color::WHITE, t.text, t.text_secondary)
        };
        let icon: fn(&mut Scene, Bounds, Color) = match smart {
            Smart::Today => icons::calendar,
            Smart::Scheduled => icons::scheduled,
            Smart::All => icons::tray,
            Smart::Flagged => icons::flag,
            Smart::Completed => |s, b, c| icons::check(s, b, c, 2.0),
        };
        let mut tile = div()
            .id(("tile", smart))
            .flex_1()
            .h(62.0)
            .rounded(10.0)
            .pt(8.0)
            .pb(7.0)
            .px(9.0)
            .flex_col()
            .justify_between()
            .bg(if selected { c } else { t.tile })
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.set_view(View::Smart(smart), cx))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .size(26.0)
                            .rounded(13.0)
                            .bg(badge)
                            .items_center()
                            .justify_center()
                            .child(canvas(move |s, b| icon(s, b, glyph)).size(16.0)),
                    )
                    .child(text(count.to_string()).size(20.0).weight(Weight::Bold).tabular().color(ink)),
            )
            .child(text(smart.name()).size(13.0).weight(Weight::Semibold).color(name_ink).truncate());
        if !t.dark && !selected {
            tile = tile.shadow(rgba(0x000000, 0.08), 2.0, 0.5);
        }
        tile
    }

    fn list_row(&self, l: &List, cx: &Cx, t: &Theme) -> Div<Self> {
        let id = l.id;
        let selected = self.view == View::List(id) && !self.searching();
        let c = rgb(color_rgb(&l.color));
        let name: AnyElement<Self> = if self.renaming == Some(id) {
            div()
                .flex_1()
                .min_w_0()
                .h(22.0)
                .px(4.0)
                .items_center()
                .rounded(4.0)
                .bg(t.content)
                .border(t.divider, 0.5)
                .focus_ring(cx.is_focused("rename"), cx)
                .child(input("rename", &self.rename).flex_1())
                .into()
        } else {
            text(l.name.clone()).truncate().flex_1().into()
        };
        div()
            .id(("list", id))
            .h(native::SIDEBAR_ROW_H)
            .shrink_0()
            .mx(native::SIDEBAR_INSET)
            .pl(native::SIDEBAR_ROW_PAD - (LIST_ICON - native::SIDEBAR_ICON) / 2.0)
            .pr(native::SIDEBAR_ROW_PAD)
            .rounded(native::SIDEBAR_RADIUS)
            .flex_row()
            .items_center()
            .gap(6.0)
            .bg_opt(selected.then_some(native::sidebar_selection(cx)))
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                if s.renaming.is_some_and(|r| r != id) {
                    s.finish_rename(true, cx);
                }
                if s.renaming != Some(id) && (s.view != View::List(id) || s.searching()) {
                    s.set_view(View::List(id), cx);
                }
            })
            .on_double_click(move |s: &mut Self, cx: &mut Cx| s.start_rename(id, cx))
            .on_right_click(move |s: &mut Self, cx: &mut Cx| s.list_menu(id, cx))
            .child(
                div()
                    .size(LIST_ICON)
                    .rounded(LIST_ICON / 2.0)
                    .bg(c)
                    .shrink_0()
                    .items_center()
                    .justify_center()
                    .child(canvas(|s, b| icons::bullets(s, b, Color::WHITE)).size(12.0)),
            )
            .child(name)
            .child(text(self.store.count_in(id).to_string()).tabular().color(t.text_secondary).shrink_0())
    }

    fn content(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let color = self.view_color(t);
        let can_add = self.view != View::Smart(Smart::Completed);
        let completed_on = self.show_completed;
        let toggle_c = if completed_on { t.accent } else { t.icon };
        let mut tools = div().flex_row().items_center().gap(4.0).child(Self::icon_button(
            "toggle-completed",
            cx,
            t,
            toggle_c,
            icons::check_circle,
            |s: &mut Self, _: &mut Cx| s.show_completed = !s.show_completed,
        ));
        if can_add {
            tools = tools.child(Self::icon_button("add", cx, t, t.icon, icons::plus, |s: &mut Self, cx: &mut Cx| s.new_reminder(None, cx)));
        }
        let toolbar = div()
            .h(native::TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(10.0)
            .px(10.0)
            .window_drag()
            .child(tools)
            .children(ui::chrome::window_buttons(false, cx, t.text));

        let today = today();
        let count = match (self.searching(), self.view) {
            (true, _) => self.visible_ids().len(),
            (false, View::Smart(s)) => s.count(&self.store, today),
            (false, View::List(l)) => self.store.count_in(l),
        };
        let header = div()
            .flex_row()
            .items_center()
            .justify_between()
            .px(22.0)
            .pb(2.0)
            .shrink_0()
            .child(text(self.view_name()).size(26.0).weight(Weight::Bold).color(color).truncate().flex_1())
            .child(text(count.to_string()).size(26.0).weight(Weight::Medium).tabular().color(color).shrink_0());

        // "3 Completed · Show", under a list's title.
        let done_here = match (self.searching(), self.view) {
            (false, View::List(l)) => self.store.reminders.iter().filter(|r| r.list == l && r.is_done()).count(),
            (false, View::Smart(Smart::All)) => Smart::Completed.count(&self.store, today),
            _ => 0,
        };
        let subline = div()
            .flex_row()
            .items_center()
            .gap(4.0)
            .px(22.0)
            .h(20.0)
            .shrink_0()
            .child_if(done_here > 0, || text(format!("{done_here} Completed \u{2022}")).size(12.0).tabular().color(t.text_secondary).into())
            .child_if(done_here > 0, || {
                div()
                    .id("show-completed")
                    .cursor(CursorIcon::Pointer)
                    .on_click(|s: &mut Self, _: &mut Cx| s.show_completed = !s.show_completed)
                    .child(text(if completed_on { "Hide" } else { "Show" }).size(12.0).color(t.accent))
                    .into()
            });

        let sections = self.sections();
        let mut body: Vec<AnyElement<Self>> = Vec::new();
        let sel_focused = (cx.is_focused("list") || cx.is_focused("title") || cx.is_focused("notes")) && cx.window_focused;
        for sec in &sections {
            if let Some(title) = &sec.title {
                let c = sec.color.map_or(t.text, rgb);
                body.push(
                    div()
                        .ml(12.0)
                        .mr(18.0)
                        .mt(if body.is_empty() { 6.0 } else { 18.0 })
                        .pb(5.0)
                        .px(10.0)
                        .border_b(t.divider)
                        .child(text(title.clone()).size(15.0).weight(Weight::Bold).color(c))
                        .into(),
                );
            }
            let highlighted = |id: u64| self.selected == Some(id) || self.details == Some(id);
            for (i, row) in sec.rows.iter().enumerate() {
                let Some(r) = self.store.get(row.id) else { continue };
                // No separator under the last row, or touching a selected one.
                let last = i + 1 == sec.rows.len() || highlighted(row.id) || sec.rows.get(i + 1).is_some_and(|n| highlighted(n.id));
                body.push(self.reminder_row(r, row.depth, last, sel_focused, cx, t).into());
            }
        }
        let empty = sections.iter().all(|s| s.rows.is_empty());
        if empty {
            let msg = if self.searching() {
                "No Results"
            } else if matches!(self.view, View::Smart(Smart::Completed)) {
                "No Completed Reminders"
            } else {
                "No Reminders"
            };
            body.push(
                div()
                    .h(240.0)
                    .items_center()
                    .justify_center()
                    .child(text(msg).size(15.0).weight(Weight::Medium).color(t.text_tertiary))
                    .into(),
            );
        }
        // The space below the last reminder: a click adds one.
        body.push(
            div()
                .id("add-space")
                .grow()
                .min_h(120.0)
                .on_mouse_down(|s: &mut Self, cx: &mut Cx| {
                    s.menu = None;
                    if s.details.take().is_some() {
                    } else if s.editing.is_some() {
                        s.finish_edit(cx);
                    } else {
                        s.new_reminder(None, cx);
                    }
                })
                .into(),
        );

        div()
            .flex_1()
            .h_full()
            .flex_col()
            .bg(t.content)
            .child(toolbar)
            .child(header)
            .child(subline)
            .child(
                div()
                    .id("list")
                    .focusable()
                    .flex_1()
                    .flex_col()
                    .child(div().id("content-scroll").scroll_y().flex_1().flex_col().pt(4.0).children(body)),
            )
    }

    fn reminder_row(&self, r: &Reminder, depth: u8, last: bool, focused: bool, cx: &Cx, t: &Theme) -> Div<Self> {
        let id = r.id;
        let editing = self.editing == Some(id);
        let selected = self.selected == Some(id) || self.details == Some(id);
        let hovered = cx.hovered(("row", id));
        let list_c = rgb(self.store.list_color(r.list));
        let done = r.is_done();
        let today = today();

        let mut title_line = div().flex_row().items_center().gap(4.0).min_h(18.0);
        if r.priority != Priority::None {
            title_line = title_line.child(text(r.priority.marks()).weight(Weight::Bold).color(if done { t.text_secondary } else { list_c }).shrink_0());
        }
        title_line = if editing {
            title_line.child(input("title", &self.title).flex_1())
        } else {
            let title = if r.title.is_empty() { "New Reminder".to_string() } else { r.title.clone() };
            let c = if done || r.title.is_empty() { t.text_secondary } else { t.text };
            title_line.child(text(title).color(c).flex_1().min_w_0())
        };

        let mut lines = div().flex_col().flex_1().min_w_0().gap(2.0).child(title_line);
        if editing {
            lines = lines.child(input("notes", &self.notes).w_full());
        } else if !r.notes.is_empty() {
            lines = lines.child(
                div()
                    .id(("row-notes", id))
                    .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                        s.menu = None;
                        s.details = None;
                        s.start_edit(id, Field::Notes, cx);
                    })
                    .child(text(r.notes.clone()).size(12.0).line_height(1.3).color(t.text_secondary).w_full()),
            );
        }
        let mut meta = div().flex_row().gap(6.0).items_center();
        let mut has_meta = false;
        if let Some(due) = r.due {
            let late = !done && due.is_overdue(now());
            meta = meta.child(text(due_text(due, today)).size(12.0).tabular().color(if late { t.destructive } else { t.text_secondary }));
            has_meta = true;
        }
        // Smart lists say which list a reminder is in.
        if matches!(self.view, View::Smart(Smart::Today | Smart::Scheduled | Smart::Flagged | Smart::Completed)) && !self.searching() {
            if let Some(l) = self.store.list(r.list) {
                meta = meta.child(text(l.name.clone()).size(12.0).color(t.text_tertiary));
                has_meta = true;
            }
        }
        let subtasks = self.store.children(id).count();
        if subtasks > 0 && depth == 0 && !matches!(self.view, View::List(_) | View::Smart(Smart::All)) {
            meta = meta.child(text(format!("{subtasks} subtask{}", if subtasks == 1 { "" } else { "s" })).size(12.0).color(t.text_tertiary));
            has_meta = true;
        }
        if has_meta {
            lines = lines.child(meta);
        }

        let mut trailing = div().flex_row().items_center().gap(4.0).shrink_0();
        if selected || hovered {
            let c = t.accent;
            trailing = trailing.child(
                div()
                    .id(("info", id))
                    .size(22.0)
                    .rounded(11.0)
                    .items_center()
                    .justify_center()
                    .on_click(move |s: &mut Self, cx: &mut Cx| {
                        if s.details == Some(id) {
                            s.details = None;
                        } else {
                            s.open_details(id, cx);
                        }
                    })
                    .child(canvas(move |s, b| icons::info(s, b, c)).size(18.0)),
            );
        }
        if r.flagged {
            let c = t.flag;
            trailing = trailing.child(div().size(22.0).items_center().justify_center().child(canvas(move |s, b| icons::flag(s, b, c)).size(15.0)));
        }

        let ring = t.ring;
        let check = div()
            .id(("check", id))
            .size(22.0)
            .shrink_0()
            .rounded(11.0)
            .on_click(move |s: &mut Self, _: &mut Cx| s.toggle_done(id))
            .child(canvas(move |s, b| icons::completion(s, b, ring, list_c, done)).size(22.0));

        let content = div()
            .flex_row()
            .flex_1()
            .min_w_0()
            .items_start()
            .gap(8.0)
            .pt(2.0)
            .pb(9.0)
            .child(lines)
            .child(trailing);
        let content = if last { content } else { content.border_b(t.row_divider) };

        let bounds = self.row_bounds.clone();
        let mut row = div()
            .id(("row", id))
            .mx(10.0)
            .pl(10.0 + depth as f32 * INDENT)
            .pr(8.0)
            .pt(7.0)
            .rounded(7.0)
            .flex_row()
            .items_start()
            .gap(9.0)
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.menu = None;
                s.details = None;
                if s.editing != Some(id) {
                    s.start_edit(id, Field::Title, cx);
                }
            })
            .on_right_click(move |s: &mut Self, cx: &mut Cx| s.reminder_menu(id, cx))
            .child(canvas(move |_, b| {
                bounds.borrow_mut().insert(id, b);
            }).inset_0())
            .child(check)
            .child(content);
        if selected {
            row = row.bg(if focused { t.row_selection } else { t.hover });
        }
        row
    }

    /// A macOS switch.
    fn switch(id: (&'static str, u64), on: bool, t: &Theme, toggle: impl Fn(&mut Self, &mut Cx) + 'static) -> Div<Self> {
        let track = if on { t.accent } else if t.dark { rgba(0xffffff, 0.16) } else { rgba(0x000000, 0.12) };
        div()
            .id(id)
            .w(32.0)
            .h(18.0)
            .rounded(9.0)
            .bg(track)
            .p(1.5)
            .flex_row()
            .justify_end()
            .on_click(toggle)
            .child_if(on, || div().w(14.0).into())
            .child(div().size(15.0).rounded(7.5).bg(Color::WHITE).shadow(rgba(0x000000, 0.25), 1.5, 0.5))
            .child_if(!on, || div().w(14.0).into())
    }

    /// `‹ value ›` with buttons that step backward and forward.
    fn stepper(id: &'static str, value: String, t: &Theme, step: impl Fn(&mut Self, i64) + Clone + 'static) -> Div<Self> {
        let ink = t.text_secondary;
        let back = step.clone();
        div()
            .flex_row()
            .items_center()
            .h(26.0)
            .rounded(6.0)
            .bg(t.field)
            .child(
                div()
                    .id((id, 0))
                    .size(26.0)
                    .rounded(6.0)
                    .items_center()
                    .justify_center()
                    .hover_bg(t.hover)
                    .on_click(move |s: &mut Self, _: &mut Cx| back(s, -1))
                    .child(canvas(move |s, b| icons::chevron_left(s, b, ink)).size(16.0)),
            )
            .child(div().flex_1().min_w_0().items_center().justify_center().child(text(value).size(13.0).tabular().truncate()))
            .child(
                div()
                    .id((id, 1))
                    .size(26.0)
                    .rounded(6.0)
                    .items_center()
                    .justify_center()
                    .hover_bg(t.hover)
                    .on_click(move |s: &mut Self, _: &mut Cx| step(s, 1))
                    .child(canvas(move |s, b| icons::chevron_right(s, b, ink)).size(16.0)),
            )
    }

    /// The (i) popover: notes, date, time, flag, priority, list.
    fn details_popover(&self, id: u64, cx: &Cx, t: &Theme) -> Option<Div<Self>> {
        let r = self.store.get(id)?;
        // Point at the row's (i) button. Rows span to the window's right edge;
        // only their top is recorded reliably.
        let (w, h) = cx.window_size;
        let row_y = self.row_bounds.borrow().get(&id).map_or(h / 3.0, |b| b.y);
        let flag_w = if r.flagged { 26.0 } else { 0.0 };
        let anchor = Bounds::new(w - 18.0 - flag_w - 22.0, row_y + 4.0, 22.0, 22.0);

        let label = |s: &str| text(s.to_string()).size(13.0).flex_1();
        let row = || div().flex_row().items_center().gap(10.0).h(30.0);
        let hair = || div().h(1.0).bg(t.row_divider).w_full();
        let due = r.due;
        let day = due.map(|d| d.day()).unwrap_or_else(today);

        let mut card = div()
            .w_full()
            .flex_col()
            .gap(6.0)
            .px(16.0)
            .pt(14.0)
            .pb(12.0)
            .child(
                text(if r.title.is_empty() { "New Reminder".to_string() } else { r.title.clone() })
                    .size(15.0)
                    .weight(Weight::Bold)
                    .truncate(),
            )
            .child(
                div()
                    .id("details-notes-box")
                    .min_h(52.0)
                    .rounded(6.0)
                    .bg(t.field)
                    .focus_ring(cx.is_focused("details-notes"), cx)
                    .px(8.0)
                    .py(6.0)
                    .cursor(CursorIcon::Text)
                    .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("details-notes"))
                    .child(input("details-notes", &self.details_notes).w_full()),
            )
            .child(hair());

        // Date: on/off, then a day stepper.
        card = card.child(row().child(label("Date")).child(Self::switch(("sw-date", id), due.is_some(), t, move |s, _| {
            s.update(id, |r| r.due = if r.due.is_some() { None } else { Some(Due::date(today())) });
        })));
        if due.is_some() {
            card = card.child(Self::stepper("step-day", long_date(day), t, move |s: &mut Self, dir: i64| {
                s.update(id, |r| {
                    r.due = r.due.map(|d| match d {
                        Due::Date(_) => Due::date(d.day() + Days::days(dir)),
                        Due::Time(t) => Due::Time(at(d.day() + Days::days(dir), minutes_of(t))),
                    })
                });
            }));
        }
        // Time: on/off (turning it on sets a date too), then a 15-minute stepper.
        let timed = matches!(due, Some(Due::Time(_)));
        card = card.child(row().child(label("Time")).child(Self::switch(("sw-time", id), timed, t, move |s, _| {
            s.update(id, |r| {
                r.due = match r.due {
                    Some(Due::Time(t)) => Some(Due::date(Due::Time(t).day())),
                    Some(Due::Date(k)) => Some(Due::Time(at(Due::Date(k).day(), 9 * 60))),
                    None => Some(Due::Time(at(today(), 9 * 60))),
                };
            });
        })));
        if let Some(Due::Time(ts)) = due {
            card = card.child(Self::stepper("step-time", time_text(ts), t, move |s: &mut Self, dir: i64| {
                s.update(id, |r| {
                    if let Some(Due::Time(ts)) = r.due {
                        let m = (minutes_of(ts) + dir * 15).rem_euclid(24 * 60);
                        r.due = Some(Due::Time(at(Due::Time(ts).day(), m)));
                    }
                });
            }));
        }
        card = card.child(hair());
        card = card.child(row().child(label("Flag")).child(Self::switch(("sw-flag", id), r.flagged, t, move |s, _| {
            s.update(id, |r| r.flagged = !r.flagged);
        })));
        let p = r.priority as usize;
        card = card.child(row().child(label("Priority")).child(ui::widgets::segmented(
            cx,
            "priority",
            &["None", "Low", "Medium", "High"],
            p,
            move |s: &mut Self, i, _| s.update(id, |r| r.priority = Priority::from_i64(i as i64)),
        )));
        let list = self.store.list(r.list);
        let list_c = rgb(self.store.list_color(r.list));
        let ink = t.text_secondary;
        card = card.child(
            row().child(label("List")).child(
                div()
                    .id("details-list")
                    .h(24.0)
                    .px(8.0)
                    .rounded(6.0)
                    .flex_row()
                    .items_center()
                    .gap(6.0)
                    .hover_bg(t.hover)
                    .on_click(move |s: &mut Self, cx: &mut Cx| s.move_menu(id, cx))
                    .child(div().size(10.0).rounded(5.0).bg(list_c))
                    .child(text(list.map_or(String::new(), |l| l.name.clone())).size(13.0))
                    .child(canvas(move |s, b| icons::chevron_right(s, b, ink)).size(12.0)),
            ),
        );
        card = card.child(hair()).child(
            div()
                .id("details-delete")
                .h(28.0)
                .rounded(6.0)
                .items_center()
                .justify_center()
                .hover_bg(t.hover)
                .on_click(move |s: &mut Self, cx: &mut Cx| s.delete_reminder(id, cx))
                .child(text("Delete Reminder").color(t.destructive)),
        );

        Some(
            div()
                .inset_0()
                .deferred()
                .on_mouse_down(|s: &mut Self, _| s.details = None)
                .on_right_click(|s: &mut Self, _| s.details = None)
                .child(ui::widgets::popover(cx, anchor, POPOVER_W, POPOVER_H, t.popover, t.menu_border, t.shadow, card)),
        )
    }
}

impl PairingHost for RemindersApp {
    fn pairing(&mut self) -> &mut Pairing {
        &mut self.pairing
    }
}

impl App for RemindersApp {
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Self::theme(cx).text }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        let t = Self::theme(cx);
        self.sync_input_styles(&t);
        let (w, h) = cx.window_size;
        let mut root = div()
            .w(w)
            .h(h)
            .flex_row()
            .bg(t.content)
            .text_color(t.text)
            .child(self.sidebar(cx, &t))
            .child(div().w(1.0).h_full().shrink_0().bg(t.divider))
            .child(self.content(cx, &t));
        if let Some(pop) = self.details.and_then(|id| self.details_popover(id, cx, &t)) {
            root = root.child(pop);
        }
        if let Some(menu) = &self.menu {
            root = root.child(menu.render(cx, |s: &mut Self, a, cx| s.run_menu(a, cx), |s: &mut Self, _| s.menu = None));
        }
        if let Some(sheet) = self.pairing.sheet(cx, &Self::sync_colors(&t)) {
            root = root.child(sheet);
        }
        // Frameless windows get no compositor outline (except on y7ya-os), so
        // draw a hairline edge.
        if cx.transparent && !cx.maximized && !ui::is_y7ya() {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(t.window_border, 1.0));
        }
        root
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if self.pairing.on_key(key) {
            return true;
        }
        if let Some(menu) = &mut self.menu {
            match menu.key(key, cx) {
                MenuKey::Pick(a) => self.run_menu(a, cx),
                MenuKey::Close => self.menu = None,
                MenuKey::Stay => {}
            }
            return true;
        }
        if key.is_named(NamedKey::Escape) {
            if self.details.take().is_some() {
            } else if self.renaming.is_some() {
                self.finish_rename(false, cx);
            } else if self.editing.is_some() {
                self.finish_edit(cx);
            } else if cx.is_focused("search") {
                self.query.clear();
                self.search.borrow_mut().set_text(&mut cx.text.fonts, "");
                cx.focus("list");
            }
            return true;
        }
        if key.ctrl && !key.alt {
            if key.is_char('n') && key.shift {
                self.new_list(cx);
                return true;
            }
            if key.is_char('n') {
                self.new_reminder(None, cx);
                return true;
            }
            if key.is_char('f') {
                self.finish_edit(cx);
                self.search.borrow_mut().select_all();
                cx.focus("search");
                return true;
            }
            if key.is_char('i') {
                if let Some(id) = self.editing.or(self.selected) {
                    self.open_details(id, cx);
                }
                return true;
            }
            if key.is_char('q') || key.is_char('w') {
                self.finish_edit(cx);
                self.save(cx);
                cx.close_window();
                return true;
            }
            if key.is_char('s') {
                self.save(cx);
                return true;
            }
        }
        if self.renaming.is_some() {
            if key.is_named(NamedKey::Enter) {
                self.finish_rename(true, cx);
                return true;
            }
            if !cx.is_focused("rename") {
                self.finish_rename(true, cx);
            }
            return false;
        }
        let editing_title = cx.is_focused("title") && self.editing.is_some();
        if editing_title {
            let id = self.editing.unwrap();
            if key.is_named(NamedKey::Enter) {
                // Enter on a reminder with a title starts the next one.
                if self.store.get(id).is_some_and(|r| !r.title.trim().is_empty()) {
                    self.new_reminder(Some(id), cx);
                } else {
                    self.finish_edit(cx);
                }
                return true;
            }
            if key.is_named(NamedKey::Tab) {
                if key.shift { self.outdent(id) } else { self.indent(id) }
                return true;
            }
            if key.is_named(NamedKey::ArrowUp) || key.is_named(NamedKey::ArrowDown) {
                self.move_selection(if key.is_named(NamedKey::ArrowUp) { -1 } else { 1 }, cx);
                return true;
            }
            return false;
        }
        if cx.is_focused("search") && key.is_named(NamedKey::Enter) {
            cx.focus("list");
            return true;
        }
        if cx.is_focused("list") {
            let sel = self.selected;
            match &key.key {
                Key::Named(NamedKey::ArrowDown) => self.move_selection(1, cx),
                Key::Named(NamedKey::ArrowUp) => self.move_selection(-1, cx),
                Key::Named(NamedKey::Enter) => match sel {
                    Some(id) => self.start_edit(id, Field::Title, cx),
                    None => self.new_reminder(None, cx),
                },
                Key::Named(NamedKey::Delete | NamedKey::Backspace) => {
                    if let Some(id) = sel {
                        self.delete_reminder(id, cx);
                    }
                }
                Key::Named(NamedKey::Space) => {
                    if let Some(id) = sel {
                        self.toggle_done(id);
                    }
                }
                Key::Named(NamedKey::Tab) => {
                    if let Some(id) = sel {
                        if key.shift { self.outdent(id) } else { self.indent(id) }
                    }
                }
                _ => return false,
            }
            return true;
        }
        false
    }

    fn on_input(&mut self, id: ElementId, cx: &mut Cx) {
        if id == eid("title") || id == eid("notes") {
            let title = id == eid("title");
            let text = if title { self.title.borrow().text() } else { self.notes.borrow().text() };
            if let Some(rid) = self.editing {
                self.update(rid, |r| if title { r.title = text } else { r.notes = text });
            }
        } else if id == eid("details-notes") {
            let text = self.details_notes.borrow().text();
            if let Some(rid) = self.details {
                self.update(rid, |r| r.notes = text);
            }
        } else if id == eid("search") {
            self.query = self.search.borrow().text();
            self.selected = None;
            cx.scroll_to_top("content-scroll");
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        let save = self.dirty_since.map(|t| t + SAVE_DELAY);
        save.into_iter().chain(self.pairing.next_wake()).min()
    }

    fn on_wake(&mut self, cx: &mut Cx) {
        if self.dirty_since.is_some_and(|t| t.elapsed() >= SAVE_DELAY) {
            self.save(cx);
        }
    }

    fn on_external(&mut self, cx: &mut Cx) {
        if self.events.as_ref().is_some_and(|e| e.take_changed(DOC)) {
            // Merge any unsaved edits first, then show the combined result.
            self.dirty_since = None;
            self.store.save();
            self.after_reload(cx);
        }
    }

    fn on_exit(&mut self) {
        if let Some(id) = self.editing.take() {
            if self.store.get(id).is_some_and(|r| r.title.trim().is_empty() && r.notes.trim().is_empty()) {
                self.store.reminders.retain(|r| r.id != id);
            }
        }
        self.store.save();
        if let Some(core) = &self.store.core {
            core.sync().shutdown();
        }
    }
}

/// Sample content for `--snapshot` renders, dated relative to today.
pub fn demo_store() -> Store {
    let mut s = Store::detached();
    let (groceries, work, family) = (901, 902, 903);
    s.lists.push(List { id: groceries, name: "Groceries".into(), color: "orange".into() });
    s.lists.push(List { id: work, name: "Work".into(), color: "purple".into() });
    s.lists.push(List { id: family, name: "Family".into(), color: "green".into() });
    let today = today();
    let base = now() - 30 * 86400;
    let mut n = 0;
    let mut add = |list: u64, title: &str, f: &dyn Fn(&mut Reminder)| -> u64 {
        let mut r = Reminder::new(list, title);
        n += 1;
        r.created = base + n * 60;
        f(&mut r);
        let id = r.id;
        s.reminders.push(r);
        id
    };
    let day = |d: i64| Due::date(today + Days::days(d));
    let time = |d: i64, h: i64, m: i64| Due::Time(at(today + Days::days(d), h * 60 + m));
    add(DEFAULT_LIST, "Call the dentist", &|r| {
        r.due = Some(day(-1));
        r.priority = Priority::High;
    });
    add(DEFAULT_LIST, "Renew passport", &|r| {
        r.due = Some(time(0, 9, 30));
        r.notes = "Photos are in the top drawer".into();
        r.flagged = true;
    });
    add(DEFAULT_LIST, "Pick up dry cleaning", &|r| r.due = Some(time(0, 17, 0)));
    add(DEFAULT_LIST, "Water the plants", &|r| r.due = Some(day(1)));
    add(DEFAULT_LIST, "Back up the laptop", &|_| {});
    add(DEFAULT_LIST, "Return library books", &|r| r.completed = Some(now() - 3600));
    add(groceries, "Oat milk", &|r| r.priority = Priority::Low);
    add(groceries, "Sourdough", &|_| {});
    add(groceries, "Lemons", &|r| r.notes = "Six, unwaxed".into());
    add(groceries, "Coffee beans", &|r| r.flagged = true);
    add(groceries, "Eggs", &|r| r.completed = Some(now() - 7200));
    let launch = add(work, "Ship the redesign", &|r| {
        r.due = Some(day(3));
        r.priority = Priority::Medium;
        r.flagged = true;
    });
    add(work, "Final QA pass", &|r| r.parent = Some(launch));
    add(work, "Write release notes", &|r| r.parent = Some(launch));
    add(work, "Review Sam's PR", &|r| r.due = Some(time(0, 14, 0)));
    add(work, "Quarterly report", &|r| r.due = Some(day(12)));
    add(work, "Expense receipts", &|r| r.completed = Some(now() - 86400 * 2));
    add(family, "Book flights for the holidays", &|r| {
        r.due = Some(day(6));
        r.notes = "Check the Thursday morning ones".into();
    });
    add(family, "Mom's birthday gift", &|r| {
        r.due = Some(day(9));
        r.priority = Priority::High;
    });
    add(family, "Plan the weekend hike", &|r| r.completed = Some(now() - 600));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(app: &mut RemindersApp, cx: &mut Cx, s: &str) {
        for ch in s.chars() {
            let kp = KeyPress { key: Key::Character(ch.to_string().into()), text: Some(ch.to_string()), ctrl: false, shift: false, alt: false };
            let out = app.title.borrow_mut().handle_key(&mut cx.text.fonts, &kp, &mut None);
            if out.changed {
                app.on_input(eid("title"), cx);
            }
        }
    }

    fn press(app: &mut RemindersApp, cx: &mut Cx, key: NamedKey, shift: bool) {
        let kp = KeyPress { key: Key::Named(key), text: None, ctrl: false, shift, alt: false };
        assert!(app.on_key(&kp, cx));
    }

    fn titles(app: &RemindersApp) -> Vec<String> {
        app.visible_ids().iter().map(|&id| app.store.get(id).unwrap().title.clone()).collect()
    }

    #[test]
    fn quick_add_subtasks_and_completion() {
        let path = std::env::temp_dir().join("reminders-selftest.png");
        ui::snapshot((980.0, 660.0), 1.0, false, &path, |cx| RemindersApp::new(demo_store(), None, cx), |app, cx| {
            app.set_view(View::List(901), cx);
            assert_eq!(titles(app), ["Oat milk", "Sourdough", "Lemons", "Coffee beans"]);

            // Enter on a reminder starts a new one right below it.
            let oat = app.visible_ids()[0];
            app.start_edit(oat, Field::Title, cx);
            press(app, cx, NamedKey::Enter, false);
            assert!(cx.is_focused("title"));
            type_text(app, cx, "Butter");
            press(app, cx, NamedKey::Enter, false);
            type_text(app, cx, "Salted");
            assert_eq!(titles(app), ["Oat milk", "Butter", "Salted", "Sourdough", "Lemons", "Coffee beans"]);

            // Tab makes it a subtask of the one above; Shift+Tab undoes it.
            press(app, cx, NamedKey::Tab, false);
            let salted = app.editing.unwrap();
            let butter = app.visible_ids()[1];
            assert_eq!(app.store.get(salted).unwrap().parent, Some(butter));
            assert_eq!(app.sections()[0].rows[2].depth, 1);
            press(app, cx, NamedKey::Tab, true);
            assert_eq!(app.store.get(salted).unwrap().parent, None);
            press(app, cx, NamedKey::Tab, false);

            // An empty new reminder disappears when editing ends.
            press(app, cx, NamedKey::Enter, false);
            let blank = app.editing.unwrap();
            press(app, cx, NamedKey::Escape, false);
            assert!(app.store.get(blank).is_none());
            assert_eq!(app.store.count_in(901), 6);

            // Checking one off keeps it in place until the view changes.
            let sourdough = app.visible_ids()[3];
            app.toggle_done(sourdough);
            assert!(app.visible_ids().contains(&sourdough));
            assert_eq!(Smart::Completed.count(&app.store, today()), 5);
            app.set_view(View::Smart(Smart::Flagged), cx);
            app.set_view(View::List(901), cx);
            assert!(!app.visible_ids().contains(&sourdough));
            // A completed parent still shows while it has open subtasks.
            app.toggle_done(butter);
            app.set_view(View::List(902), cx);
            app.set_view(View::List(901), cx);
            assert!(app.visible_ids().contains(&butter) && app.visible_ids().contains(&salted));

            // Smart views add reminders that belong in them.
            app.set_view(View::Smart(Smart::Flagged), cx);
            app.new_reminder(None, cx);
            let id = app.editing.unwrap();
            assert!(app.store.get(id).unwrap().flagged);

            // The details popover changes the date, time and priority.
            type_text(app, cx, "Flagged one");
            app.open_details(id, cx);
            app.update(id, |r| r.due = Some(Due::Time(at(today(), 9 * 60))));
            assert_eq!(app.sections()[0].rows.len(), 4);
            app.run_menu(MenuAction::Priority(id, Priority::High), cx);
            assert_eq!(app.store.get(id).unwrap().priority, Priority::High);

            // The context menu works from the keyboard: typing jumps to a
            // row, Return picks it, Escape closes only the menu.
            app.reminder_menu(id, cx);
            let l = KeyPress { key: Key::Character("l".into()), text: Some("l".into()), ctrl: false, shift: false, alt: false };
            assert!(app.on_key(&l, cx));
            press(app, cx, NamedKey::Enter, false);
            assert!(app.menu.is_none());
            assert_eq!(app.store.get(id).unwrap().priority, Priority::Low);
            app.reminder_menu(id, cx);
            press(app, cx, NamedKey::ArrowDown, false);
            press(app, cx, NamedKey::Escape, false);
            assert!(app.menu.is_none() && app.details == Some(id));
            assert!(app.store.get(id).unwrap().flagged);
        });
        assert!(path.exists());
    }
}
