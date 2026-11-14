mod icons;
pub mod model;
mod theme;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::ui::*;
use model::{DEFAULT_FOLDER, Folder, Note, Store, full_date, list_date, now, section};
use theme::Theme;

const SIDEBAR_W: f32 = 220.0;
const LIST_W: f32 = 300.0;
const TOOLBAR_H: f32 = 48.0;
const SAVE_DELAY: Duration = Duration::from_millis(600);
const PURGE_AFTER_SECS: i64 = 30 * 24 * 3600;

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Folder(u64),
    Deleted,
}

#[derive(Clone)]
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

struct MenuItem {
    label: String,
    /// `None` draws a separator.
    action: Option<MenuAction>,
}

struct Menu {
    pos: Point,
    items: Vec<MenuItem>,
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
    menu: Option<Menu>,
    dirty_since: Option<Instant>,
    /// Window buttons for the left and right ends of the header, from KDE settings.
    buttons: (Vec<WindowButton>, Vec<WindowButton>),
}

impl NotesApp {
    pub fn new(mut store: Store, cx: &mut Cx) -> Self {
        let cutoff = now() - PURGE_AFTER_SECS;
        store.notes.retain(|n| n.deleted_at.is_none_or(|t| t > cutoff));

        let fonts = &mut cx.text.fonts;
        let mut editor = TextInput::new(fonts, TextStyle::default(), false);
        editor.first_line_style = Some(TextStyle::default());
        let mut search = TextInput::new(fonts, TextStyle::default(), true);
        search.placeholder = "Search".into();
        let rename = TextInput::new(fonts, TextStyle::default(), true);
        cx.window_radius = 8.0;

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
            buttons: window_button_layout(),
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

    fn save(&mut self) {
        if self.dirty_since.take().is_some() {
            if let Err(e) = self.store.save() {
                eprintln!("notes: failed to save: {e}");
            }
        }
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
        let item = |label: &str, a: MenuAction| MenuItem { label: label.into(), action: Some(a) };
        let sep = || MenuItem { label: String::new(), action: None };
        let items = if note.deleted_at.is_some() {
            vec![item("Recover", MenuAction::Recover(id)), sep(), item("Delete Permanently", MenuAction::DeleteNote(id))]
        } else {
            let mut items = vec![
                item(if note.pinned { "Unpin Note" } else { "Pin Note" }, MenuAction::Pin(id, !note.pinned)),
                sep(),
            ];
            for f in self.store.folders.iter().filter(|f| f.id != note.folder) {
                items.push(item(&format!("Move to \u{201c}{}\u{201d}", f.name), MenuAction::Move(id, f.id)));
            }
            if self.store.folders.len() > 1 {
                items.push(sep());
            }
            items.push(item("Delete", MenuAction::DeleteNote(id)));
            items
        };
        self.menu = Some(Menu { pos: cx.mouse, items });
    }

    fn folder_menu(&mut self, id: u64, cx: &mut Cx) {
        let item = |label: &str, a: MenuAction| MenuItem { label: label.into(), action: Some(a) };
        let mut items = vec![item("New Note", MenuAction::NewNote), item("New Folder", MenuAction::NewFolder)];
        if id != DEFAULT_FOLDER {
            items.push(MenuItem { label: String::new(), action: None });
            items.push(item("Rename Folder", MenuAction::RenameFolder(id)));
            items.push(item("Delete Folder", MenuAction::DeleteFolder(id)));
        }
        self.menu = Some(Menu { pos: cx.mouse, items });
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

    /// Breeze-style window buttons; empty when that side has none.
    fn window_buttons(&self, left: bool, cx: &Cx, t: &Theme) -> Option<Div<Self>> {
        let buttons = if left { &self.buttons.0 } else { &self.buttons.1 };
        if buttons.is_empty() {
            return None;
        }
        let mut row = div().flex_row().items_center().gap(6.0);
        for &kind in buttons {
            let hovered = cx.hovered(("window-button", kind));
            let (bg, fg) = match (kind, hovered) {
                (WindowButton::Close, true) => (Some(rgb(0xda4453)), Color::WHITE),
                (_, true) => (Some(t.text.alpha(0.15)), t.text),
                _ => (None, t.text),
            };
            let glyph: fn(&mut Scene, Bounds, Color) = match kind {
                WindowButton::Close => icons::breeze_close,
                WindowButton::Minimize => icons::breeze_minimize,
                WindowButton::Maximize if cx.maximized => icons::breeze_restore,
                WindowButton::Maximize => icons::breeze_maximize,
            };
            row = row.child(
                div()
                    .id(("window-button", kind))
                    .size(20.0)
                    .rounded(10.0)
                    .bg_opt(bg)
                    .on_click(move |_: &mut Self, cx: &mut Cx| match kind {
                        WindowButton::Close => cx.close_window(),
                        WindowButton::Minimize => cx.minimize_window(),
                        WindowButton::Maximize => cx.toggle_maximize(),
                    })
                    .child(canvas(move |s, b| glyph(s, b, fg)).size(20.0)),
            );
        }
        Some(row)
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
            .h(26.0)
            .rounded(6.0)
            .items_center()
            .justify_center()
            .hover_bg(t.icon_hover_bg)
            .on_click(on_click)
            .child(canvas(move |s, b| icon(s, b, color)).size(20.0))
    }

    fn sidebar(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut header = div()
            .h(TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .justify_between()
            .px(12.0)
            .window_drag();
        if let Some(buttons) = self.window_buttons(true, cx, t) {
            header = header.child(buttons);
        }
        let header = header.child(Self::icon_button("toggle-sidebar", t, icons::sidebar, |s, _| s.sidebar_open = false));

        let mut rows: Vec<AnyElement<Self>> = vec![
            text("On My Computer")
                .size(11.0)
                .weight(Weight::Semibold)
                .color(t.text_secondary)
                .pl(10.0)
                .pt(6.0)
                .pb(6.0)
                .into(),
        ];
        for f in &self.store.folders {
            rows.push(self.folder_row(f.id, &f.name, self.store.count_in(f.id), icons::folder, View::Folder(f.id), t).into());
        }
        let deleted = self.store.deleted_count();
        if deleted > 0 || self.view == View::Deleted {
            rows.push(self.folder_row(u64::MAX, "Recently Deleted", deleted, icons::trash, View::Deleted, t).into());
        }

        let accent = t.accent;
        let new_folder = div()
            .id("new-folder")
            .h(44.0)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(6.0)
            .px(16.0)
            .cursor(CursorIcon::Pointer)
            .on_click(|s: &mut Self, cx: &mut Cx| s.new_folder(cx))
            .child(canvas(move |s, b| icons::plus_circle(s, b, accent)).size(18.0))
            .child(text("New Folder").color(t.text_secondary));

        div()
            .w(SIDEBAR_W)
            .h_full()
            .shrink_0()
            .flex_col()
            .bg(t.sidebar)
            .child(header)
            .child(div().id("sidebar-scroll").scroll_y().flex_1().flex_col().px(10.0).gap(1.0).children(rows))
            .child(new_folder)
    }

    fn folder_row(
        &self,
        id: u64,
        name: &str,
        count: usize,
        icon: fn(&mut Scene, Bounds, Color),
        view: View,
        t: &Theme,
    ) -> Div<Self> {
        let selected = self.view == view && !self.searching();
        let accent = t.accent;
        let name_el: AnyElement<Self> = if self.renaming == Some(id) {
            div()
                .flex_1()
                .h(20.0)
                .px(4.0)
                .items_center()
                .rounded(4.0)
                .bg(t.content)
                .border(t.accent, 1.5)
                .child(input("rename", &self.rename).flex_1())
                .into()
        } else {
            text(name.to_string()).truncate().flex_1().into()
        };
        div()
            .id(("folder", id))
            .h(28.0)
            .shrink_0()
            .px(8.0)
            .rounded(6.0)
            .flex_row()
            .items_center()
            .gap(7.0)
            .bg_opt(selected.then_some(t.sidebar_selection))
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
            .child(canvas(move |s, b| icon(s, b, accent)).size(18.0).shrink_0())
            .child(name_el)
            .child(text(count.to_string()).color(t.text_secondary).shrink_0())
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
            .h(TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(8.0)
            .pl(if self.sidebar_open { 18.0 } else { 12.0 })
            .pr(8.0)
            .window_drag();
        if !self.sidebar_open {
            if let Some(buttons) = self.window_buttons(true, cx, t) {
                toolbar = toolbar.child(buttons.mr(6.0));
            }
            toolbar = toolbar.child(Self::icon_button("toggle-sidebar", t, icons::sidebar, |s, _| s.sidebar_open = true));
        }
        toolbar = toolbar
            .child(
                div()
                    .flex_col()
                    .flex_1()
                    .child(text(title).weight(Weight::Bold).truncate())
                    .child(text(count).size(11.0).color(t.text_secondary).truncate()),
            )
            .child(Self::icon_button("delete-note", t, icons::trash, |s: &mut Self, cx: &mut Cx| {
                if let Some(id) = s.selected {
                    s.delete_note(id, cx);
                }
            }));

        let list_focused = cx.is_focused("list");
        let mut rows: Vec<AnyElement<Self>> = Vec::new();
        let mut last_section = String::new();
        let pins = !self.searching() && self.view != View::Deleted;
        for (i, n) in notes.iter().enumerate() {
            let sec = if pins && n.pinned { "Pinned".to_string() } else { section(n.modified) };
            if sec != last_section {
                let header = div()
                    .flex_row()
                    .items_center()
                    .gap(4.0)
                    .px(10.0)
                    .pt(if rows.is_empty() { 4.0 } else { 16.0 })
                    .pb(6.0);
                let header = if sec == "Pinned" {
                    let c = t.text_secondary;
                    header.child(canvas(move |s, b| icons::pin(s, b, c)).size(12.0))
                } else {
                    header
                };
                rows.push(header.child(text(sec.clone()).size(12.0).weight(Weight::Bold).color(t.text)).into());
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
            rows.push(
                div()
                    .h(200.0)
                    .items_center()
                    .justify_center()
                    .child(text(empty).size(14.0).color(t.text_tertiary))
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
        let mut second = div().flex_row().gap(6.0).child(text(list_date(n.modified)).size(12.0).color(date_c).shrink_0());
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
        let accent_icon = t.icon;
        let query_set = !self.query.is_empty();
        let search = div()
            .id("search-box")
            .w(200.0)
            .h(28.0)
            .rounded(7.0)
            .bg(t.field)
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
                div()
                    .id("search-clear")
                    .size(16.0)
                    .rounded(8.0)
                    .bg(c.alpha(0.5))
                    .shrink_0()
                    .on_click(|s: &mut Self, cx: &mut Cx| {
                        s.clear_search(cx);
                        cx.focus("list");
                    })
                    .child(canvas(|s, b| icons::xmark(s, b, Color::WHITE, 6.0, 1.4)).size(16.0))
                    .into()
            });

        let toolbar = div()
            .h(TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .justify_between()
            .px(10.0)
            .window_drag()
            .child(Self::icon_button("compose", t, icons::compose, |s: &mut Self, cx: &mut Cx| s.new_note(cx)))
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

    fn menu_overlay(&self, menu: &Menu, cx: &Cx, t: &Theme) -> Div<Self> {
        let (w, h) = cx.window_size;
        let width = 230.0;
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
            match &item.action {
                None => panel = panel.child(div().h(1.0).mt(4.0).mb(4.0).px(6.0).child(div().flex_1().h(1.0).bg(t.menu_border))),
                Some(action) => {
                    let hovered = cx.hovered(("menu-item", i));
                    let action = action.clone();
                    panel = panel.child(
                        div()
                            .id(("menu-item", i))
                            .h(24.0)
                            .px(10.0)
                            .rounded(5.0)
                            .items_center()
                            .hover_bg(t.menu_highlight)
                            .on_click(move |s: &mut Self, cx: &mut Cx| s.run_menu(action.clone(), cx))
                            .child(text(item.label.clone()).color(if hovered { Color::WHITE } else { t.text }).truncate()),
                    );
                }
            }
        }
        div()
            .inset_0()
            .deferred()
            .on_mouse_down(|s: &mut Self, _| s.menu = None)
            .on_right_click(|s: &mut Self, _| s.menu = None)
            .child(panel)
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
            root = root.child(self.menu_overlay(menu, cx, &t));
        }
        // Frameless windows get no compositor outline, so draw a hairline edge.
        if cx.transparent && !cx.maximized {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(t.window_border, 1.0));
        }
        root
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        let focused_list = cx.is_focused("list");
        if key.is_named(NamedKey::Escape) {
            if self.menu.take().is_some() {
            } else if self.renaming.is_some() {
                self.finish_rename(false, cx);
            } else if cx.is_focused("search") {
                self.clear_search(cx);
                cx.focus("list");
            } else {
                cx.focus("list");
            }
            return true;
        }
        if self.menu.is_some() {
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
                self.save();
                cx.close_window();
                return true;
            }
            if key.is_char('s') {
                self.save();
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
        self.dirty_since.map(|t| t + SAVE_DELAY)
    }

    fn on_wake(&mut self, _cx: &mut Cx) {
        if self.dirty_since.is_some_and(|t| t.elapsed() >= SAVE_DELAY) {
            self.save();
        }
    }

    fn on_exit(&mut self) {
        if self.selected.and_then(|id| self.store.note(id)).is_some_and(|n| n.is_blank()) {
            let id = self.selected.unwrap();
            self.store.notes.retain(|n| n.id != id);
            self.mark_dirty();
        }
        self.save();
    }
}

/// Sample content for `--snapshot` renders.
pub fn demo_store() -> Store {
    let mut s = Store::load(std::path::PathBuf::from("/nonexistent/notes.json"));
    s.path = None;
    s.notes.clear();
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
        crate::ui::snapshot((1180.0, 740.0), 2.0, false, &path, |cx| NotesApp::new(demo_store(), cx), |app, cx| {
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

            app.select(Some(id), cx);
            cx.mouse = Point { x: 520.0, y: 200.0 };
            app.note_menu(id, cx);
            cx.mouse = Point { x: 560.0, y: 256.0 };
        });
        assert!(path.exists());
    }
}
