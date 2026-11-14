mod icons;
pub mod model;
mod theme;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ui::widgets::{Menu, MenuItem, MenuKey};
use ui::*;
use model::{DEFAULT_FOLDER, Folder, Note, Store, full_date, list_date, now, section};
use notes_core::NOTES_DOC;
use sync_ui::{Pairing, PairingHost, SyncEvents};
use theme::Theme;

const LIST_W: f32 = 300.0;
const SAVE_DELAY: Duration = Duration::from_millis(600);
const PURGE_AFTER_SECS: i64 = 30 * 24 * 3600;

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Folder(u64),
    Deleted,
}

#[derive(Clone, Debug, PartialEq)]
enum MenuAction {
    NewNote,
    NewFolder,
    Pin(u64, bool),
    Move(u64, u64),
    DeleteNote(u64),
    Recover(u64),
    RenameFolder(u64),
    DeleteFolder(u64),
}

pub struct NotesApp {
    store: Store,
    view: View,
    selected: Option<u64>,
    editor: Rc<RefCell<TextInput>>,
    editor_note: Option<u64>,
    search: Rc<RefCell<TextInput>>,
    query: String,
    rename: Rc<RefCell<TextInput>>,
    renaming: Option<u64>,
    sidebar_open: bool,
    menu: Option<Menu<MenuAction>>,
    dirty_since: Option<Instant>,
    events: Option<Arc<SyncEvents>>,
    pairing: Pairing,
}

impl NotesApp {
    pub fn new(mut store: Store, events: Option<Arc<SyncEvents>>, cx: &mut Cx) -> Self {
        let cutoff = now() - PURGE_AFTER_SECS;
        let before = store.notes.len();
        store.notes.retain(|n| n.deleted_at.is_none_or(|t| t > cutoff));
        if store.notes.len() != before {
            store.save();
        }
        if let Some(events) = &events {
            events.set_waker(cx.waker());
        }

        let fonts = &mut cx.text.fonts;
        let mut editor = TextInput::new(fonts, TextStyle::default(), false);
        editor.first_line_style = Some(TextStyle::default());
        let mut search = TextInput::new(fonts, TextStyle::default(), true);
        search.placeholder = "Search".into();
        let rename = TextInput::new(fonts, TextStyle::default(), true);
        cx.window_radius = 8.0;
        let hub = store.core.as_ref().map(|c| c.hub());

        let mut app = Self {
            store,
            view: View::Folder(DEFAULT_FOLDER),
            selected: None,
            editor: Rc::new(RefCell::new(editor)),
            editor_note: None,
            search: Rc::new(RefCell::new(search)),
            query: String::new(),
            rename: Rc::new(RefCell::new(rename)),
            renaming: None,
            sidebar_open: true,
            menu: None,
            dirty_since: None,
            events,
            pairing: Pairing::new(hub),
        };
        app.ensure_selection(cx);
        cx.focus("list");
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

    /// The store picked up edits from another device: bring the UI in line.
    fn after_reload(&mut self, cx: &mut Cx) {
        if let Some(id) = self.editor_note {
            match self.store.note(id) {
                Some(n) if self.editor.borrow().text() != n.body => {
                    let body = n.body.clone();
                    self.editor.borrow_mut().replace_text(&mut cx.text.fonts, &body);
                }
                Some(_) => {}
                None => {
                    self.selected = None;
                    self.editor_note = None;
                }
            }
        }
        if let View::Folder(f) = self.view {
            if !self.store.folders.iter().any(|x| x.id == f) {
                self.view = View::Folder(DEFAULT_FOLDER);
            }
        }
        if self.renaming.is_some_and(|id| !self.store.folders.iter().any(|f| f.id == id)) {
            self.renaming = None;
        }
        self.ensure_selection(cx);
    }

    /// For previews: blocks until the pairing code is ready.
    pub fn wait_for_pairing_code(&self) {
        self.pairing.wait_for_code();
    }

    pub fn open_sync(&mut self) {
        self.menu = None;
        self.pairing.open();
    }

    // -- previews -----------------------------------------------------------

    /// Shows the folder with this name, or Recently Deleted for "deleted".
    pub fn show_folder(&mut self, name: &str, cx: &mut Cx) {
        let view = match self.store.folders.iter().find(|f| f.name.eq_ignore_ascii_case(name)) {
            Some(f) => View::Folder(f.id),
            None if name == "deleted" => View::Deleted,
            None => panic!("no folder named {name}"),
        };
        self.set_view(view, cx);
    }

    pub fn search_for(&mut self, q: &str, cx: &mut Cx) {
        self.search.borrow_mut().set_text(&mut cx.text.fonts, q);
        self.on_input(eid("search"), cx);
        cx.focus("search");
    }

    /// Opens the selected note's context menu (or its folder's) next to it,
    /// with the pointer over the second row.
    pub fn menu_preview(&mut self, folder: bool, cx: &mut Cx) {
        if folder {
            cx.mouse = Point { x: 120.0, y: 112.0 };
            let id = match self.view {
                View::Folder(f) => f,
                View::Deleted => DEFAULT_FOLDER,
            };
            self.folder_menu(id, cx);
        } else if let Some(id) = self.selected {
            cx.mouse = Point { x: native::SIDEBAR_W + 150.0, y: 110.0 };
            self.note_menu(id, cx);
        }
        cx.mouse.y += 6.0 + 24.0 + 12.0;
        cx.mouse.x += 20.0;
    }

    pub fn rename_preview(&mut self, cx: &mut Cx) {
        if let View::Folder(f) = self.view {
            self.start_rename(f, cx);
        }
    }

    pub fn hide_sidebar(&mut self) {
        self.sidebar_open = false;
    }

    // -- queries ------------------------------------------------------------

    fn searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    fn visible_notes(&self) -> Vec<&Note> {
        let q = self.query.trim().to_lowercase();
        let mut v: Vec<&Note> = self
            .store
            .notes
            .iter()
            .filter(|n| {
                if !q.is_empty() {
                    return n.deleted_at.is_none() && n.body.to_lowercase().contains(&q);
                }
                match self.view {
                    View::Folder(f) => n.folder == f && n.deleted_at.is_none(),
                    View::Deleted => n.deleted_at.is_some(),
                }
            })
            .collect();
        let pins = q.is_empty() && self.view != View::Deleted;
        v.sort_by(|a, b| (pins && b.pinned).cmp(&(pins && a.pinned)).then(b.modified.cmp(&a.modified)));
        v
    }

    fn visible_ids(&self) -> Vec<u64> {
        self.visible_notes().iter().map(|n| n.id).collect()
    }

    // -- selection ----------------------------------------------------------

    fn select(&mut self, id: Option<u64>, cx: &mut Cx) {
        if self.selected == id {
            return;
        }
        // Like Notes, a note left empty disappears once you move on.
        if let Some(prev) = self.selected {
            if self.store.note(prev).is_some_and(|n| n.is_blank() && n.deleted_at.is_none()) {
                self.store.notes.retain(|n| n.id != prev);
                self.mark_dirty();
            }
        }
        self.selected = id;
        self.load_editor(cx);
    }

    fn load_editor(&mut self, cx: &mut Cx) {
        let body = self.selected.and_then(|id| self.store.note(id)).map(|n| n.body.clone()).unwrap_or_default();
        self.editor.borrow_mut().set_text(&mut cx.text.fonts, &body);
        self.editor_note = self.selected;
        cx.scroll_to_top("editor-scroll");
    }

    fn ensure_selection(&mut self, cx: &mut Cx) {
        let ids = self.visible_ids();
        if self.selected.is_none_or(|s| !ids.contains(&s)) {
            self.select(ids.first().copied(), cx);
        }
    }

    fn set_view(&mut self, view: View, cx: &mut Cx) {
        self.menu = None;
        if self.searching() {
            self.clear_search(cx);
        }
        self.view = view;
        cx.scroll_to_top("list-scroll");
        self.ensure_selection(cx);
    }

    fn clear_search(&mut self, cx: &mut Cx) {
        self.query.clear();
        self.search.borrow_mut().set_text(&mut cx.text.fonts, "");
        self.ensure_selection(cx);
    }

    fn move_selection(&mut self, delta: i32, cx: &mut Cx) {
        let ids = self.visible_ids();
        if ids.is_empty() {
            return;
        }
        let i = self.selected.and_then(|s| ids.iter().position(|&x| x == s)).map_or(0, |i| i as i32 + delta);
        let i = i.clamp(0, ids.len() as i32 - 1) as usize;
        self.select(Some(ids[i]), cx);
    }

    // -- actions ------------------------------------------------------------

    fn new_note(&mut self, cx: &mut Cx) {
        self.menu = None;
        if self.searching() {
            self.clear_search(cx);
        }
        let folder = match self.view {
            View::Folder(f) => f,
            View::Deleted => {
                self.view = View::Folder(DEFAULT_FOLDER);
                DEFAULT_FOLDER
            }
        };
        let reuse = self.selected.and_then(|id| self.store.note(id)).is_some_and(|n| n.is_blank() && n.folder == folder);
        if !reuse {
            let id = self.store.alloc_id();
            let t = now();
            self.store.notes.push(Note { id, folder, body: String::new(), created: t, modified: t, pinned: false, deleted_at: None });
            self.select(Some(id), cx);
            self.mark_dirty();
        }
        cx.scroll_to_top("list-scroll");
        cx.focus("editor");
    }

    fn delete_note(&mut self, id: u64, cx: &mut Cx) {
        let ids = self.visible_ids();
        let next = ids.iter().position(|&x| x == id).and_then(|i| ids.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| ids.get(j))));
        let next = next.copied().filter(|&n| n != id);
        if let Some(note) = self.store.note_mut(id) {
            if note.deleted_at.is_some() || note.is_blank() {
                self.store.notes.retain(|n| n.id != id);
            } else {
                note.deleted_at = Some(now());
                note.pinned = false;
            }
            self.mark_dirty();
        }
        if self.selected == Some(id) {
            self.selected = None;
            self.select(next, cx);
        }
        self.ensure_selection(cx);
    }

    fn recover_note(&mut self, id: u64, cx: &mut Cx) {
        let folders: Vec<u64> = self.store.folders.iter().map(|f| f.id).collect();
        if let Some(n) = self.store.note_mut(id) {
            n.deleted_at = None;
            if !folders.contains(&n.folder) {
                n.folder = DEFAULT_FOLDER;
            }
            self.mark_dirty();
        }
        self.ensure_selection(cx);
    }

    fn new_folder(&mut self, cx: &mut Cx) {
        self.menu = None;
        let id = self.store.alloc_id();
        let name = self.store.unique_folder_name("New Folder");
        self.store.folders.push(Folder { id, name });
        self.mark_dirty();
        self.set_view(View::Folder(id), cx);
        self.start_rename(id, cx);
    }

    fn start_rename(&mut self, id: u64, cx: &mut Cx) {
        if id == DEFAULT_FOLDER {
            return;
        }
        self.menu = None;
        let name = self.store.folder_name(id).to_string();
        let mut r = self.rename.borrow_mut();
        r.set_text(&mut cx.text.fonts, &name);
        r.select_all();
        drop(r);
        self.renaming = Some(id);
        cx.focus("rename");
    }

    fn finish_rename(&mut self, commit: bool, cx: &mut Cx) {
        let Some(id) = self.renaming.take() else { return };
        if commit {
            let name = self.rename.borrow().text().trim().to_string();
            if !name.is_empty() {
                if let Some(f) = self.store.folders.iter_mut().find(|f| f.id == id) {
                    f.name = name;
                    self.mark_dirty();
                }
            }
        }
        cx.focus("list");
    }

    fn delete_folder(&mut self, id: u64, cx: &mut Cx) {
        if id == DEFAULT_FOLDER {
            return;
        }
        let t = now();
        for n in self.store.notes.iter_mut().filter(|n| n.folder == id && n.deleted_at.is_none()) {
            n.deleted_at = Some(t);
            n.pinned = false;
        }
        self.store.folders.retain(|f| f.id != id);
        self.mark_dirty();
        if self.view == View::Folder(id) {
            self.set_view(View::Folder(DEFAULT_FOLDER), cx);
        }
        self.ensure_selection(cx);
    }

    fn run_menu(&mut self, action: MenuAction, cx: &mut Cx) {
        self.menu = None;
        match action {
            MenuAction::NewNote => self.new_note(cx),
            MenuAction::NewFolder => self.new_folder(cx),
            MenuAction::Pin(id, pinned) => {
                if let Some(n) = self.store.note_mut(id) {
                    n.pinned = pinned;
                    self.mark_dirty();
                }
            }
            MenuAction::Move(id, folder) => {
                if let Some(n) = self.store.note_mut(id) {
                    n.folder = folder;
                    self.mark_dirty();
                }
                self.ensure_selection(cx);
            }
            MenuAction::DeleteNote(id) => self.delete_note(id, cx),
            MenuAction::Recover(id) => self.recover_note(id, cx),
            MenuAction::RenameFolder(id) => self.start_rename(id, cx),
            MenuAction::DeleteFolder(id) => self.delete_folder(id, cx),
        }
    }

    fn note_menu(&mut self, id: u64, cx: &mut Cx) {
        self.select(Some(id), cx);
        cx.focus("list");
        let Some(note) = self.store.note(id) else { return };
        let items = if note.deleted_at.is_some() {
            vec![
                MenuItem::new("Recover", MenuAction::Recover(id)),
                MenuItem::separator(),
                MenuItem::new("Delete Permanently", MenuAction::DeleteNote(id)).destructive(),
            ]
        } else {
            let mut items = vec![
                MenuItem::new(if note.pinned { "Unpin Note" } else { "Pin Note" }, MenuAction::Pin(id, !note.pinned)),
                MenuItem::separator(),
            ];
            for f in self.store.folders.iter().filter(|f| f.id != note.folder) {
                items.push(MenuItem::new(format!("Move to \u{201c}{}\u{201d}", f.name), MenuAction::Move(id, f.id)));
            }
            if self.store.folders.len() > 1 {
                items.push(MenuItem::separator());
            }
            items.push(MenuItem::new("Delete", MenuAction::DeleteNote(id)).shortcut("Delete"));
            items
        };
        self.menu = Some(Menu::new(cx.mouse, items));
    }

    fn folder_menu(&mut self, id: u64, cx: &mut Cx) {
        let mut items = vec![
            MenuItem::new("New Note", MenuAction::NewNote).shortcut("Ctrl+N"),
            MenuItem::new("New Folder", MenuAction::NewFolder).shortcut("Ctrl+Shift+N"),
        ];
        if id != DEFAULT_FOLDER {
            items.push(MenuItem::separator());
            items.push(MenuItem::new("Rename Folder", MenuAction::RenameFolder(id)));
            items.push(MenuItem::new("Delete Folder", MenuAction::DeleteFolder(id)));
        }
        self.menu = Some(Menu::new(cx.mouse, items));
    }

    // -- rendering ----------------------------------------------------------

    fn sync_input_styles(&self, t: &Theme) {
        let mut e = self.editor.borrow_mut();
        e.set_styles(
            TextStyle { size: 14.0, line_height: 1.45, weight: Weight::Regular, color: t.text },
            Some(TextStyle { size: 24.0, line_height: 1.3, weight: Weight::Bold, color: t.text }),
        );
        e.caret_color = t.caret;
        e.selection_color = t.text_selection;
        for input in [&self.search, &self.rename] {
            let mut i = input.borrow_mut();
            i.set_styles(TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: t.text }, None);
            i.placeholder_color = t.text_secondary;
            i.caret_color = t.text;
            i.selection_color = t.text_selection;
        }
    }

    /// Window buttons for one end of a header; `None` when that end has none.
    fn window_buttons(&self, left: bool, cx: &Cx, t: &Theme) -> Option<Div<Self>> {
        ui::chrome::window_buttons(left, cx, t.text)
    }

    /// A 32×28 toolbar button: hover fill, darker while pressed, dimmed when
    /// disabled or in an inactive window.
    fn icon_button(
        id: &'static str,
        enabled: bool,
        cx: &Cx,
        t: &Theme,
        icon: fn(&mut Scene, Bounds, Color),
        on_click: impl Fn(&mut Self, &mut Cx) + 'static,
    ) -> Div<Self> {
        let ink = native::toolbar_ink(cx, t.icon);
        let color = if enabled { ink } else { ink.alpha(ink.a * 0.4) };
        let button = div().id(id).w(32.0).h(28.0).rounded(6.0).items_center().justify_center();
        if !enabled {
            return button.child(canvas(move |s, b| icon(s, b, color)).size(20.0));
        }
        let pressed = cx.hovered(id) && cx.mouse_down;
        button
            .bg_opt(pressed.then_some(t.icon_pressed_bg))
            .hover_bg(if pressed { t.icon_pressed_bg } else { t.icon_hover_bg })
            .on_click(on_click)
            .child(canvas(move |s, b| icon(s, b, color)).size(20.0))
    }

    fn sidebar(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut header = div()
            .h(native::TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .justify_between()
            .pl(native::TRAFFIC_LIGHTS_X)
            .pr(10.0)
            .window_drag();
        if let Some(buttons) = self.window_buttons(true, cx, t) {
            header = header.child(buttons);
        }
        let header = header.child(Self::icon_button("toggle-sidebar", true, cx, t, icons::sidebar, |s, _| s.sidebar_open = false));

        let mut rows: Vec<AnyElement<Self>> = vec![native::sidebar_header("On My Computer", cx).pt(4.0).into()];
        for f in &self.store.folders {
            rows.push(self.folder_row(f.id, &f.name, self.store.count_in(f.id), icons::folder, View::Folder(f.id), cx, t).into());
        }
        let deleted = self.store.deleted_count();
        if deleted > 0 || self.view == View::Deleted {
            rows.push(self.folder_row(u64::MAX, "Recently Deleted", deleted, icons::trash, View::Deleted, cx, t).into());
        }

        // "⊕ New Folder" at the bottom, lined up with the folder icons.
        let pressed = cx.hovered("new-folder") && cx.mouse_down;
        let accent = if pressed { t.accent.alpha(0.6) } else { t.accent };
        let new_folder = div()
            .id("new-folder")
            .h(44.0)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(8.0)
            .px(native::SIDEBAR_INSET + native::SIDEBAR_ROW_PAD)
            .on_click(|s: &mut Self, cx: &mut Cx| s.new_folder(cx))
            .child(canvas(move |s, b| icons::plus_circle(s, b, accent)).size(native::SIDEBAR_ICON).shrink_0())
            .child(text("New Folder").color(if pressed { t.text } else { t.text_secondary }).truncate());

        div()
            .w(native::SIDEBAR_W)
            .h_full()
            .shrink_0()
            .flex_col()
            .bg(t.sidebar)
            .child(header)
            .child(div().id("sidebar-scroll").scroll_y().flex_1().flex_col().gap(1.0).pb(8.0).children(rows))
            .child(self.pairing.sidebar_row(&Self::sync_colors(t)))
            .child(new_folder)
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

    fn folder_row(
        &self,
        id: u64,
        name: &str,
        count: usize,
        icon: fn(&mut Scene, Bounds, Color),
        view: View,
        cx: &Cx,
        t: &Theme,
    ) -> Div<Self> {
        let selected = self.view == view && !self.searching();
        let accent = native::sidebar_icon(cx, t.accent);
        let name_el: AnyElement<Self> = if self.renaming == Some(id) {
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
            text(name.to_string()).truncate().flex_1().into()
        };
        div()
            .id(("folder", id))
            .h(native::SIDEBAR_ROW_H)
            .shrink_0()
            .mx(native::SIDEBAR_INSET)
            .px(native::SIDEBAR_ROW_PAD)
            .rounded(native::SIDEBAR_RADIUS)
            .flex_row()
            .items_center()
            .gap(8.0)
            .bg_opt(selected.then_some(native::sidebar_selection(cx)))
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                if s.renaming.is_some_and(|r| r != id) {
                    s.finish_rename(true, cx);
                }
                if s.view != view || s.searching() {
                    s.set_view(view, cx);
                }
            })
            .on_double_click(move |s: &mut Self, cx: &mut Cx| {
                if let View::Folder(f) = view {
                    s.start_rename(f, cx);
                }
            })
            .on_right_click(move |s: &mut Self, cx: &mut Cx| {
                if let View::Folder(f) = view {
                    s.folder_menu(f, cx);
                }
            })
            .child(canvas(move |s, b| icon(s, b, accent)).size(native::SIDEBAR_ICON).shrink_0())
            .child(name_el)
            .child(text(count.to_string()).tabular().color(t.text_secondary).shrink_0())
    }

    fn list_column(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let notes = self.visible_notes();
        let (title, empty) = if self.searching() {
            ("Search Results".to_string(), "No Results")
        } else {
            match self.view {
                View::Folder(f) => (self.store.folder_name(f).to_string(), "No Notes"),
                View::Deleted => ("Recently Deleted".to_string(), "No Notes"),
            }
        };
        let count = match notes.len() {
            1 => "1 note".to_string(),
            n => format!("{n} notes"),
        };

        let mut toolbar = div()
            .h(native::TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(8.0)
            .pl(if self.sidebar_open { 22.0 } else { native::TRAFFIC_LIGHTS_X })
            .pr(10.0)
            .window_drag();
        if !self.sidebar_open {
            if let Some(buttons) = self.window_buttons(true, cx, t) {
                toolbar = toolbar.child(buttons.mr(8.0));
            }
            toolbar = toolbar.child(Self::icon_button("toggle-sidebar", true, cx, t, icons::sidebar, |s, _| s.sidebar_open = true));
        }
        toolbar = toolbar
            .child(
                div()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(text(title).weight(Weight::Bold).color(native::toolbar_ink(cx, t.text)).truncate())
                    .child(text(count).size(11.0).tabular().color(native::toolbar_ink(cx, t.text_secondary)).truncate()),
            )
            .child(Self::icon_button("delete-note", self.selected.is_some(), cx, t, icons::trash, |s: &mut Self, cx: &mut Cx| {
                if let Some(id) = s.selected {
                    s.delete_note(id, cx);
                }
            }));

        // Notes' yellow while the list has focus in the active window; gray otherwise.
        let list_focused = cx.is_focused("list") && cx.window_focused;
        let mut rows: Vec<AnyElement<Self>> = Vec::new();
        let mut last_section = String::new();
        let pins = !self.searching() && self.view != View::Deleted;
        for (i, n) in notes.iter().enumerate() {
            let sec = if pins && n.pinned { "Pinned".to_string() } else { section(n.modified) };
            if sec != last_section {
                // Section headers line up with the note titles below them.
                let header = div()
                    .flex_row()
                    .items_center()
                    .gap(4.0)
                    .px(12.0)
                    .pt(if rows.is_empty() { 4.0 } else { 16.0 })
                    .pb(6.0);
                let header = if sec == "Pinned" {
                    let c = t.text_secondary;
                    header.child(canvas(move |s, b| icons::pin(s, b, c)).size(14.0).shrink_0())
                } else {
                    header
                };
                rows.push(header.child(text(sec.clone()).size(12.0).weight(Weight::Bold).color(t.text).truncate()).into());
                last_section = sec;
            }
            let selected = self.selected == Some(n.id);
            let next_selected = notes.get(i + 1).is_some_and(|m| self.selected == Some(m.id));
            let next_same_section = notes.get(i + 1).is_some_and(|m| {
                let s = if pins && m.pinned { "Pinned".to_string() } else { section(m.modified) };
                s == last_section
            });
            rows.push(self.note_row(n, selected, list_focused, t).into());
            let show_sep = !selected && !next_selected && next_same_section;
            rows.push(
                div()
                    .h(1.0)
                    .mx_auto()
                    .w(LIST_W - 2.0 * 10.0 - 2.0 * 12.0)
                    .bg(if show_sep { t.row_divider } else { Color::TRANSPARENT })
                    .into(),
            );
        }
        if notes.is_empty() {
            // Centered in the column, as Notes shows "No Notes".
            rows.push(
                div()
                    .grow()
                    .min_h(120.0)
                    .pb(native::TOOLBAR_H)
                    .items_center()
                    .justify_center()
                    .child(text(empty).size(15.0).weight(Weight::Medium).color(t.text_tertiary))
                    .into(),
            );
        }

        div()
            .w(LIST_W)
            .h_full()
            .shrink_0()
            .flex_col()
            .bg(t.content)
            .child(toolbar)
            .child(
                div()
                    .id("list")
                    .focusable()
                    .flex_1()
                    .flex_col()
                    .child(div().id("list-scroll").scroll_y().flex_1().flex_col().px(10.0).pb(10.0).children(rows)),
            )
    }

    fn note_row(&self, n: &Note, selected: bool, focused: bool, t: &Theme) -> Div<Self> {
        let id = n.id;
        let (bg, title_c, date_c, preview_c) = match (selected, focused) {
            (true, true) => (Some(t.selection_focused), t.selection_text_focused, t.selection_text_focused, t.selection_text_focused.alpha(0.7)),
            (true, false) => (Some(t.selection_unfocused), t.text, t.text, t.text_secondary),
            _ => (None, t.text, t.text, t.text_secondary),
        };
        let mut second = div().flex_row().gap(6.0).child(text(list_date(n.modified)).size(12.0).tabular().color(date_c).shrink_0());
        second = second.child(text(n.preview().to_string()).size(12.0).color(preview_c).truncate().flex_1());
        let mut row = div()
            .id(("note", id))
            .flex_col()
            .gap(3.0)
            .px(12.0)
            .py(10.0)
            .rounded(8.0)
            .bg_opt(bg)
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.menu = None;
                s.select(Some(id), cx);
                cx.focus("list");
            })
            .on_right_click(move |s: &mut Self, cx: &mut Cx| s.note_menu(id, cx))
            .child(text(n.title().to_string()).weight(Weight::Bold).color(title_c).truncate());
        row = row.child(second);
        if self.searching() {
            let folder = self.store.folder_name(n.folder).to_string();
            let c = t.text_secondary;
            row = row.child(
                div()
                    .flex_row()
                    .gap(4.0)
                    .items_center()
                    .child(canvas(move |s, b| icons::folder(s, b, c)).size(13.0))
                    .child(text(folder).size(11.0).color(c).truncate()),
            );
        }
        row
    }

    fn editor_column(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let accent_icon = native::toolbar_ink(cx, t.icon);
        let query_set = !self.query.is_empty();
        let search = div()
            .id("search-box")
            .w(200.0)
            .h(28.0)
            .rounded(7.0)
            .bg(t.field)
            .focus_ring(cx.is_focused("search"), cx)
            .flex_row()
            .items_center()
            .gap(6.0)
            .px(8.0)
            .cursor(CursorIcon::Text)
            .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("search"))
            .child(canvas(move |s, b| icons::magnifier(s, b, accent_icon)).size(14.0).shrink_0())
            .child(input("search", &self.search).flex_1())
            .child_if(query_set, || {
                let c = t.text_secondary;
                let pressed = cx.hovered("search-clear") && cx.mouse_down;
                div()
                    .id("search-clear")
                    .size(16.0)
                    .rounded(8.0)
                    .bg(c.alpha(if pressed { 0.8 } else { 0.5 }))
                    .shrink_0()
                    .on_click(|s: &mut Self, cx: &mut Cx| {
                        s.clear_search(cx);
                        cx.focus("list");
                    })
                    .child(canvas(|s, b| icons::xmark(s, b, Color::WHITE, 6.0, 1.4)).size(16.0))
                    .into()
            });

        let toolbar = div()
            .h(native::TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .justify_between()
            .px(10.0)
            .window_drag()
            .child(Self::icon_button("compose", true, cx, t, icons::compose, |s: &mut Self, cx: &mut Cx| s.new_note(cx)))
            .child(div().flex_row().items_center().gap(12.0).child(search).children(self.window_buttons(false, cx, t)));

        let note = self.selected.and_then(|id| self.store.note(id));
        let body: AnyElement<Self> = match note {
            Some(n) => div()
                .id("editor-scroll")
                .scroll_y()
                .flex_1()
                .flex_col()
                .child(
                    text(full_date(n.modified))
                        .size(12.0)
                        .color(t.text_secondary)
                        .mx_auto()
                        .pt(2.0)
                        .pb(14.0),
                )
                .child(
                    div()
                        .id("editor-area")
                        .flex_col()
                        .grow()
                        .px(28.0)
                        .pb(120.0)
                        .cursor(CursorIcon::Text)
                        .on_mouse_down(|s: &mut Self, cx: &mut Cx| {
                            s.editor.borrow_mut().move_to_end();
                            cx.focus("editor");
                        })
                        .child(input("editor", &self.editor).w_full()),
                )
                .into(),
            None => div().flex_1().into(),
        };

        div().flex_1().h_full().flex_col().bg(t.content).child(toolbar).child(body)
    }
}

impl PairingHost for NotesApp {
    fn pairing(&mut self) -> &mut Pairing {
        &mut self.pairing
    }
}

impl App for NotesApp {
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Self::theme(cx).text }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        let t = Self::theme(cx);
        self.sync_input_styles(&t);
        let (w, h) = cx.window_size;
        let divider = || div().w(1.0).h_full().shrink_0().bg(t.divider);

        let mut root = div().w(w).h(h).flex_row().bg(t.content).text_color(t.text);
        if self.sidebar_open {
            root = root.child(self.sidebar(cx, &t)).child(divider());
        }
        root = root.child(self.list_column(cx, &t)).child(divider()).child(self.editor_column(cx, &t));
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
        let focused_list = cx.is_focused("list");
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
            if self.renaming.is_some() {
                self.finish_rename(false, cx);
            } else if cx.is_focused("search") {
                self.clear_search(cx);
                cx.focus("list");
            } else {
                cx.focus("list");
            }
            return true;
        }
        if key.ctrl && !key.alt {
            if key.is_char('n') && key.shift {
                self.new_folder(cx);
                return true;
            }
            if key.is_char('n') {
                self.new_note(cx);
                return true;
            }
            if key.is_char('f') {
                self.search.borrow_mut().select_all();
                cx.focus("search");
                return true;
            }
            if key.is_char('q') || key.is_char('w') {
                self.save(cx);
                cx.close_window();
                return true;
            }
            if key.is_char('s') {
                self.save(cx);
                return true;
            }
        }
        if key.is_named(NamedKey::Enter) {
            if self.renaming.is_some() {
                self.finish_rename(true, cx);
                return true;
            }
            if focused_list && self.selected.is_some() {
                self.editor.borrow_mut().move_to_end();
                cx.focus("editor");
                return true;
            }
            if cx.is_focused("search") {
                cx.focus("list");
                return true;
            }
        }
        if focused_list {
            if key.is_named(NamedKey::ArrowDown) {
                self.move_selection(1, cx);
                return true;
            }
            if key.is_named(NamedKey::ArrowUp) {
                self.move_selection(-1, cx);
                return true;
            }
            if key.is_named(NamedKey::Delete) || key.is_named(NamedKey::Backspace) {
                if let Some(id) = self.selected {
                    self.delete_note(id, cx);
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
        if id == eid("editor") {
            let text = self.editor.borrow().text();
            if let Some(n) = self.editor_note.and_then(|nid| self.store.note_mut(nid)) {
                if n.body != text {
                    n.body = text;
                    n.modified = now();
                    self.mark_dirty();
                }
            }
        } else if id == eid("search") {
            self.query = self.search.borrow().text();
            cx.scroll_to_top("list-scroll");
            self.ensure_selection(cx);
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
        if self.events.as_ref().is_some_and(|e| e.take_changed(NOTES_DOC)) {
            // Merge any unsaved typing first, then show the combined result.
            self.dirty_since = None;
            self.store.save();
            self.after_reload(cx);
        }
    }

    fn on_exit(&mut self) {
        if self.selected.and_then(|id| self.store.note(id)).is_some_and(|n| n.is_blank()) {
            let id = self.selected.unwrap();
            self.store.notes.retain(|n| n.id != id);
        }
        self.store.save();
        if let Some(core) = &self.store.core {
            core.sync().shutdown();
        }
    }
}

/// Sample content for `--snapshot` renders.
pub fn demo_store() -> Store {
    let mut s = Store::detached();
    s.folders.push(Folder { id: 900, name: "Work".into() });
    s.folders.push(Folder { id: 901, name: "Recipes".into() });
    s.folders.push(Folder { id: 902, name: "Travel".into() });
    let t = now();
    let mut add = |folder: u64, body: &str, age_h: i64, pinned: bool| {
        let id = s.alloc_id();
        s.notes.push(Note {
            id,
            folder,
            body: body.into(),
            created: t - age_h * 3600,
            modified: t - age_h * 3600,
            pinned,
            deleted_at: None,
        });
    };
    add(DEFAULT_FOLDER, "Grocery List\nOat milk, eggs, spinach, sourdough\nCoffee beans\nLemons", 0, true);
    add(
        DEFAULT_FOLDER,
        "Weekend Plans\nHike the ridge trail Saturday morning, then brunch at the corner café.\n\nSunday: farmers market, call Mom, finish reading Piranesi.",
        1,
        false,
    );
    add(DEFAULT_FOLDER, "Book recommendations\nThe Remains of the Day\nA Psalm for the Wild-Built", 3, false);
    add(DEFAULT_FOLDER, "Ideas for the app\nKeyboard shortcuts everywhere\nSync with phone?", 30, false);
    add(DEFAULT_FOLDER, "Meeting notes — Q4 planning\nShip the redesign before the holidays", 80, false);
    add(DEFAULT_FOLDER, "Packing list\nPassport, chargers, adapters", 24 * 12, false);
    add(DEFAULT_FOLDER, "Workout routine\nMon: push · Wed: pull · Fri: legs", 24 * 50, false);
    add(900, "Standup\nYesterday: renderer", 2, false);
    add(901, "Pasta al limone\nZest two lemons", 40, false);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(app: &mut NotesApp, cx: &mut Cx, s: &str) {
        for ch in s.chars() {
            let key = if ch == '\n' { Key::Named(NamedKey::Enter) } else { Key::Character(ch.to_string().into()) };
            let kp = KeyPress { key, text: Some(ch.to_string()), ctrl: false, shift: false, alt: false };
            let out = app.editor.borrow_mut().handle_key(&mut cx.text.fonts, &kp, &mut None);
            if out.changed {
                app.on_input(eid("editor"), cx);
            }
        }
    }

    #[test]
    fn new_note_typing_and_menu() {
        let path = std::env::temp_dir().join("notes-selftest.png");
        let path = std::env::var_os("NOTES_TEST_PNG").map(std::path::PathBuf::from).unwrap_or(path);
        ui::snapshot((1180.0, 740.0), 2.0, false, &path, |cx| NotesApp::new(demo_store(), None, cx), |app, cx| {
            let before = app.store.count_in(DEFAULT_FOLDER);
            app.new_note(cx);
            assert!(cx.is_focused("editor"));
            type_text(app, cx, "Hello from Rust\nThis note was typed by a test.");
            let id = app.selected.unwrap();
            let note = app.store.note(id).unwrap();
            assert_eq!(note.title(), "Hello from Rust");
            assert_eq!(note.preview(), "This note was typed by a test.");
            assert_eq!(app.store.count_in(DEFAULT_FOLDER), before + 1);

            // Undo removes the last typing run.
            let undo = KeyPress { key: Key::Character("z".into()), text: None, ctrl: true, shift: false, alt: false };
            app.editor.borrow_mut().handle_key(&mut cx.text.fonts, &undo, &mut None);
            assert_ne!(app.editor.borrow().text(), "Hello from Rust\nThis note was typed by a test.");
            let redo = KeyPress { shift: true, ..undo };
            app.editor.borrow_mut().handle_key(&mut cx.text.fonts, &redo, &mut None);
            assert_eq!(app.editor.borrow().text(), "Hello from Rust\nThis note was typed by a test.");

            // An empty new note is discarded when moving away from it.
            app.new_note(cx);
            assert_ne!(app.selected, Some(id));
            let blank = app.selected.unwrap();
            app.select(Some(id), cx);
            assert!(app.store.note(blank).is_none());

            // Delete moves to Recently Deleted; recover brings it back.
            app.delete_note(id, cx);
            assert!(app.store.note(id).unwrap().deleted_at.is_some());
            app.recover_note(id, cx);
            assert!(app.store.note(id).unwrap().deleted_at.is_none());

            // Search filters across folders.
            app.search.borrow_mut().set_text(&mut cx.text.fonts, "lemon");
            app.on_input(eid("search"), cx);
            let mut hits: Vec<String> = app.visible_notes().iter().map(|n| n.title().to_string()).collect();
            hits.sort();
            assert_eq!(hits, ["Grocery List", "Pasta al limone"]);
            app.clear_search(cx);

            // The context menu works from the keyboard: Down + Return pins,
            // Escape closes, typing a letter jumps to that row.
            let key = |k: NamedKey| KeyPress { key: Key::Named(k), text: None, ctrl: false, shift: false, alt: false };
            app.select(Some(id), cx);
            cx.mouse = Point { x: 520.0, y: 200.0 };
            app.note_menu(id, cx);
            assert!(app.on_key(&key(NamedKey::ArrowDown), cx));
            assert!(app.on_key(&key(NamedKey::Enter), cx));
            assert!(app.menu.is_none());
            assert!(app.store.note(id).unwrap().pinned);
            app.note_menu(id, cx);
            assert!(app.on_key(&key(NamedKey::Escape), cx));
            assert!(app.menu.is_none() && app.store.note(id).unwrap().pinned);
            app.note_menu(id, cx);
            let d = KeyPress { key: Key::Character("d".into()), text: Some("d".into()), ..key(NamedKey::Enter) };
            app.on_key(&d, cx);
            app.on_key(&key(NamedKey::Enter), cx);
            assert!(app.store.note(id).unwrap().deleted_at.is_some());
            app.recover_note(id, cx);
            app.note_menu(id, cx);
            cx.mouse = Point { x: 560.0, y: 256.0 };
        });
        assert!(path.exists());
    }
}
