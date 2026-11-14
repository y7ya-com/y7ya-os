//! `clipboard --pick`: a Spotlight-style panel over the clipboard history.
//! Type to search, arrows to move, Return to copy the entry back and close.
//!
//! Self-contained so it can move onto a layer-shell surface later; today it
//! runs in a normal undecorated window.

mod icons;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Local, TimeZone};
use ui::widgets::image::{Fit, Image, image};
use ui::widgets::table::{Cell, Column, Row, Table, TableState, TableStyle};
use ui::*;

use crate::history::{Entry, Kind, Store, now_ms};

pub const SIZE: (f32, f32) = (780.0, 480.0);
const LIST_W: f32 = 330.0;
const ROW_H: f32 = 50.0;
const SEARCH_SIZE: f32 = 20.0;
const PLACEHOLDER: &str = "Search Clipboard";
/// Characters of text shown in the preview pane.
const PREVIEW_CHARS: usize = 20_000;

#[derive(Clone, Copy)]
struct Theme {
    bg: Color,
    preview_bg: Color,
    divider: Color,
    text: Color,
    text_secondary: Color,
    text_tertiary: Color,
    accent: Color,
    hover: Color,
    field: Color,
    text_selection: Color,
    window_border: Color,
}

impl Theme {
    fn get(dark: bool) -> Self {
        if dark {
            Self {
                bg: rgb(0x262628),
                preview_bg: rgb(0x1e1e1e),
                divider: rgba(0xffffff, 0.10),
                text: rgba(0xffffff, 0.85),
                text_secondary: rgba(0xffffff, 0.55),
                text_tertiary: rgba(0xffffff, 0.25),
                accent: rgb(0x0a84ff),
                hover: rgba(0xffffff, 0.06),
                field: rgba(0xffffff, 0.08),
                text_selection: rgba(0x0a84ff, 0.40),
                window_border: rgba(0xffffff, 0.14),
            }
        } else {
            Self {
                bg: rgb(0xf6f6f6),
                preview_bg: rgb(0xffffff),
                divider: rgba(0x000000, 0.10),
                text: rgba(0x000000, 0.85),
                text_secondary: rgba(0x000000, 0.50),
                text_tertiary: rgba(0x000000, 0.26),
                accent: rgb(0x007aff),
                hover: rgba(0x000000, 0.05),
                field: rgba(0x000000, 0.05),
                text_selection: rgba(0x007aff, 0.25),
                window_border: rgba(0x000000, 0.22),
            }
        }
    }
}

/// Decoded pictures for an image entry: list thumbnail and preview.
struct Pictures {
    thumb: Image,
    large: Image,
}

pub struct Picker {
    store: Store,
    entries: Vec<Entry>,
    texts: HashMap<String, String>,
    pictures: HashMap<String, Option<Pictures>>,
    search: Rc<RefCell<TextInput>>,
    query: String,
    selected: Option<String>,
    list: TableState,
    /// Close once focus moves elsewhere, like Spotlight (after we had it).
    had_focus: bool,
    /// Previews render without side effects (no copying, no closing).
    preview: bool,
}

impl Picker {
    pub fn new(store: Store, preview: bool, cx: &mut Cx) -> Self {
        let entries = store.load();
        let texts = entries.iter().filter(|e| e.kind == Kind::Text).filter_map(|e| Some((e.id.clone(), store.read_text(e)?))).collect();
        // The placeholder is drawn by `search_bar`, clear of the caret.
        let search = TextInput::new(&mut cx.text.fonts, TextStyle::default(), true);
        cx.window_radius = 12.0;
        cx.focus("search");
        let selected = entries.first().map(|e| e.id.clone());
        Self {
            store,
            entries,
            texts,
            pictures: HashMap::new(),
            search: Rc::new(RefCell::new(search)),
            query: String::new(),
            selected,
            list: TableState::new("clips"),
            had_focus: false,
            preview,
        }
    }

    fn reload(&mut self) {
        self.entries = self.store.load();
        let ids: Vec<&String> = self.entries.iter().map(|e| &e.id).collect();
        self.texts.retain(|k, _| ids.contains(&k));
    }

    fn visible(&self) -> Vec<&Entry> {
        let q = self.query.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| match e.kind {
                _ if q.is_empty() => true,
                Kind::Text => self.texts.get(&e.id).is_some_and(|t| t.to_lowercase().contains(&q)),
                Kind::Image => "image".contains(&q) || "picture".contains(&q),
            })
            .collect()
    }

    fn ensure_selection(&mut self) {
        let vis = self.visible();
        if self.selected.as_ref().is_none_or(|s| !vis.iter().any(|e| &e.id == s)) {
            self.selected = vis.first().map(|e| e.id.clone());
        }
    }

    /// Selects the `n`th visible entry (for previews).
    pub fn select_nth(&mut self, n: usize) {
        self.selected = None;
        self.ensure_selection();
        self.move_selection(n as i32);
    }

    /// Types `q` into the search field (for previews and tests).
    pub fn search_for(&mut self, q: &str, cx: &mut Cx) {
        let mut s = self.search.borrow_mut();
        s.set_text(&mut cx.text.fonts, q);
        s.move_to_end();
        drop(s);
        self.on_input(eid("search"), cx);
    }

    fn move_selection(&mut self, delta: i32) {
        let ids: Vec<String> = self.visible().iter().map(|e| e.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let i = self.selected.as_ref().and_then(|s| ids.iter().position(|x| x == s)).map_or(0, |i| i as i32 + delta);
        let i = i.clamp(0, ids.len() as i32 - 1) as usize;
        self.selected = Some(ids[i].clone());
        self.list.reveal(i);
    }

    /// Copies the entry back to the clipboard and closes.
    fn pick(&mut self, id: &str, cx: &mut Cx) {
        if self.preview {
            return;
        }
        let _ = self.store.touch(id, now_ms());
        if let Err(e) = crate::serve::spawn(id) {
            eprintln!("clipboard: couldn't copy: {e}");
        }
        cx.close_window();
    }

    fn toggle_pin(&mut self, id: &str) {
        let pinned = self.entries.iter().find(|e| e.id == id).is_some_and(|e| e.pinned);
        let _ = self.store.set_pinned(id, !pinned);
        self.reload();
    }

    fn remove(&mut self, id: &str) {
        // Select the neighbor before the row disappears.
        let ids: Vec<String> = self.visible().iter().map(|e| e.id.clone()).collect();
        if let Some(i) = ids.iter().position(|x| x == id) {
            self.selected = ids.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| ids.get(j))).cloned();
        }
        let _ = self.store.remove(id);
        self.pictures.remove(id);
        self.reload();
    }

    fn pictures(&mut self, e: &Entry) -> Option<&Pictures> {
        if !self.pictures.contains_key(&e.id) {
            let decoded = self.store.read(e).and_then(|png| {
                Some(Pictures { thumb: Image::decode(&png, 96)?, large: Image::decode(&png, 512)? })
            });
            self.pictures.insert(e.id.clone(), decoded);
        }
        self.pictures.get(&e.id).and_then(|p| p.as_ref())
    }

    fn sync_input_style(&self, t: &Theme) {
        let mut s = self.search.borrow_mut();
        s.set_styles(TextStyle { size: SEARCH_SIZE, line_height: 1.25, weight: Weight::Regular, color: t.text }, None);
        s.placeholder_color = t.text_tertiary;
        s.caret_color = t.accent;
        s.selection_color = t.text_selection;
    }

    fn search_bar(&self, t: &Theme) -> Div<Self> {
        let c = t.text_secondary;
        let n = self.visible().len();
        let count = match n {
            1 => "1 item".to_string(),
            n => format!("{n} items"),
        };
        div()
            .h(56.0)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(12.0)
            .px(18.0)
            .border_b(t.divider)
            .window_drag()
            .child(canvas(move |s, b| icons::magnifier(s, b, c)).size(22.0).shrink_0())
            .child(
                div()
                    .id("search-box")
                    .flex_1()
                    .min_w_0()
                    .cursor(CursorIcon::Text)
                    .child(input("search", &self.search).w_full())
                    // Starts just past the caret, as in Spotlight, rather than under it.
                    .child_if(self.query.is_empty(), || {
                        text(PLACEHOLDER)
                            .size(SEARCH_SIZE)
                            .line_height(1.25)
                            .color(t.text_tertiary)
                            .absolute(3.0, 0.0)
                            .into()
                    }),
            )
            .child(text(count).size(12.0).tabular().color(t.text_secondary).shrink_0())
    }

    /// One entry. `selected` rows are accent-colored while the window is
    /// active (`emphasized`) and gray otherwise.
    fn row(&mut self, e: &Entry, selected: bool, emphasized: bool, t: &Theme) -> Row<Self> {
        let lit = selected && emphasized;
        let ink = if lit { Color::WHITE } else { t.text };
        let secondary = if lit { rgba(0xffffff, 0.75) } else { t.text_secondary };
        let id = e.id.clone();

        let mut thumb = div().size(34.0).shrink_0().rounded(7.0).items_center().justify_center().clip();
        let (title, detail) = match e.kind {
            Kind::Text => {
                let text = self.texts.get(&e.id).map(String::as_str).unwrap_or("");
                let c = secondary;
                thumb = thumb.bg(if lit { rgba(0xffffff, 0.18) } else { t.field }).child(canvas(move |s, b| icons::document(s, b, c)).size(20.0));
                let chars = text.chars().count();
                (one_line(text), format!("{} · {} characters", ago(e.time), group_digits(chars)))
            }
            Kind::Image => {
                if let Some(p) = self.pictures(e) {
                    let (w, h) = fit(p.thumb.width(), p.thumb.height(), 34.0, 34.0);
                    let el = image(&p.thumb, Fit::Contain, 0.0).w(w).h(h);
                    thumb = thumb.bg(if lit { rgba(0xffffff, 0.18) } else { t.field }).child(el);
                }
                let dims = e.dims.map_or(String::new(), |(w, h)| format!(" {w}×{h}"));
                (format!("Image{dims}"), format!("{} · {}", ago(e.time), human_size(e.size)))
            }
        };
        let content = div()
            .flex_row()
            .items_center()
            .gap(10.0)
            .flex_1()
            .min_w_0()
            .child(thumb)
            .child(
                div()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(2.0)
                    .child(text(title).color(ink).truncate())
                    .child(text(detail).size(11.0).tabular().color(secondary).truncate()),
            )
            .child_if(e.pinned, || {
                let c = if lit { ink } else if emphasized { t.accent } else { t.text_secondary };
                canvas(move |s, b| icons::pin(s, b, c)).size(14.0).shrink_0().into()
            });
        let (a, b) = (id.clone(), id.clone());
        Row::new(("clip", id), vec![Cell::new(content)])
            .selected(selected)
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.selected = Some(a.clone());
                cx.focus("search");
            })
            .on_double_click(move |s: &mut Self, cx: &mut Cx| s.pick(&b.clone(), cx))
    }

    fn preview_pane(&mut self, t: &Theme) -> Div<Self> {
        let pane = div().flex_1().min_w_0().h_full().flex_col().bg(t.preview_bg);
        let Some(e) = self.selected.as_ref().and_then(|s| self.entries.iter().find(|e| &e.id == s)).cloned() else {
            return pane;
        };
        let mut facts: Vec<(&str, String)> = Vec::new();
        let body: Div<Self> = match e.kind {
            Kind::Text => {
                let full = self.texts.get(&e.id).cloned().unwrap_or_default();
                let chars = full.chars().count();
                let shown: String = full.chars().take(PREVIEW_CHARS).collect();
                facts.push(("Content", "Text".into()));
                facts.push(("Characters", group_digits(chars)));
                facts.push(("Lines", group_digits(full.lines().count().max(1))));
                div()
                    .id("preview-scroll")
                    .scroll_y()
                    .flex_1()
                    .flex_col()
                    .px(20.0)
                    .py(16.0)
                    .child(text(shown).size(13.0).line_height(1.45).w_full())
            }
            Kind::Image => {
                facts.push(("Content", "Image (PNG)".into()));
                if let Some((w, h)) = e.dims {
                    facts.push(("Dimensions", format!("{w} × {h}")));
                }
                facts.push(("Size", human_size(e.size)));
                let border = t.divider;
                let el = self.pictures(&e).map(|p| {
                    let (w, h) = fit(p.large.width(), p.large.height(), 380.0, 250.0);
                    div().w(w).h(h).border(border, 1.0).child(image(&p.large, Fit::Contain, 0.0).w(w).h(h))
                });
                div().flex_1().items_center().justify_center().p(20.0).children(el)
            }
        };
        facts.push(("Copied", full_time(e.time)));
        if e.pinned {
            facts.push(("Pinned", "Yes".into()));
        }
        let mut info = div().flex_col().gap(5.0).px(20.0).py(14.0);
        for (k, v) in facts {
            info = info.child(
                div()
                    .flex_row()
                    .justify_between()
                    .gap(12.0)
                    .child(text(k).size(12.0).color(t.text_secondary).shrink_0())
                    .child(text(v).size(12.0).tabular().truncate()),
            );
        }
        pane.child(body).child(div().h(1.0).shrink_0().bg(t.divider)).child(info)
    }

    /// The action bar: what each key does, as "Copy ↩" pairs.
    fn footer(&self, t: &Theme) -> Div<Self> {
        let key = |label: &str, k: &str| {
            div()
                .flex_row()
                .items_center()
                .gap(6.0)
                .child(text(label.to_string()).size(11.0).color(t.text_secondary))
                .child(
                    div()
                        .h(18.0)
                        .px(5.0)
                        .rounded(4.0)
                        .bg(t.field)
                        .border(t.divider, 0.5)
                        .items_center()
                        .justify_center()
                        .child(text(k.to_string()).size(11.0).weight(Weight::Medium).color(t.text_secondary)),
                )
        };
        let pinned = self.selected.as_ref().and_then(|s| self.entries.iter().find(|e| &e.id == s)).is_some_and(|e| e.pinned);
        div()
            .h(36.0)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(16.0)
            .px(16.0)
            .child(text("Clipboard").size(11.0).weight(Weight::Semibold).color(t.text_secondary).truncate().flex_1())
            .child(key("Copy", "↩"))
            .child(key(if pinned { "Unpin" } else { "Pin" }, "Ctrl+P"))
            .child(key("Remove", "Del"))
            .child(key("Close", "Esc"))
    }
}

impl App for Picker {
    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Theme::get(cx.dark).text }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        let t = Theme::get(cx.dark);
        self.sync_input_style(&t);
        if cx.window_focused {
            self.had_focus = true;
        } else if self.had_focus && !self.preview {
            cx.close_window();
        }
        let (w, h) = cx.window_size;

        let entries: Vec<Entry> = self.visible().into_iter().cloned().collect();
        let style = TableStyle {
            row_h: ROW_H,
            text: t.text,
            text_secondary: t.text_secondary,
            divider: t.divider,
            hover: t.hover,
            // The list follows the search field's focus: accent while the
            // window is active, gray when it isn't.
            selection: native::selection(cx, true),
            selection_text: native::selection_text(cx, true, t.text),
            zebra: None,
        };
        let columns = [Column::new("")];
        let selected = self.selected.clone();
        let emphasized = cx.window_focused;
        let list = if entries.is_empty() {
            let msg = if self.entries.is_empty() { "No Items" } else { "No Results" };
            div().flex_1().flex_col().items_center().justify_center().child(text(msg).size(15.0).weight(Weight::Medium).color(t.text_tertiary))
        } else {
            // Rows decode thumbnails lazily, which needs `&mut self`.
            let state = std::mem::replace(&mut self.list, TableState::new("unused"));
            let list = Table::new(&state, &columns, entries.len(), style)
                .headerless()
                .render(cx, |i, _cx| {
                    let e = &entries[i];
                    let sel = selected.as_deref() == Some(e.id.as_str());
                    self.row(e, sel, emphasized, &t)
                });
            self.list = state;
            div().flex_1().flex_col().pt(6.0).child(list)
        };

        let mut root = div()
            .w(w)
            .h(h)
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .child(self.search_bar(&t))
            .child(
                div()
                    .flex_1()
                    .min_h(0.0)
                    .flex_row()
                    .child(div().w(LIST_W).h_full().shrink_0().flex_col().child(list))
                    .child(div().w(1.0).h_full().bg(t.divider))
                    .child(self.preview_pane(&t)),
            )
            .child(div().h(1.0).shrink_0().bg(t.divider))
            .child(self.footer(&t));
        if cx.transparent && !cx.maximized && !ui::is_y7ya() {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(t.window_border, 1.0));
        }
        root
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        let sel = self.selected.clone();
        if key.is_named(NamedKey::Escape) {
            if self.query.is_empty() {
                cx.close_window();
            } else {
                self.query.clear();
                self.search.borrow_mut().set_text(&mut cx.text.fonts, "");
                self.ensure_selection();
            }
            return true;
        }
        if key.is_named(NamedKey::ArrowDown) {
            self.move_selection(1);
            return true;
        }
        if key.is_named(NamedKey::ArrowUp) {
            self.move_selection(-1);
            return true;
        }
        if key.is_named(NamedKey::PageDown) || key.is_named(NamedKey::PageUp) {
            let n = self.list.page_rows(ROW_H) as i32;
            self.move_selection(if key.is_named(NamedKey::PageDown) { n } else { -n });
            return true;
        }
        if key.is_named(NamedKey::Enter) {
            if let Some(id) = sel {
                self.pick(&id, cx);
            }
            return true;
        }
        if key.ctrl && key.is_char('p') {
            if let Some(id) = sel {
                self.toggle_pin(&id);
            }
            return true;
        }
        if key.is_named(NamedKey::Delete) {
            if let Some(id) = sel {
                self.remove(&id);
            }
            return true;
        }
        if key.ctrl && (key.is_char('w') || key.is_char('q')) {
            cx.close_window();
            return true;
        }
        // Everything else edits the search field.
        cx.focus("search");
        false
    }

    fn on_input(&mut self, id: ElementId, cx: &mut Cx) {
        if id == eid("search") {
            self.query = self.search.borrow().text();
            self.list.scroll_to_top(cx);
            self.selected = None;
            self.ensure_selection();
        }
    }
}

/// The first non-blank line, whitespace collapsed.
fn one_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "Just now", "5 min ago", "3 h ago", "Yesterday", "Mar 4".
fn ago(ms: i64) -> String {
    let secs = (now_ms() - ms).max(0) / 1000;
    match secs {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        86400..172800 => "Yesterday".into(),
        _ => Local.timestamp_millis_opt(ms).single().map_or(String::new(), |t| t.format("%b %-d").to_string()),
    }
}

fn full_time(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map_or(String::new(), |t| t.format("%a %b %-d, %H:%M").to_string())
}

fn group_digits(n: usize) -> String {
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

fn human_size(b: u64) -> String {
    match b {
        0..1024 => format!("{b} bytes"),
        1024..1_048_576 => format!("{:.0} KB", b as f64 / 1024.0),
        _ => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

/// The largest size with the image's aspect ratio that fits `max_w`×`max_h`
/// (never scaled up).
fn fit(w: u32, h: u32, max_w: f32, max_h: f32) -> (f32, f32) {
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let k = (max_w / w).min(max_h / h).min(1.0);
    ((w * k).round().max(1.0), (h * k).round().max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(one_line("\n\n  hello   there \nsecond"), "hello there");
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(fit(1920, 1080, 380.0, 250.0), (380.0, 214.0));
        assert_eq!(fit(100, 400, 34.0, 34.0), (9.0, 34.0));
        assert_eq!(fit(10, 10, 34.0, 34.0), (10.0, 10.0));
    }

    #[test]
    fn search_filters_and_keeps_a_selection() {
        let dir = std::env::temp_dir().join(format!("clipboard-picker-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir);
        for (i, t) in ["sudo pacman -Syu", "hello@example.com", "cargo test"].iter().enumerate() {
            store.add(&crate::history::Content::Text(t.to_string()), now_ms() - i as i64 * 1000).unwrap();
        }
        let png = dir.join("snapshot.png");
        ui::snapshot(SIZE, 1.0, false, &png, |cx| Picker::new(store, true, cx), |p, cx| {
            assert_eq!(p.visible().len(), 3);
            p.search_for("PACMAN", cx);
            assert_eq!(p.visible().len(), 1);
            assert_eq!(p.selected.as_deref(), Some(p.visible()[0].id.as_str()));
            p.search_for("nothing like this", cx);
            assert!(p.visible().is_empty() && p.selected.is_none());
            p.search_for("", cx);
            assert_eq!(p.visible().len(), 3);
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
