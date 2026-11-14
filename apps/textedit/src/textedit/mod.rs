pub mod files;
mod icons;
mod theme;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use files::Suggestion;
use theme::Theme;
use ui::*;

const TOOLBAR_H: f32 = 52.0;
const FIND_ROW_H: f32 = 34.0;
const STATUS_H: f32 = 24.0;
const SHEET_W: f32 = 460.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Sheet {
    /// `then_close`: the window closes once saved (from the unsaved-changes alert).
    Save { then_close: bool },
    Open,
    /// "Do you want to save the changes…?" before closing.
    Unsaved,
}

pub struct TextEditApp {
    doc: Rc<RefCell<TextView>>,
    path: Option<PathBuf>,
    crlf: bool,
    /// `TextView::state_id` when last saved or loaded.
    saved_state: u64,
    find_open: bool,
    replace_open: bool,
    case_sensitive: bool,
    find: Rc<RefCell<TextInput>>,
    replace: Rc<RefCell<TextInput>>,
    sheet: Option<Sheet>,
    path_field: Rc<RefCell<TextInput>>,
    suggestions: Vec<Suggestion>,
    sheet_error: Option<String>,
    /// The chosen file exists; saving again replaces it.
    confirm_replace: bool,
    /// The user chose to close; don't ask again.
    closing: bool,
    /// Why the file from the command line couldn't be read.
    load_error: Option<String>,
}

impl TextEditApp {
    pub fn new(path: Option<PathBuf>, cx: &mut Cx) -> Self {
        let fonts = &mut cx.text.fonts;
        let doc = TextView::new(fonts, TextStyle::default());
        let mut find = TextInput::new(fonts, TextStyle::default(), true);
        find.placeholder = "Find".into();
        let mut replace = TextInput::new(fonts, TextStyle::default(), true);
        replace.placeholder = "Replace".into();
        let path_field = TextInput::new(fonts, TextStyle::default(), true);
        cx.window_radius = 10.0;
        let mut app = Self {
            doc: Rc::new(RefCell::new(doc)),
            path: None,
            crlf: false,
            saved_state: 0,
            find_open: false,
            replace_open: false,
            case_sensitive: false,
            find: Rc::new(RefCell::new(find)),
            replace: Rc::new(RefCell::new(replace)),
            sheet: None,
            path_field: Rc::new(RefCell::new(path_field)),
            suggestions: Vec::new(),
            sheet_error: None,
            confirm_replace: false,
            closing: false,
            load_error: None,
        };
        if let Some(path) = path {
            match app.load(&path) {
                Ok(()) => {}
                // A new file: it's created on the first save.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => app.path = Some(path),
                Err(e) => app.load_error = Some(format!("Couldn\u{2019}t open \u{201c}{}\u{201d}: {e}", path.display())),
            }
        }
        cx.focus("doc");
        app
    }

    fn theme(cx: &Cx) -> Theme {
        if cx.dark { Theme::dark() } else { Theme::light() }
    }

    pub fn doc(&self) -> &Rc<RefCell<TextView>> {
        &self.doc
    }

    pub fn is_edited(&self) -> bool {
        self.doc.borrow().state_id() != self.saved_state
    }

    fn name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map_or("Untitled".into(), |n| n.to_string_lossy().into_owned())
    }

    // -- files --------------------------------------------------------------

    fn load(&mut self, path: &Path) -> std::io::Result<()> {
        let loaded = files::load(path)?;
        let mut doc = self.doc.borrow_mut();
        doc.set_text(&loaded.text);
        self.saved_state = doc.state_id();
        self.crlf = loaded.crlf;
        self.path = Some(path.to_path_buf());
        self.load_error = None;
        Ok(())
    }

    fn write(&mut self, path: &Path) -> Result<(), String> {
        let text = self.doc.borrow().text();
        files::save(path, &text, self.crlf).map_err(|e| format!("Couldn\u{2019}t save: {e}"))?;
        self.path = Some(path.to_path_buf());
        self.saved_state = self.doc.borrow().state_id();
        Ok(())
    }

    pub fn save(&mut self, cx: &mut Cx) {
        match self.path.clone() {
            Some(path) => {
                if let Err(e) = self.write(&path) {
                    self.load_error = Some(e);
                }
            }
            None => self.begin_save_as(false, cx),
        }
    }

    pub fn begin_save_as(&mut self, then_close: bool, cx: &mut Cx) {
        let typed = match &self.path {
            Some(p) => files::contract(p),
            None => files::default_save_path("Untitled.txt"),
        };
        self.open_sheet(Sheet::Save { then_close }, &typed, cx);
    }

    pub fn begin_open(&mut self, cx: &mut Cx) {
        let dir = self.path.as_ref().and_then(|p| p.parent()).map_or("~".into(), files::contract);
        self.open_sheet(Sheet::Open, &format!("{dir}/"), cx);
    }

    fn open_sheet(&mut self, sheet: Sheet, typed: &str, cx: &mut Cx) {
        self.sheet = Some(sheet);
        let mut field = self.path_field.borrow_mut();
        field.set_text(&mut cx.text.fonts, typed);
        field.move_to_end();
        drop(field);
        self.sheet_error = None;
        self.confirm_replace = false;
        self.refresh_suggestions();
        cx.focus("path");
    }

    fn close_sheet(&mut self, cx: &mut Cx) {
        self.sheet = None;
        self.sheet_error = None;
        cx.focus("doc");
    }

    fn refresh_suggestions(&mut self) {
        let typed = self.path_field.borrow().text();
        let folders_only = matches!(self.sheet, Some(Sheet::Save { .. }));
        self.suggestions = files::suggestions(&typed, folders_only);
    }

    fn pick_suggestion(&mut self, i: usize, cx: &mut Cx) {
        let Some(s) = self.suggestions.get(i).cloned() else { return };
        if !s.is_dir && self.sheet == Some(Sheet::Open) {
            self.open_path(files::expand(&s.completion), cx);
            return;
        }
        let mut field = self.path_field.borrow_mut();
        field.set_text(&mut cx.text.fonts, &s.completion);
        field.move_to_end();
        drop(field);
        self.sheet_error = None;
        self.confirm_replace = false;
        self.refresh_suggestions();
        cx.focus("path");
    }

    /// The sheet's default button (Save, Open, or Save in the alert).
    fn confirm_sheet(&mut self, cx: &mut Cx) {
        let typed = self.path_field.borrow().text();
        let path = files::expand(&typed);
        match self.sheet {
            Some(Sheet::Save { then_close }) => {
                if typed.trim().is_empty() || typed.ends_with('/') || path.is_dir() {
                    self.sheet_error = Some("Type a name for the document.".into());
                    return;
                }
                if path.exists() && !self.confirm_replace && self.path.as_ref() != Some(&path) {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.sheet_error = Some(format!("\u{201c}{name}\u{201d} already exists. Click Replace to overwrite it."));
                    self.confirm_replace = true;
                    return;
                }
                match self.write(&path) {
                    Ok(()) => {
                        self.close_sheet(cx);
                        if then_close {
                            self.closing = true;
                            cx.close_window();
                        }
                    }
                    Err(e) => self.sheet_error = Some(e),
                }
            }
            Some(Sheet::Open) => {
                if path.is_dir() {
                    let typed = if typed.ends_with('/') { typed } else { format!("{typed}/") };
                    self.open_sheet(Sheet::Open, &typed, cx);
                } else if !path.exists() {
                    self.sheet_error = Some("There\u{2019}s no file at that path.".into());
                } else {
                    self.open_path(path, cx);
                }
            }
            Some(Sheet::Unsaved) => {
                self.sheet = None;
                if self.path.is_some() {
                    self.save(cx);
                    if !self.is_edited() {
                        self.closing = true;
                        cx.close_window();
                    }
                } else {
                    self.begin_save_as(true, cx);
                }
            }
            None => {}
        }
    }

    fn discard_and_close(&mut self, cx: &mut Cx) {
        self.sheet = None;
        self.closing = true;
        cx.close_window();
    }

    /// Opens here if this window is an untouched new document, else in a new window.
    fn open_path(&mut self, path: PathBuf, cx: &mut Cx) {
        let untouched = self.path.is_none() && !self.is_edited() && self.doc.borrow().is_empty();
        if untouched {
            if let Err(e) = self.load(&path) {
                self.sheet_error = Some(format!("Couldn\u{2019}t open: {e}"));
                return;
            }
        } else {
            spawn_window(Some(&path));
        }
        self.close_sheet(cx);
    }

    // -- find ---------------------------------------------------------------

    pub fn open_find(&mut self, replace: bool, cx: &mut Cx) {
        self.find_open = true;
        self.replace_open |= replace;
        self.find.borrow_mut().select_all();
        cx.focus(if replace && !self.find.borrow().is_empty() { "replace" } else { "find" });
        self.update_query();
    }

    fn close_find(&mut self, cx: &mut Cx) {
        self.find_open = false;
        self.replace_open = false;
        self.doc.borrow_mut().set_query("", false);
        cx.focus("doc");
    }

    fn update_query(&mut self) {
        let q = self.find.borrow().text();
        let mut doc = self.doc.borrow_mut();
        doc.set_query(&q, self.case_sensitive);
        if !q.is_empty() {
            doc.find_from_selection();
        }
    }

    /// For previews: searches for `q`.
    pub fn set_find_text(&mut self, q: &str, cx: &mut Cx) {
        self.find.borrow_mut().set_text(&mut cx.text.fonts, q);
        self.update_query();
    }

    /// Ctrl+E: search for the selected text.
    fn use_selection_for_find(&mut self, cx: &mut Cx) {
        let Some(sel) = self.doc.borrow().selected_text().filter(|s| !s.contains('\n')) else { return };
        self.find.borrow_mut().set_text(&mut cx.text.fonts, &sel);
        self.find_open = true;
        self.update_query();
    }

    fn find_next(&mut self, forward: bool) {
        self.doc.borrow_mut().find_next(forward);
    }

    fn replace_one(&mut self) {
        let with = self.replace.borrow().text();
        self.doc.borrow_mut().replace_current(&with);
    }

    fn replace_all(&mut self) {
        let with = self.replace.borrow().text();
        self.doc.borrow_mut().replace_all(&with);
    }

    // -- rendering ----------------------------------------------------------

    fn sync_styles(&self, t: &Theme) {
        let mut doc = self.doc.borrow_mut();
        doc.set_style(TextStyle { size: 13.0, line_height: 1.4, weight: Weight::Regular, color: t.text });
        doc.caret_color = t.text;
        doc.selection_color = t.selection;
        doc.inactive_selection_color = t.selection_inactive;
        doc.match_color = t.find_match;
        doc.current_match_color = t.find_current;
        doc.scrollbar_color = t.scrollbar;
        doc.inset = (10.0, 8.0);
        for input in [&self.find, &self.replace, &self.path_field] {
            let mut i = input.borrow_mut();
            i.set_styles(TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: t.text }, None);
            i.placeholder_color = t.text_tertiary;
            i.caret_color = t.text;
            i.selection_color = t.selection;
        }
    }

    fn icon_button(
        id: &'static str,
        t: &Theme,
        on: bool,
        icon: fn(&mut Scene, Bounds, Color),
        on_click: impl Fn(&mut Self, &mut Cx) + 'static,
    ) -> Div<Self> {
        let color = if on { t.accent } else { t.icon };
        div()
            .id(id)
            .w(32.0)
            .h(28.0)
            .rounded(6.0)
            .items_center()
            .justify_center()
            .bg_opt(on.then_some(t.toggled))
            .hover_bg(if on { t.toggled } else { t.hover })
            .on_click(on_click)
            .child(canvas(move |s, b| icon(s, b, color)).size(20.0))
    }

    fn toolbar(&self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let mono = self.doc.borrow().is_monospace();
        let wraps = self.doc.borrow().wraps();
        let mut left = div().flex_1().flex_row().items_center();
        if let Some(buttons) = ui::chrome::window_buttons(true, cx, t.text) {
            left = left.child(buttons);
        }
        let mono_color = if mono { t.accent } else { t.icon };
        let font_button = div()
            .id("mono")
            .w(32.0)
            .h(28.0)
            .rounded(6.0)
            .items_center()
            .justify_center()
            .bg_opt(mono.then_some(t.toggled))
            .hover_bg(if mono { t.toggled } else { t.hover })
            .on_click(|s: &mut Self, cx: &mut Cx| {
                let on = !s.doc.borrow().is_monospace();
                s.doc.borrow_mut().set_monospace(&cx.text.fonts, on);
            })
            .child(text("Aa").size(14.0).weight(Weight::Medium).color(mono_color));
        let mut right = div()
            .flex_1()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(2.0)
            .child(font_button)
            .child(Self::icon_button("wrap", t, wraps, icons::wrap, |s: &mut Self, _| {
                let on = !s.doc.borrow().wraps();
                s.doc.borrow_mut().set_wrap(on);
            }))
            .child(Self::icon_button("find-button", t, self.find_open, icons::magnifier, |s: &mut Self, cx: &mut Cx| {
                if s.find_open { s.close_find(cx) } else { s.open_find(false, cx) }
            }));
        if let Some(buttons) = ui::chrome::window_buttons(false, cx, t.text) {
            right = right.child(buttons.ml(14.0));
        }
        // Equal flexible sides keep the title centered in the window.
        let mut title = div().flex_row().items_center().gap(5.0).min_w_0().child(text(self.name()).weight(Weight::Bold).truncate());
        if self.is_edited() {
            title = title.child(text("\u{2014} Edited").color(t.text_secondary).shrink_0());
        }
        div()
            .h(TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(12.0)
            .pl(20.0)
            .pr(12.0)
            .bg(t.window)
            .border_b(t.separator)
            .window_drag()
            .child(left)
            .child(title)
            .child(right)
    }

    fn push_button(id: &'static str, label: &str, default: bool, t: &Theme, on_click: impl Fn(&mut Self, &mut Cx) + 'static) -> Div<Self> {
        div()
            .id(id)
            .h(24.0)
            .px(12.0)
            .rounded(6.0)
            .items_center()
            .justify_center()
            .bg(if default { t.accent } else { t.field })
            .hover_bg(if default { t.accent } else { t.toggled })
            .on_click(on_click)
            .child(text(label.to_string()).color(if default { Color::WHITE } else { t.text }).weight(if default { Weight::Medium } else { Weight::Regular }))
    }

    fn find_bar(&self, t: &Theme) -> Div<Self> {
        let count = self.doc.borrow_mut().match_count();
        let current = self.doc.borrow_mut().current_match();
        let query_set = !self.find.borrow().is_empty();
        let status = match (query_set, count, current) {
            (false, ..) => String::new(),
            (true, 0, _) => "Not found".into(),
            (true, n, Some(i)) => format!("{} of {n}", i + 1),
            (true, 1, None) => "1 match".into(),
            (true, n, None) => format!("{n} matches"),
        };
        let icon = t.icon;
        let case = self.case_sensitive;
        let field = div()
            .id("find-field")
            .flex_1()
            .h(24.0)
            .rounded(7.0)
            .bg(t.field)
            .flex_row()
            .items_center()
            .gap(6.0)
            .pl(8.0)
            .pr(3.0)
            .cursor(CursorIcon::Text)
            .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("find"))
            .child(canvas(move |s, b| icons::magnifier(s, b, icon)).size(14.0).shrink_0())
            .child(input("find", &self.find).flex_1())
            .child(text(status).size(11.0).color(t.text_secondary).shrink_0())
            .child(
                div()
                    .id("match-case")
                    .h(18.0)
                    .px(5.0)
                    .rounded(4.0)
                    .items_center()
                    .bg_opt(case.then_some(t.toggled))
                    .hover_bg(t.toggled)
                    .on_click(|s: &mut Self, _| {
                        s.case_sensitive = !s.case_sensitive;
                        s.update_query();
                    })
                    .child(text("Aa").size(11.0).weight(Weight::Semibold).color(if case { t.accent } else { t.text_secondary })),
            );
        let segment = |id: &'static str, glyph: fn(&mut Scene, Bounds, Color), forward: bool| {
            div()
                .id(id)
                .w(28.0)
                .h(24.0)
                .items_center()
                .justify_center()
                .hover_bg(t.toggled)
                .on_click(move |s: &mut Self, _| s.find_next(forward))
                .child(canvas(move |s, b| glyph(s, b, icon)).size(14.0))
        };
        let arrows = div()
            .flex_row()
            .rounded(6.0)
            .bg(t.field)
            .clip()
            .child(segment("find-prev", icons::chevron_left, false))
            .child(div().w(1.0).h(24.0).bg(t.separator))
            .child(segment("find-next", icons::chevron_right, true));
        let replace_on = self.replace_open;
        let accent = t.accent;
        let checkbox = div()
            .id("replace-toggle")
            .flex_row()
            .items_center()
            .gap(5.0)
            .on_click(|s: &mut Self, cx: &mut Cx| {
                s.replace_open = !s.replace_open;
                cx.focus(if s.replace_open { "replace" } else { "find" });
            })
            .child(
                div()
                    .size(14.0)
                    .rounded(4.0)
                    .bg(if replace_on { accent } else { t.window })
                    .border(if replace_on { accent } else { t.text_tertiary }, if replace_on { 0.0 } else { 1.0 })
                    .child_if(replace_on, || canvas(|s, b| icons::check(s, b, Color::WHITE)).size(14.0).into()),
            )
            .child(text("Replace"));

        let row = |content: Div<Self>| content.h(FIND_ROW_H).shrink_0().flex_row().items_center().gap(8.0).px(10.0);
        let mut bar = div()
            .shrink_0()
            .flex_col()
            .bg(t.bar)
            .border_b(t.separator)
            .child(row(div().child(field).child(arrows).child(Self::push_button("find-done", "Done", false, t, |s, cx| s.close_find(cx))).child(checkbox)));
        if self.replace_open {
            let field = div()
                .id("replace-field")
                .flex_1()
                .h(24.0)
                .rounded(7.0)
                .bg(t.field)
                .flex_row()
                .items_center()
                .px(8.0)
                .cursor(CursorIcon::Text)
                .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("replace"))
                .child(input("replace", &self.replace).flex_1());
            bar = bar.child(
                row(div())
                    .pt(0.0)
                    .child(field)
                    .child(Self::push_button("replace-one", "Replace", false, t, |s, _| s.replace_one()))
                    .child(Self::push_button("replace-all", "All", false, t, |s, _| s.replace_all())),
            );
        }
        bar
    }

    fn status_bar(&self, t: &Theme) -> Div<Self> {
        let doc = self.doc.borrow();
        let (line, col) = doc.line_col();
        let lines = doc.line_count();
        let left = match &self.load_error {
            Some(e) => text(e.clone()).size(11.0).color(t.destructive).truncate(),
            None => text(format!("Line {line}, Column {col}")).size(11.0).color(t.text_secondary),
        };
        let right = format!("{} \u{b7} {}", if lines == 1 { "1 line".to_string() } else { format!("{} lines", group(lines)) }, if self.crlf { "CRLF" } else { "LF" });
        div()
            .h(STATUS_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(12.0)
            .px(12.0)
            .bg(t.window)
            .child(left)
            .child(text(right).size(11.0).color(t.text_tertiary).shrink_0())
    }

    fn sheet_card(&self, sheet: Sheet, cx: &Cx, t: &Theme) -> Div<Self> {
        let card = div()
            .w(SHEET_W)
            .flex_col()
            .p(18.0)
            .gap(12.0)
            .rounded(12.0)
            .bg(t.sheet)
            .border(t.sheet_border, 0.5)
            .shadow(t.shadow, 30.0, 10.0)
            .occlude();
        if sheet == Sheet::Unsaved {
            return card
                .child(
                    text(format!("Do you want to save the changes made to the document \u{201c}{}\u{201d}?", self.name()))
                        .weight(Weight::Bold)
                        .line_height(1.35)
                        .w_full(),
                )
                .child(text("Your changes will be lost if you don\u{2019}t save them.").size(11.0).color(t.text_secondary).w_full())
                .child(
                    div()
                        .flex_row()
                        .gap(8.0)
                        .mt(6.0)
                        .child(Self::push_button("dont-save", "Don\u{2019}t Save", false, t, |s, cx| s.discard_and_close(cx)))
                        .child(div().flex_1())
                        .child(Self::push_button("sheet-cancel", "Cancel", false, t, |s, cx| s.close_sheet(cx)))
                        .child(Self::push_button("sheet-ok", "Save\u{2026}", true, t, |s, cx| s.confirm_sheet(cx))),
                );
        }

        let (label, action) = match sheet {
            Sheet::Open => ("Open:", "Open"),
            _ if self.confirm_replace => ("Save As:", "Replace"),
            _ => ("Save As:", "Save"),
        };
        let mut list = div().flex_col().w_full().rounded(8.0).bg(t.field).py(4.0).px(4.0);
        for (i, s) in self.suggestions.iter().enumerate() {
            let hovered = cx.hovered(("suggestion", i));
            let fg = if hovered { Color::WHITE } else { t.text };
            let icon_c = if hovered { Color::WHITE } else { t.accent };
            let icon: fn(&mut Scene, Bounds, Color) = if s.is_dir { icons::folder } else { icons::document };
            list = list.child(
                div()
                    .id(("suggestion", i))
                    .h(24.0)
                    .px(6.0)
                    .rounded(5.0)
                    .flex_row()
                    .items_center()
                    .gap(6.0)
                    .hover_bg(t.accent)
                    .on_click(move |s: &mut Self, cx: &mut Cx| s.pick_suggestion(i, cx))
                    .child(canvas(move |sc, b| icon(sc, b, icon_c)).size(16.0).shrink_0())
                    .child(text(s.name.clone()).color(fg).truncate()),
            );
        }
        let field = div()
            .id("path-field")
            .flex_1()
            .h(26.0)
            .rounded(6.0)
            .bg(t.window)
            .border(t.separator, 1.0)
            .flex_row()
            .items_center()
            .px(7.0)
            .cursor(CursorIcon::Text)
            .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("path"))
            .child(input("path", &self.path_field).flex_1());
        let error = self.sheet_error.clone();
        let warn = self.confirm_replace;
        card.child(div().flex_row().items_center().gap(10.0).child(text(label).weight(Weight::Medium)).child(field))
            .child_if(!self.suggestions.is_empty(), || list.into())
            .child_if(error.is_some(), || {
                text(error.clone().unwrap_or_default()).size(11.0).color(if warn { t.text_secondary } else { t.destructive }).w_full().into()
            })
            .child(
                div()
                    .flex_row()
                    .justify_end()
                    .gap(8.0)
                    .child(Self::push_button("sheet-cancel", "Cancel", false, t, |s, cx| s.close_sheet(cx)))
                    .child(Self::push_button("sheet-ok", action, true, t, |s, cx| s.confirm_sheet(cx))),
            )
    }

    fn sheet_overlay(&self, sheet: Sheet, cx: &Cx, t: &Theme) -> Div<Self> {
        let (w, h) = cx.window_size;
        let width = if sheet == Sheet::Unsaved { 420.0 } else { SHEET_W };
        // Sheets hang from the toolbar and keep the window modal below it.
        div()
            .absolute(0.0, TOOLBAR_H)
            .w(w)
            .h(h - TOOLBAR_H)
            .deferred()
            .occlude()
            .child(self.sheet_card(sheet, cx, t).w(width).absolute(((w - width) / 2.0).max(8.0), 8.0))
    }
}

/// Opens another TextEdit window (a separate process).
pub fn spawn_window(path: Option<&Path>) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    if let Some(p) = path {
        cmd.arg(p);
    }
    let _ = cmd.spawn();
}

fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl App for TextEditApp {
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Self::theme(cx).text }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        let t = Self::theme(cx);
        self.sync_styles(&t);
        let (w, h) = cx.window_size;
        let mut root = div().w(w).h(h).flex_col().bg(t.window).text_color(t.text).child(self.toolbar(cx, &t));
        if self.find_open {
            root = root.child(self.find_bar(&t));
        }
        root = root
            .child(widget("doc", &self.doc).flex_1().cursor(CursorIcon::Text))
            .child(div().h(1.0).shrink_0().bg(t.separator))
            .child(self.status_bar(&t));
        if let Some(sheet) = self.sheet {
            root = root.child(self.sheet_overlay(sheet, cx, &t));
        }
        if cx.transparent && !cx.maximized && !ui::is_y7ya() {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(t.window_border, 1.0));
        }
        root
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if let Some(sheet) = self.sheet {
            if key.is_named(NamedKey::Escape) {
                self.close_sheet(cx);
                return true;
            }
            if key.is_named(NamedKey::Enter) {
                self.confirm_sheet(cx);
                return true;
            }
            if sheet == Sheet::Unsaved {
                if key.ctrl && key.is_char('d') {
                    self.discard_and_close(cx);
                }
                return true;
            }
            if key.is_named(NamedKey::Tab) {
                self.pick_suggestion(0, cx);
                return true;
            }
            // Everything else edits the path.
            return key.ctrl && !(key.is_char('a') || key.is_char('c') || key.is_char('x') || key.is_char('v') || key.is_char('z'));
        }
        if key.ctrl && !key.alt {
            if key.is_char('s') {
                if key.shift { self.begin_save_as(false, cx) } else { self.save(cx) }
                return true;
            }
            if key.is_char('o') {
                self.begin_open(cx);
                return true;
            }
            if key.is_char('n') {
                spawn_window(None);
                return true;
            }
            if key.is_char('f') {
                self.open_find(false, cx);
                return true;
            }
            if key.is_char('h') {
                self.open_find(true, cx);
                return true;
            }
            if key.is_char('g') {
                self.find_next(!key.shift);
                return true;
            }
            if key.is_char('e') {
                self.use_selection_for_find(cx);
                return true;
            }
            if key.is_char('w') || key.is_char('q') {
                cx.close_window();
                return true;
            }
        }
        if key.ctrl && key.alt && key.is_char('f') {
            self.open_find(true, cx);
            return true;
        }
        if key.is_named(NamedKey::Escape) && self.find_open {
            self.close_find(cx);
            return true;
        }
        if key.is_named(NamedKey::Enter) {
            if cx.is_focused("find") {
                self.find_next(!key.shift);
                return true;
            }
            if cx.is_focused("replace") {
                self.replace_one();
                return true;
            }
        }
        if key.is_named(NamedKey::Tab) && cx.is_focused("find") {
            cx.focus(if self.replace_open { "replace" } else { "doc" });
            return true;
        }
        false
    }

    fn on_input(&mut self, id: ElementId, _cx: &mut Cx) {
        if id == eid("find") {
            self.update_query();
        } else if id == eid("path") {
            self.sheet_error = None;
            self.confirm_replace = false;
            self.refresh_suggestions();
        }
    }

    fn should_close(&mut self, cx: &mut Cx) -> bool {
        if self.closing || !self.is_edited() {
            return true;
        }
        self.sheet = Some(Sheet::Unsaved);
        cx.blur();
        false
    }
}

/// Sample text for `--snapshot` renders.
pub const DEMO_TEXT: &str = "Packing list for the coast

Clothes: two sweaters, rain jacket, hiking boots, one nice shirt for dinner on Saturday.
Gear: headlamp, water bottle, the small first-aid kit, binoculars for the tide pools.
Food: trail mix, coffee beans, and whatever bread is left from the bakery.

Before leaving
- Water the plants and set the thermostat to away.
- Download offline maps; the signal drops past the lighthouse road.
- Charge the camera batteries and empty the memory card.

Notes from last trip: the north trail floods at high tide, so check the tide table before heading out. The café by the harbor opens at seven and the cinnamon buns sell out by nine.
";

#[cfg(test)]
mod tests {
    use super::*;

    fn press(app: &mut TextEditApp, cx: &mut Cx, key: Key, ctrl: bool, shift: bool) {
        let kp = KeyPress { key, text: None, ctrl, shift, alt: false };
        app.on_key(&kp, cx);
    }

    fn ctrl(app: &mut TextEditApp, cx: &mut Cx, c: &str, shift: bool) {
        press(app, cx, Key::Character(c.into()), true, shift);
    }

    #[test]
    fn save_load_and_close_flow() {
        let dir = std::env::temp_dir().join(format!("textedit-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("existing.txt");
        std::fs::write(&existing, "old\r\ncontents\r\n").unwrap();
        let png = std::env::temp_dir().join("textedit-selftest.png");

        ui::snapshot((720.0, 520.0), 1.0, false, &png, |cx| TextEditApp::new(Some(existing.clone()), cx), |app, cx| {
            assert_eq!(app.doc.borrow().text(), "old\ncontents\n");
            assert!(app.crlf && !app.is_edited());
            assert!(app.should_close(cx));

            // Editing marks the document; undoing back clears it.
            app.doc.borrow_mut().insert("new ");
            assert!(app.is_edited());
            app.doc.borrow_mut().undo();
            assert!(!app.is_edited());

            app.doc.borrow_mut().insert("new ");
            assert!(!app.should_close(cx));
            assert!(app.sheet == Some(Sheet::Unsaved));
            press(app, cx, Key::Named(NamedKey::Escape), false, false);
            assert!(app.sheet.is_none());

            // Ctrl+S writes back with the file's own line endings.
            ctrl(app, cx, "s", false);
            assert!(!app.is_edited());
            assert_eq!(std::fs::read_to_string(&existing).unwrap(), "new old\r\ncontents\r\n");

            // Save As through the sheet; an existing file needs a second confirm.
            ctrl(app, cx, "s", true);
            assert!(matches!(app.sheet, Some(Sheet::Save { .. })));
            let other = dir.join("copy.txt");
            std::fs::write(&other, "keep me").unwrap();
            app.path_field.borrow_mut().set_text(&mut cx.text.fonts, &other.display().to_string());
            app.on_input(eid("path"), cx);
            press(app, cx, Key::Named(NamedKey::Enter), false, false);
            assert!(app.confirm_replace);
            assert_eq!(std::fs::read_to_string(&other).unwrap(), "keep me");
            press(app, cx, Key::Named(NamedKey::Enter), false, false);
            assert!(app.sheet.is_none());
            assert_eq!(app.path.as_deref(), Some(other.as_path()));
            assert_eq!(std::fs::read_to_string(&other).unwrap(), "new old\r\ncontents\r\n");

            // Find and replace through the bar.
            ctrl(app, cx, "f", false);
            assert!(app.find_open && cx.is_focused("find"));
            app.find.borrow_mut().set_text(&mut cx.text.fonts, "O");
            app.on_input(eid("find"), cx);
            assert_eq!(app.doc.borrow_mut().match_count(), 2);
            app.replace.borrow_mut().set_text(&mut cx.text.fonts, "0");
            app.replace_all();
            assert_eq!(app.doc.borrow().text(), "new 0ld\nc0ntents\n");
            press(app, cx, Key::Named(NamedKey::Escape), false, false);
            assert!(!app.find_open && cx.is_focused("doc"));

            // Closing with changes, then choosing Don't Save.
            assert!(!app.should_close(cx));
            app.discard_and_close(cx);
            assert!(app.should_close(cx));
        });
    }

    #[test]
    fn missing_file_becomes_a_new_document() {
        let path = std::env::temp_dir().join(format!("textedit-new-{}.txt", std::process::id()));
        let png = std::env::temp_dir().join("textedit-selftest-new.png");
        ui::snapshot((720.0, 520.0), 1.0, true, &png, |cx| TextEditApp::new(Some(path.clone()), cx), |app, _| {
            assert_eq!(app.name(), path.file_name().unwrap().to_string_lossy());
            assert!(app.load_error.is_none());
            assert!(app.doc.borrow().is_empty());
        });
    }
}
