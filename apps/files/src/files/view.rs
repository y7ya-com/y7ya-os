//! Drawing the window: sidebar, toolbar, the icon, list and column views,
//! path bar and status line, and the overlays (menus, Quick Look, dialogs,
//! progress, drag ghost).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use media::Kind;
use ui::*;

use super::fs::{self, Entry, Op, SortKey, format_size};
use super::icons::{self, Icon};
use super::theme::Theme;
use super::{Action, Dialog, DropTarget, FilesApp, Location, Menu, Preview, SIDEBAR_W, TOOLBAR_H, ViewMode};

const ICON: f32 = 64.0;
const CELL_MIN_W: f32 = 104.0;
const CELL_H: f32 = 112.0;
const GRID_PAD: f32 = 14.0;
const LABEL_LINE: f32 = 16.0;
const ROW_H: f32 = 24.0;
const COL_W: f32 = 230.0;
const PREVIEW_COL_W: f32 = 270.0;
const DATE_W: f32 = 180.0;
const SIZE_W: f32 = 84.0;
const KIND_W: f32 = 150.0;
const TRASH_BAR_H: f32 = 34.0;
const PATH_BAR_H: f32 = 26.0;
const STATUS_H: f32 = 24.0;

/// Icon view geometry for a content width: columns and their width.
pub fn icon_layout(width: f32) -> (usize, f32) {
    let inner = (width - 2.0 * GRID_PAD).max(CELL_MIN_W);
    let cols = (inner / CELL_MIN_W).floor().max(1.0) as usize;
    (cols, inner / cols as f32)
}

/// Where item `i`'s icon and label sit, in scrolled content coordinates.
pub fn icon_item_rect(i: usize, cols: usize, width: f32) -> Bounds {
    let (_, cell_w) = icon_layout(width);
    let cols = cols.max(1);
    let (row, col) = (i / cols, i % cols);
    let x = GRID_PAD + col as f32 * cell_w + cell_w / 2.0;
    let y = GRID_PAD + row as f32 * CELL_H;
    Bounds::new(x - 44.0, y + 2.0, 88.0, CELL_H - 10.0)
}

/// The range of rows of height `row_h` visible in a viewport.
fn visible_rows(scroll: f32, viewport_h: f32, top: f32, row_h: f32, rows: usize) -> (usize, usize) {
    let first = ((scroll - top) / row_h).floor().max(0.0) as usize;
    let last = (((scroll + viewport_h - top) / row_h).ceil().max(0.0) as usize + 1).min(rows);
    (first.min(rows), last)
}

/// Scroll offset that brings `[y, y + h)` into a viewport of `view_h`.
fn reveal_offset(scroll: f32, view_h: f32, y: f32, h: f32) -> f32 {
    if y < scroll {
        y
    } else if y + h > scroll + view_h {
        y + h - view_h
    } else {
        scroll
    }
}

impl FilesApp {
    fn sync_input_styles(&self, t: &Theme) {
        for input in [&self.search, &self.rename] {
            let mut i = input.borrow_mut();
            i.set_styles(TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: t.text }, None);
            i.placeholder_color = t.text_secondary;
            i.caret_color = t.accent;
            i.selection_color = t.text_selection;
        }
    }

    pub(super) fn render_window(&mut self, cx: &mut Cx) -> Div<Self> {
        let t = Self::theme(cx);
        self.sync_input_styles(&t);
        self.drop_targets.clear();
        let (w, h) = cx.window_size;

        let mut root = div().w(w).h(h).flex_row().bg(t.content).text_color(t.text);
        root = root.child(self.sidebar(cx, &t)).child(div().w(1.0).h_full().shrink_0().bg(t.divider)).child(self.main(cx, &t));
        if let Some(p) = self.progress_panel(&t) {
            root = root.child(p);
        }
        if self.quicklook.is_some() {
            root = root.child(self.quicklook_panel(cx, &t));
        }
        if let Some(menu) = &self.menu {
            root = root.child(menu_overlay(menu, cx, &t));
        }
        if let Some(d) = &self.dialog {
            root = root.child(self.dialog_sheet(d, cx, &t));
        }
        if let Some(g) = self.drag_ghost(cx) {
            root = root.child(g);
        }
        // Frameless windows get no compositor outline (except on y7ya-os).
        if cx.transparent && !cx.maximized && !ui::is_y7ya() {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(t.window_border, 1.0));
        }
        self.trim_thumbs();
        root
    }

    fn dragging_over(&self, id: ElementId, cx: &Cx) -> bool {
        self.drag.is_some() && cx.is_hovered(id)
    }

    // -- sidebar --------------------------------------------------------------

    fn sidebar(&mut self, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut header = div().h(TOOLBAR_H).shrink_0().flex_row().items_center().pl(20.0).pr(12.0).window_drag();
        if let Some(buttons) = ui::chrome::window_buttons(true, cx, t.text) {
            header = header.child(buttons);
        }

        let section = |label: &str| -> AnyElement<Self> {
            text(label.to_string()).size(11.0).weight(Weight::Semibold).color(t.text_secondary).pl(10.0).pt(12.0).pb(5.0).into()
        };
        let mut rows: Vec<AnyElement<Self>> = vec![section("Favorites")];
        rows.push(self.sidebar_row("Recents".into(), icons::clock, Location::Recents, None, cx, t).into());
        let p = self.places.clone();
        let home_name = p.home.file_name().map_or("Home".to_string(), |n| n.to_string_lossy().into_owned());
        let favorites: [(String, Icon, &PathBuf); 7] = [
            (home_name, icons::house, &p.home),
            ("Desktop".into(), icons::desktop, &p.desktop),
            ("Documents".into(), icons::document, &p.documents),
            ("Downloads".into(), icons::download, &p.downloads),
            ("Pictures".into(), icons::photo, &p.pictures),
            ("Music".into(), icons::music, &p.music),
            ("Videos".into(), icons::film, &p.videos),
        ];
        for (name, icon, path) in favorites {
            if path.is_dir() {
                rows.push(self.sidebar_row(name, icon, Location::Dir(path.clone()), None, cx, t).into());
            }
        }
        rows.push(section("Locations"));
        for m in self.mounts.clone() {
            let icon: Icon = if m.removable {
                icons::external_drive
            } else if m.device.starts_with("/dev/") {
                icons::drive
            } else {
                icons::network
            };
            let free = super::places::space(&m.path).map(|(free, _)| format_size(free));
            rows.push(self.sidebar_row(m.name.clone(), icon, Location::Dir(m.path.clone()), free, cx, t).into());
        }
        rows.push(self.sidebar_row("Trash".into(), icons::trash, Location::Trash, None, cx, t).into());

        div()
            .w(SIDEBAR_W)
            .h_full()
            .shrink_0()
            .flex_col()
            .bg(t.sidebar)
            .child(header)
            .child(div().id("sidebar-scroll").scroll_y().flex_1().flex_col().px(10.0).pb(10.0).gap(1.0).children(rows))
    }

    fn sidebar_row(&mut self, name: String, icon: Icon, loc: Location, detail: Option<String>, cx: &Cx, t: &Theme) -> Div<Self> {
        let id = eid(("place", format!("{loc:?}")));
        let selected = self.loc == loc && !self.searching();
        let target = match &loc {
            Location::Dir(p) => Some(DropTarget::Folder(p.clone())),
            Location::Trash => Some(DropTarget::Trash),
            Location::Recents => None,
        };
        let hot = target.is_some() && self.dragging_over(id, cx);
        if let Some(target) = target {
            self.drop_targets.push((id, target));
        }
        let accent = t.accent;
        let bg = if hot { Some(t.accent.alpha(0.22)) } else { selected.then_some(t.sidebar_selection) };
        let l = loc.clone();
        div()
            .id(id)
            .h(28.0)
            .shrink_0()
            .px(8.0)
            .rounded(6.0)
            .flex_row()
            .items_center()
            .gap(7.0)
            .bg_opt(bg)
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.go(l.clone(), cx))
            .child(canvas(move |s, b| icon(s, b, accent)).size(18.0).shrink_0())
            .child(text(name).truncate().flex_1())
            .child_if(detail.is_some(), || text(detail.clone().unwrap_or_default()).size(11.0).color(t.text_secondary).shrink_0().into())
    }

    // -- toolbar --------------------------------------------------------------

    fn title(&self) -> String {
        if self.searching() {
            return format!("Searching \u{201c}{}\u{201d}", self.query.trim());
        }
        match &self.loc {
            Location::Dir(p) => self.places.display_name(p, &self.root_name()),
            Location::Recents => "Recents".into(),
            Location::Trash => "Trash".into(),
        }
    }

    fn root_name(&self) -> String {
        self.mounts.iter().find(|m| m.path == Path::new("/")).map_or_else(|| self.disk_name.clone(), |m| m.name.clone())
    }

    fn tool_button(id: &'static str, t: &Theme, enabled: bool, icon: Icon, on_click: impl Fn(&mut Self, &mut Cx) + 'static) -> Div<Self> {
        let color = if enabled { t.icon } else { t.icon_disabled };
        let mut d = div().id(id).w(32.0).h(28.0).rounded(6.0).items_center().justify_center().child(canvas(move |s, b| icon(s, b, color)).size(18.0));
        if enabled {
            d = d.hover_bg(t.icon_hover_bg).on_click(on_click);
        }
        d
    }

    fn toolbar(&mut self, cx: &Cx, t: &Theme) -> Div<Self> {
        let nav = div()
            .flex_row()
            .items_center()
            .child(Self::tool_button("back", t, !self.back.is_empty(), icons::chevron_left, |s, cx| s.go_back(cx)))
            .child(Self::tool_button("forward", t, !self.forward.is_empty(), icons::chevron_right, |s, cx| s.go_forward(cx)));

        let mut seg = div().flex_row().items_center().h(28.0).p(2.0).rounded(8.0).bg(t.field);
        for (mode, icon) in [(ViewMode::Icons, icons::grid as Icon), (ViewMode::List, icons::list), (ViewMode::Columns, icons::columns)] {
            let on = self.view == mode;
            let color = if on { t.text } else { t.icon };
            let mut s = div()
                .id(("view-mode", mode.name()))
                .w(34.0)
                .h(24.0)
                .rounded(6.0)
                .items_center()
                .justify_center()
                .on_click(move |s: &mut Self, _: &mut Cx| s.set_view(mode))
                .child(canvas(move |sc, b| icon(sc, b, color)).size(16.0));
            if on {
                s = s.bg(t.segment_selected).shadow(rgba(0x000000, 0.12), 2.0, 0.5);
            }
            seg = seg.child(s);
        }

        let icon_c = t.icon;
        let action = div()
            .id("action-menu")
            .h(28.0)
            .px(6.0)
            .rounded(6.0)
            .flex_row()
            .items_center()
            .gap(1.0)
            .hover_bg(t.icon_hover_bg)
            .on_click(|s: &mut Self, cx: &mut Cx| {
                let at = Point { x: cx.mouse.x - 20.0, y: TOOLBAR_H - 6.0 };
                s.action_menu(at);
            })
            .child(canvas(move |s, b| icons::ellipsis_circle(s, b, icon_c)).size(18.0))
            .child(canvas(move |s, b| icons::chevron_down(s, b, icon_c)).size(10.0));

        let query_set = !self.query.is_empty();
        let search = div()
            .id("search-box")
            .w(180.0)
            .h(28.0)
            .rounded(7.0)
            .bg(t.field)
            .flex_row()
            .items_center()
            .gap(6.0)
            .px(8.0)
            .cursor(CursorIcon::Text)
            .on_mouse_down(|_: &mut Self, cx: &mut Cx| cx.focus("search"))
            .child(canvas(move |s, b| icons::magnifier(s, b, icon_c)).size(14.0).shrink_0())
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
                        s.search.borrow_mut().set_text(&mut cx.text.fonts, "");
                        s.on_input(eid("search"), cx);
                        cx.focus("content");
                    })
                    .child(canvas(|s, b| icons::xmark(s, b, Color::WHITE)).size(16.0))
                    .into()
            });

        div()
            .h(TOOLBAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .gap(8.0)
            .pl(10.0)
            .pr(12.0)
            .window_drag()
            .child(nav)
            .child(text(self.title()).size(15.0).weight(Weight::Bold).truncate().flex_1())
            .child(seg)
            .child(action)
            .child(search)
            .children(ui::chrome::window_buttons(false, cx, t.text))
    }

    // -- main column ------------------------------------------------------------

    fn main(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        // The content area's place follows from the fixed chrome around it;
        // the virtualized views and the rubber band need it.
        let (w, h) = cx.window_size;
        let top = TOOLBAR_H + 1.0 + if self.in_trash() && !self.searching() { TRASH_BAR_H } else { 0.0 };
        let x = SIDEBAR_W + 1.0;
        self.content_bounds.set(Bounds::new(x, top, (w - x).max(0.0), (h - top - PATH_BAR_H - STATUS_H).max(0.0)));
        let mut content = div().id("content").focusable().flex_1().min_w_0().flex_col().clip();
        let view = if self.searching() && self.view == ViewMode::Columns { ViewMode::List } else { self.view };
        let view = if view == ViewMode::Columns && self.current_dir().is_none() { ViewMode::List } else { view };
        content = match view {
            ViewMode::Icons => content.child(self.icon_view(cx, t)),
            ViewMode::List => content.child(self.list_view(cx, t)),
            ViewMode::Columns => content.child(self.column_view(cx, t)),
        };
        if self.searching() && self.entries.is_empty() {
            let label = if self.searching_busy { "Searching\u{2026}" } else { "No Results" };
            content = content.child({ let vp = self.content_bounds.get(); div().absolute(0.0, 0.0).w(vp.w).h(vp.h) }.items_center().justify_center().child(text(label).size(15.0).color(t.text_tertiary)));
        }

        let mut col = div().flex_1().min_w_0().h_full().flex_col().bg(t.content).child(self.toolbar(cx, t)).child(div().h(1.0).shrink_0().bg(t.divider));
        if self.in_trash() && !self.searching() {
            col = col.child(self.trash_bar(t));
        }
        col.child(content).child(self.path_bar(cx, t)).child(self.status_bar(t))
    }

    fn trash_bar(&self, t: &Theme) -> Div<Self> {
        let empty = self.trash_is_empty();
        div()
            .h(TRASH_BAR_H)
            .shrink_0()
            .flex_row()
            .items_center()
            .justify_between()
            .px(14.0)
            .bg(t.sidebar)
            .border_b(t.divider)
            .child(text("Trash").weight(Weight::Semibold))
            .child(
                div()
                    .id("empty-trash")
                    .h(22.0)
                    .px(12.0)
                    .rounded(6.0)
                    .items_center()
                    .bg(t.field)
                    .hover_bg(t.hover)
                    .on_click(move |s: &mut Self, _: &mut Cx| {
                        if !empty {
                            s.dialog = Some(Dialog::EmptyTrash);
                        }
                    })
                    .child(text("Empty").size(12.0).color(if empty { t.text_tertiary } else { t.text })),
            )
    }

    fn path_bar(&mut self, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut bar = div().h(PATH_BAR_H).shrink_0().flex_row().items_center().px(10.0).gap(2.0).border_b(t.divider);
        let mut segments: Vec<(String, Option<PathBuf>, Kind)> = Vec::new();
        match &self.loc {
            Location::Dir(dir) => {
                for a in dir.ancestors().collect::<Vec<_>>().into_iter().rev() {
                    let name = self.places.display_name(a, &self.root_name());
                    segments.push((name, Some(a.to_path_buf()), Kind::Folder));
                }
            }
            Location::Recents => segments.push(("Recents".into(), None, Kind::Folder)),
            Location::Trash => segments.push(("Trash".into(), None, Kind::Folder)),
        }
        if let Some(e) = self.single_selected().filter(|e| !self.searching() && e.path.parent() == self.current_dir()) {
            segments.push((e.name.clone(), None, e.kind));
        }
        let sep = t.text_tertiary;
        let n = segments.len();
        for (i, (name, path, kind)) in segments.into_iter().enumerate() {
            if i > 0 {
                bar = bar.child(canvas(move |s, b| icons::chevron_small(s, b, sep)).w(12.0).h(14.0).shrink_0());
            }
            let icon: AnyElement<Self> = if i == 0 && path.as_deref() == Some(Path::new("/")) {
                let c = t.icon;
                canvas(move |s, b| icons::drive(s, b, c)).size(14.0).shrink_0().into()
            } else if kind == Kind::Folder {
                canvas(|s, b| icons::folder(s, b, None)).size(16.0).shrink_0().into()
            } else {
                canvas(move |s, b| icons::page(s, b, kind)).size(16.0).shrink_0().into()
            };
            let mut seg = div().flex_row().items_center().gap(4.0).h(20.0).px(4.0).rounded(4.0).min_w_0().child(icon).child(text(name).size(11.5).color(t.text_secondary).truncate());
            if i == n - 1 {
                seg = seg.shrink_0().max_w(260.0);
            }
            if let Some(p) = path {
                let id = eid(("path", &p));
                let hot = self.dragging_over(id, cx);
                self.drop_targets.push((id, DropTarget::Folder(p.clone())));
                seg = seg
                    .id(id)
                    .hover_bg(t.hover)
                    .bg_opt(hot.then_some(t.accent.alpha(0.22)))
                    .on_click(move |s: &mut Self, cx: &mut Cx| s.go(Location::Dir(p.clone()), cx));
            }
            bar = bar.child(seg);
        }
        bar
    }

    fn status_bar(&self, t: &Theme) -> Div<Self> {
        let n = self.entries.len();
        let k = self.selection.len();
        let mut s = if k > 0 { format!("{k} of {n} selected") } else { fs::count_items(n) };
        let dir = self.current_dir().map(Path::to_path_buf).unwrap_or_else(|| self.places.home.clone());
        if let Some((free, _)) = super::places::space(&dir) {
            s.push_str(&format!(", {} available", format_size(free)));
        }
        div().h(STATUS_H).shrink_0().items_center().justify_center().child(text(s).size(11.0).color(t.text_secondary))
    }

    // -- icons ----------------------------------------------------------------

    fn emblem(&self, path: &Path) -> Option<Icon> {
        let p = &self.places;
        [(&p.desktop, icons::desktop as Icon), (&p.documents, icons::document), (&p.downloads, icons::download), (&p.pictures, icons::photo), (&p.music, icons::music), (&p.videos, icons::film), (&p.home, icons::house)]
            .into_iter()
            .find(|(d, _)| d.as_path() == path)
            .map(|(_, i)| i)
    }

    /// A file's icon: a thumbnail when there is one, the folder, or a page
    /// labeled with the extension.
    fn file_icon(&mut self, e: &Entry, size: f32) -> Div<Self> {
        let d = self.file_icon_inner(e, size);
        if e.is_link {
            return d.child(canvas(icons::alias_badge).absolute(0.0, 0.0).size(size));
        }
        d
    }

    fn file_icon_inner(&mut self, e: &Entry, size: f32) -> Div<Self> {
        let thumb = self.thumbnail(&e.path, e.modified, e.kind);
        let mut d = div().size(size).shrink_0();
        if e.is_dir {
            let emblem = self.emblem(&e.path);
            return d.child(canvas(move |s, b| icons::folder(s, b, emblem)).size(size));
        }
        if let Some(img) = thumb {
            let pdf = e.kind == Kind::Pdf;
            return d.child(canvas(move |s, b| draw_thumbnail(s, b, &img, pdf)).size(size));
        }
        let kind = e.kind;
        d = d.child(canvas(move |s, b| icons::page(s, b, kind)).size(size));
        if size >= 32.0 {
            if let Some(ext) = media::kind::extension(&e.name).filter(|x| x.len() <= 5) {
                let page = icons::page_rect(Bounds::new(0.0, 0.0, size, size));
                let fs = (size * 0.14).round();
                d = d.child(
                    div()
                        .absolute(page.x, page.y + page.h * 0.66)
                        .w(page.w)
                        .justify_center()
                        .child(text(ext.to_uppercase()).size(fs).weight(Weight::Bold).color(icons::label_color(kind))),
                );
            }
        }
        d
    }

    /// Splits a name into at most two centered lines that fit `width`, the
    /// second shortened in the middle like Finder ("Very long fi…name.txt").
    fn label_lines(&mut self, cx: &mut Cx, name: &str, width: f32) -> Vec<String> {
        let key = (name.to_string(), width as u32);
        if let Some(v) = self.labels.get(&key) {
            return v.clone();
        }
        let style = TextStyle { size: 12.0, line_height: 1.3, weight: Weight::Regular, color: Color::BLACK };
        let mut fits = |s: &str| cx.text.measure(s, &style, None, false).0 <= width;
        let lines = if fits(name) {
            vec![name.to_string()]
        } else {
            let chars: Vec<char> = name.chars().collect();
            // The longest prefix that fits, preferring a break after a space,
            // dash, underscore or dot.
            let mut cut = 1;
            for i in 1..chars.len() {
                if fits(&chars[..i].iter().collect::<String>()) {
                    cut = i;
                } else {
                    break;
                }
            }
            // Break after a space if possible, else a dash or underscore, else
            // a dot that isn't the extension's.
            let ext_dot = chars.iter().rposition(|&c| c == '.');
            let at = |pred: &dyn Fn(usize) -> bool| (cut / 3..=cut).rev().find(|&i| i > 0 && pred(i - 1));
            let space = at(&|i| chars[i] == ' ');
            let dash = at(&|i| matches!(chars[i], '-' | '_'));
            let dot = at(&|i| chars[i] == '.' && Some(i) != ext_dot);
            if let Some(b) = space.or(dash).or(dot) {
                cut = b;
            }
            let first: String = chars[..cut].iter().collect::<String>().trim_end().to_string();
            let rest: Vec<char> = chars[cut..].to_vec();
            let rest_s: String = rest.iter().collect();
            let second = if fits(&rest_s) {
                rest_s
            } else {
                // Keep the end (often the extension) and as much of the start as fits.
                let tail = 7.min(rest.len());
                let end: String = rest[rest.len() - tail..].iter().collect();
                let mut keep = 0;
                for i in 0..rest.len() - tail {
                    let s = format!("{}\u{2026}{}", rest[..i].iter().collect::<String>(), end);
                    if fits(&s) {
                        keep = i;
                    } else {
                        break;
                    }
                }
                format!("{}\u{2026}{}", rest[..keep].iter().collect::<String>(), end)
            };
            vec![first, second]
        };
        if self.labels.len() > 4000 {
            self.labels.clear();
        }
        self.labels.insert(key, lines.clone());
        lines
    }

    fn rename_field(&self, t: &Theme, w: Option<f32>) -> Div<Self> {
        let mut d = div().h(20.0).px(3.0).items_center().rounded(4.0).bg(t.content).border(t.accent, 1.5).child(input("rename", &self.rename).flex_1());
        d = match w {
            Some(w) => d.w(w),
            None => d.flex_1(),
        };
        d
    }

    fn content_focused(&self, cx: &Cx) -> bool {
        cx.window_focused && (cx.is_focused("content") || cx.is_focused("rename"))
    }

    fn icon_view(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let vp = self.content_bounds.get();
        let width = vp.w;
        let view_h = vp.h;
        let (cols, cell_w) = icon_layout(width);
        self.icon_cols = cols;
        let n = self.entries.len();
        let rows = n.div_ceil(cols);

        if std::mem::take(&mut self.reveal) {
            if let Some(&i) = self.lead.as_ref().and_then(|p| self.index.get(p)) {
                let y = GRID_PAD + (i / cols) as f32 * CELL_H;
                let off = reveal_offset(cx.scroll_y("icons-scroll"), view_h, y, CELL_H);
                cx.set_scroll_y("icons-scroll", off);
            }
        }
        let scroll = cx.scroll_y("icons-scroll");
        let (first, last) = visible_rows(scroll, view_h, GRID_PAD, CELL_H, rows);
        let focused = self.content_focused(cx);

        let mut body = div().flex_col().pt(GRID_PAD + first as f32 * CELL_H).pb(GRID_PAD + (rows - last) as f32 * CELL_H).px(GRID_PAD);
        for r in first..last {
            let mut row = div().flex_row().h(CELL_H);
            let items: Vec<Entry> = self.entries[r * cols..((r + 1) * cols).min(n)].to_vec();
            for e in items {
                row = row.child(self.icon_cell(&e, cell_w, focused, cx, t));
            }
            body = body.child(row);
        }

        let mut view = div()
            .id("icons-scroll")
            .scroll_y()
            .flex_1()
            .flex_col()
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.background_mouse_down(Point { x: vp.x, y: vp.y - scroll }, cx))
            .on_right_click(|s: &mut Self, cx: &mut Cx| s.background_menu(cx))
            .child(body);
        if let Some(band) = &self.band {
            let r = Bounds::new(band.start.x.min(band.end.x), band.start.y.min(band.end.y), (band.start.x - band.end.x).abs(), (band.start.y - band.end.y).abs());
            let (fill, edge) = (t.accent.alpha(0.15), t.accent.alpha(0.7));
            view = view.child(
                canvas(move |s, b| {
                    // The canvas scrolls with the content, like the band's coordinates.
                    let rect = Bounds::new(b.x + r.x, b.y + r.y, r.w, r.h);
                    s.rect_bordered(rect, fill, 0.0, edge, 1.0);
                })
                .absolute(0.0, 0.0).w_full().h_full(),
            );
        }
        view
    }

    fn icon_cell(&mut self, e: &Entry, cell_w: f32, focused: bool, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let selected = self.selection.contains(&e.path);
        let icon_id = eid(("icon", &e.path));
        let label_id = eid(("label", &e.path));
        let target = e.is_dir && !self.drag.as_ref().is_some_and(|d| d.paths.contains(&e.path));
        let hot = target && (self.dragging_over(icon_id, cx) || self.dragging_over(label_id, cx));
        if target {
            self.drop_targets.push((icon_id, DropTarget::Folder(e.path.clone())));
            self.drop_targets.push((label_id, DropTarget::Folder(e.path.clone())));
        }

        let icon_bg = if hot { Some(t.accent.alpha(0.25)) } else { selected.then_some(t.icon_selection) };
        let icon = self.with_item_handlers(div().id(icon_id).size(ICON + 8.0).rounded(7.0).items_center().justify_center().bg_opt(icon_bg), &e.path).child(self.file_icon(e, ICON));

        let label_w = cell_w - 10.0;
        let label: Div<Self> = if self.renaming.as_ref() == Some(&e.path) {
            div().mt(4.0).child(self.rename_field(t, Some(label_w)))
        } else {
            let lines = self.label_lines(cx, &e.name, label_w - 8.0);
            let (bg, fg) = match (selected, focused) {
                (true, true) => (Some(t.accent), Color::WHITE),
                (true, false) => (Some(t.selection_unfocused), t.text),
                _ => (None, t.text),
            };
            let mut l = div().id(label_id).mt(4.0).flex_col().items_center();
            for line in lines {
                l = l.child(div().h(LABEL_LINE).px(4.0).rounded(4.0).items_center().bg_opt(bg).child(text(line).size(12.0).color(fg)));
            }
            self.with_item_handlers(l, &e.path)
        };
        div().w(cell_w).h(CELL_H).flex_col().items_center().pt(2.0).child(icon).child(label)
    }

    fn with_item_handlers(&self, d: Div<Self>, path: &Path) -> Div<Self> {
        let (a, b, c) = (path.to_path_buf(), path.to_path_buf(), path.to_path_buf());
        d.on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.item_mouse_down(a.clone(), cx))
            .on_double_click(move |s: &mut Self, cx: &mut Cx| s.open_entry(b.clone(), cx))
            .on_right_click(move |s: &mut Self, cx: &mut Cx| s.item_menu(c.clone(), cx))
    }

    // -- list -----------------------------------------------------------------

    fn list_header(&self, t: &Theme, show_kind: bool) -> Div<Self> {
        let cell = |key: SortKey, w: Option<f32>, right: bool| -> Div<Self> {
            let on = self.sort == key;
            let c = if on { t.text } else { t.text_secondary };
            let up = self.ascending;
            let mut d = div()
                .id(("sort", key.label()))
                .h_full()
                .flex_row()
                .items_center()
                .gap(4.0)
                .px(8.0)
                .hover_bg(t.hover)
                .on_click(move |s: &mut Self, _: &mut Cx| s.set_sort(key));
            d = match w {
                Some(w) => d.w(w).shrink_0(),
                None => d.flex_1().min_w_0(),
            };
            if right {
                d = d.justify_end();
            }
            d = d.child(text(key.label()).size(11.5).weight(if on { Weight::Semibold } else { Weight::Regular }).color(c).truncate());
            if on {
                let ic: Icon = if up { icons::chevron_up } else { icons::chevron_down };
                d = d.child(canvas(move |s, b| ic(s, b, c)).size(10.0).shrink_0());
            }
            d
        };
        let sep = || div().w(1.0).h(14.0).bg(t.divider).shrink_0();
        let mut h = div()
            .h(28.0)
            .shrink_0()
            .flex_row()
            .items_center()
            .px(10.0)
            .border_b(t.divider)
            .child(cell(SortKey::Name, None, false).pl(30.0))
            .child(sep())
            .child(cell(SortKey::Modified, Some(DATE_W), false))
            .child(sep())
            .child(cell(SortKey::Size, Some(SIZE_W), true));
        if show_kind {
            h = h.child(sep()).child(cell(SortKey::Kind, Some(KIND_W), false));
        }
        h
    }

    fn list_view(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let vp = self.content_bounds.get();
        let show_kind = vp.w > 560.0;
        let view_h = vp.h - 28.0;
        let n = self.entries.len();
        let top = 4.0;
        if std::mem::take(&mut self.reveal) {
            if let Some(&i) = self.lead.as_ref().and_then(|p| self.index.get(p)) {
                let off = reveal_offset(cx.scroll_y("list-scroll"), view_h, top + i as f32 * ROW_H, ROW_H);
                cx.set_scroll_y("list-scroll", off);
            }
        }
        let scroll = cx.scroll_y("list-scroll");
        let (first, last) = visible_rows(scroll, view_h, top, ROW_H, n);
        let focused = self.content_focused(cx);
        let now = SystemTime::now();

        let mut body = div().flex_col().pt(top + first as f32 * ROW_H).pb(8.0 + (n - last) as f32 * ROW_H);
        let items: Vec<(usize, Entry)> = (first..last).map(|i| (i, self.entries[i].clone())).collect();
        for (i, e) in items {
            body = body.child(self.list_row(i, &e, focused, show_kind, now, cx, t));
        }
        div()
            .flex_1()
            .flex_col()
            .min_h(0.0)
            .child(self.list_header(t, show_kind))
            .child(
                div()
                    .id("list-scroll")
                    .scroll_y()
                    .flex_1()
                    .flex_col()
                    .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.background_mouse_down(Point::default(), cx))
                    .on_right_click(|s: &mut Self, cx: &mut Cx| s.background_menu(cx))
                    .child(body),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn list_row(&mut self, i: usize, e: &Entry, focused: bool, show_kind: bool, now: SystemTime, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let selected = self.selection.contains(&e.path);
        let id = eid(("row", &e.path));
        let target = e.is_dir && !self.drag.as_ref().is_some_and(|d| d.paths.contains(&e.path));
        let hot = target && self.dragging_over(id, cx);
        if target {
            self.drop_targets.push((id, DropTarget::Folder(e.path.clone())));
        }
        let (bg, fg, fg2) = match (selected, focused) {
            _ if hot => (Some(t.accent.alpha(0.25)), t.text, t.text_secondary),
            (true, true) => (Some(t.accent), Color::WHITE, rgba(0xffffff, 0.85)),
            (true, false) => (Some(t.selection_unfocused), t.text, t.text_secondary),
            _ => ((i % 2 == 1).then_some(t.stripe), t.text, t.text_secondary),
        };
        let name: AnyElement<Self> = if self.renaming.as_ref() == Some(&e.path) {
            self.rename_field(t, None).into()
        } else {
            text(e.name.clone()).color(fg).truncate().flex_1().into()
        };
        let date = e.modified.map(|m| fs::format_date(m, now)).unwrap_or_else(|| "--".into());
        let size = if e.is_dir { "--".to_string() } else { format_size(e.size) };
        let mut row = div()
            .id(id)
            .h(ROW_H)
            .shrink_0()
            .mx(10.0)
            .rounded(5.0)
            .flex_row()
            .items_center()
            .bg_opt(bg)
            .child(div().flex_1().min_w_0().flex_row().items_center().gap(6.0).px(8.0).child(self.file_icon(e, 16.0)).child(name))
            .child(text(date).size(12.0).color(fg2).truncate().w(DATE_W).px(8.0).shrink_0())
            .child(div().w(SIZE_W).shrink_0().px(8.0).justify_end().child(text(size).size(12.0).color(fg2)));
        if show_kind {
            row = row.child(text(e.kind_name.clone()).size(12.0).color(fg2).truncate().w(KIND_W).px(8.0).shrink_0());
        }
        self.with_item_handlers(row, &e.path)
    }

    // -- columns --------------------------------------------------------------

    fn column_view(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let vp = self.content_bounds.get();
        let avail = vp.w;
        let view_h = vp.h;
        let focused = self.content_focused(cx);
        let chain = self.column_dirs();
        let reveal = std::mem::take(&mut self.reveal);

        let mut cols: Vec<Div<Self>> = Vec::new();
        let mut total = 0.0;
        for (ci, dir) in chain.iter().enumerate() {
            let active = ci == chain.len() - 1;
            let entries: Vec<Entry> = if active { self.entries.clone() } else { self.listing(dir).to_vec() };
            let selected_child = chain.get(ci + 1).cloned();
            cols.push(self.column(dir, &entries, ColumnRole::Chain { active, selected_child }, focused, view_h, reveal, cx, t));
            total += COL_W;
        }
        // What the selection opens onto: a folder's contents or a file's preview.
        if let Some(e) = self.single_selected().cloned() {
            if e.is_dir {
                let entries = self.listing(&e.path).to_vec();
                cols.push(self.column(&e.path, &entries, ColumnRole::Next, focused, view_h, false, cx, t));
                total += COL_W;
            } else {
                cols.push(self.preview_column(&e, t));
                total += PREVIEW_COL_W;
            }
        }
        // Keep the rightmost columns in view; sideways scrolling looks back.
        let overflow = (total - avail).max(0.0);
        self.col_scroll = self.col_scroll.min(overflow);
        let offset = overflow - self.col_scroll;
        div().flex_1().clip().child(div().absolute(-offset, 0.0).h_full().flex_row().children(cols))
    }

    #[allow(clippy::too_many_arguments)]
    fn column(&mut self, dir: &Path, entries: &[Entry], role: ColumnRole, focused: bool, view_h: f32, reveal: bool, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let scroll_id = eid(("col", dir));
        let n = entries.len();
        let top = 4.0;
        let wanted = match &role {
            ColumnRole::Chain { active: true, .. } => self.lead.clone(),
            ColumnRole::Chain { selected_child, .. } => selected_child.clone(),
            ColumnRole::Next => None,
        };
        if reveal {
            if let Some(i) = wanted.as_ref().and_then(|w| entries.iter().position(|e| e.path == *w)) {
                let off = reveal_offset(cx.scroll_y(scroll_id), view_h, top + i as f32 * ROW_H, ROW_H);
                cx.set_scroll_y(scroll_id, off);
            }
        }
        let (first, last) = visible_rows(cx.scroll_y(scroll_id), view_h, top, ROW_H, n);
        let mut body = div().flex_col().pt(top + first as f32 * ROW_H).pb(8.0 + (n - last) as f32 * ROW_H);
        for e in &entries[first..last] {
            body = body.child(self.column_row(dir, e, &role, focused, cx, t));
        }
        let d = dir.to_path_buf();
        let parent_of_next = matches!(role, ColumnRole::Next);
        let list = div()
            .id(scroll_id)
            .scroll_y()
            .flex_1()
            .h_full()
            .flex_col()
            .on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
                s.menu = None;
                if parent_of_next {
                    s.enter_column(d.clone(), None);
                } else if s.current_dir() != Some(d.as_path()) {
                    let child = s.column_dirs().into_iter().find(|c| c.parent() == Some(d.as_path()));
                    s.enter_column(d.clone(), child);
                } else {
                    s.clear_selection();
                }
                cx.focus("content");
            })
            .on_right_click(|s: &mut Self, cx: &mut Cx| s.background_menu(cx))
            .child(body);
        div().w(COL_W).h_full().shrink_0().flex_row().child(list).child(div().w(1.0).h_full().bg(t.divider))
    }

    fn column_row(&mut self, dir: &Path, e: &Entry, role: &ColumnRole, focused: bool, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let id = eid(("colrow", &e.path));
        let (selected, accent) = match role {
            ColumnRole::Chain { active: true, .. } => (self.selection.contains(&e.path), focused),
            ColumnRole::Chain { selected_child, .. } => (selected_child.as_ref() == Some(&e.path), false),
            ColumnRole::Next => (false, false),
        };
        let target = e.is_dir && !self.drag.as_ref().is_some_and(|d| d.paths.contains(&e.path));
        let hot = target && self.dragging_over(id, cx);
        if target {
            self.drop_targets.push((id, DropTarget::Folder(e.path.clone())));
        }
        let (bg, fg) = match (selected, accent) {
            _ if hot => (Some(t.accent.alpha(0.25)), t.text),
            (true, true) => (Some(t.accent), Color::WHITE),
            (true, false) => (Some(t.selection_unfocused), t.text),
            _ => (None, t.text),
        };
        let name: AnyElement<Self> = if self.renaming.as_ref() == Some(&e.path) {
            self.rename_field(t, None).into()
        } else {
            text(e.name.clone()).color(fg).truncate().flex_1().into()
        };
        let chevron = if selected && accent { Color::WHITE } else { t.text_secondary };
        let row = div()
            .id(id)
            .h(ROW_H)
            .shrink_0()
            .mx(6.0)
            .px(6.0)
            .rounded(5.0)
            .flex_row()
            .items_center()
            .gap(6.0)
            .bg_opt(bg)
            .child(self.file_icon(e, 16.0))
            .child(name)
            .child_if(e.is_dir, || canvas(move |s, b| icons::chevron_small(s, b, chevron)).w(10.0).h(14.0).shrink_0().into());
        let (p1, p2, p3) = (e.path.clone(), e.path.clone(), e.path.clone());
        let (d1, d2) = (dir.to_path_buf(), dir.to_path_buf());
        let active = matches!(role, ColumnRole::Chain { active: true, .. });
        row.on_mouse_down(move |s: &mut Self, cx: &mut Cx| {
            if !active {
                s.enter_column(d1.clone(), None);
            }
            s.item_mouse_down(p1.clone(), cx);
        })
        .on_double_click(move |s: &mut Self, cx: &mut Cx| {
            if s.current_dir() != Some(d2.as_path()) {
                s.enter_column(d2.clone(), Some(p2.clone()));
            }
            s.open_entry(p2.clone(), cx);
        })
        .on_right_click(move |s: &mut Self, cx: &mut Cx| s.item_menu(p3.clone(), cx))
    }

    fn preview_column(&mut self, e: &Entry, t: &Theme) -> Div<Self> {
        let info = |label: &str, value: String| -> AnyElement<Self> {
            div()
                .w_full()
                .flex_row()
                .gap(8.0)
                .child(text(label.to_string()).size(11.0).color(t.text_secondary).w(58.0).shrink_0())
                .child(text(value).size(11.0).flex_1())
                .into()
        };
        let mut col = div()
            .w(PREVIEW_COL_W)
            .h_full()
            .shrink_0()
            .flex_col()
            .items_center()
            .px(18.0)
            .pt(24.0)
            .gap(8.0)
            .child(self.file_icon(e, 128.0))
            .child(text(e.name.clone()).size(13.0).weight(Weight::Semibold).mt(6.0))
            .child(text(format!("{} \u{2013} {}", e.kind_name, format_size(e.size))).size(11.0).color(t.text_secondary))
            .child(div().h(1.0).w_full().bg(t.divider).mt(6.0))
            .child(div().w_full().child(text("Information").size(12.0).weight(Weight::Semibold)));
        if let Some(c) = e.created {
            col = col.child(info("Created", fs::format_date_long(c)));
        }
        if let Some(m) = e.modified {
            col = col.child(info("Modified", fs::format_date_long(m)));
        }
        if let Some(item) = self.trash_holding(&e.path).and_then(|t| t.items().into_iter().find(|i| i.path == e.path)) {
            if let Some(dir) = item.original.as_deref().and_then(Path::parent) {
                col = col.child(info("Where", dir.to_string_lossy().into_owned()));
            }
            if let Some(d) = item.deleted {
                col = col.child(info("Deleted", d.format("%A, %-d %B %Y at %-I:%M %p").to_string()));
            }
        }
        col
    }

    // -- overlays ---------------------------------------------------------------

    fn drag_ghost(&mut self, cx: &mut Cx) -> Option<Div<Self>> {
        let drag = self.drag.as_ref()?;
        let lead = self.press.as_ref().map(|p| p.path.clone()).or_else(|| drag.paths.first().cloned())?;
        let count = drag.paths.len();
        let e = self.index.get(&lead).map(|&i| self.entries[i].clone()).or_else(|| Entry::from_path(&lead))?;
        let icon = self.file_icon(&e, 48.0);
        let mut ghost = div().absolute(cx.mouse.x - 24.0, cx.mouse.y - 24.0).size(48.0).child(icon);
        if count > 1 {
            ghost = ghost.child(
                div()
                    .absolute(34.0, -6.0)
                    .h(18.0)
                    .px(5.0)
                    .rounded(9.0)
                    .bg(rgb(0xff3b30))
                    .items_center()
                    .justify_center()
                    .child(text(count.to_string()).size(11.0).weight(Weight::Bold).color(Color::WHITE)),
            );
        }
        Some(div().inset_0().deferred().child(ghost))
    }

    fn progress_panel(&self, t: &Theme) -> Option<Div<Self>> {
        let job = self.jobs.iter().find(|j| j.started.elapsed() >= super::PROGRESS_DELAY)?;
        let p = &job.progress;
        let (total, done) = (p.total.load(Ordering::Relaxed), p.done.load(Ordering::Relaxed));
        let verb = match job.op {
            Op::Copy => "Copying",
            Op::Move => "Moving",
            Op::Duplicate => "Duplicating",
        };
        let dest = job.dest.file_name().map_or_else(|| job.dest.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
        let what = if job.count == 1 { format!("\u{201c}{}\u{201d}", p.current.lock().unwrap()) } else { format!("{} items", job.count) };
        let frac = if total > 0 { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
        let detail = if total > 0 { format!("{} of {}", format_size(done), format_size(total)) } else { "Preparing\u{2026}".into() };
        let (track, fill) = (t.field, t.accent);
        let progress = p.clone();
        let card = div()
            .w(340.0)
            .flex_row()
            .items_center()
            .gap(12.0)
            .p(14.0)
            .rounded(12.0)
            .bg(t.panel)
            .border(t.menu_border, 0.5)
            .shadow(t.shadow, 18.0, 6.0)
            .occlude()
            .child(
                div()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(6.0)
                    .child(text(format!("{verb} {what} to \u{201c}{dest}\u{201d}")).size(12.0).weight(Weight::Semibold).truncate())
                    .child(canvas(move |s, b| {
                        let b = Bounds::new(b.x, b.y + 1.0, b.w, 6.0);
                        s.rect(b, track, 3.0);
                        s.rect(Bounds::new(b.x, b.y, (b.w * frac).max(6.0), b.h), fill, 3.0);
                    }).h(8.0).w_full())
                    .child(text(detail).size(11.0).color(t.text_secondary)),
            )
            .child(
                div()
                    .id("cancel-job")
                    .size(22.0)
                    .rounded(11.0)
                    .bg(t.field)
                    .hover_bg(t.hover)
                    .on_click(move |_: &mut Self, _: &mut Cx| progress.cancel.store(true, Ordering::Relaxed))
                    .child({
                        let c = t.text_secondary;
                        canvas(move |s, b| icons::xmark(s, b, c)).size(22.0)
                    }),
            );
        Some(div().inset_0().deferred().items_end().justify_end().pr(18.0).pb(60.0).child(card))
    }

    fn dialog_sheet(&self, dialog: &Dialog, cx: &Cx, t: &Theme) -> Div<Self> {
        let (title, info, action, destructive) = match dialog {
            Dialog::EmptyTrash => (
                "Are you sure you want to permanently erase the items in the Trash?".to_string(),
                "You can\u{2019}t undo this action.".to_string(),
                Some("Empty Trash"),
                true,
            ),
            Dialog::DeleteNow(paths) => {
                let what = match paths.as_slice() {
                    [p] => format!("\u{201c}{}\u{201d}", p.file_name().unwrap_or_default().to_string_lossy()),
                    _ => format!("these {} items", paths.len()),
                };
                (format!("Are you sure you want to delete {what}?"), "This item will be deleted immediately. You can\u{2019}t undo this action.".to_string(), Some("Delete"), true)
            }
            Dialog::Error(e) => ("The operation can\u{2019}t be completed.".to_string(), e.clone(), None, false),
        };
        let button = |id: &'static str, label: &str, primary: bool| -> Div<Self> {
            let (bg, fg) = if primary { (t.accent, Color::WHITE) } else { (t.field, t.text) };
            div().id(id).h(28.0).flex_1().rounded(7.0).bg(bg).items_center().justify_center().child(text(label.to_string()).color(fg).weight(Weight::Medium))
        };
        let mut buttons = div().flex_row().gap(8.0).w_full().mt(6.0);
        match action {
            Some(a) => {
                buttons = buttons.child(button("dialog-cancel", "Cancel", false).on_click(|s: &mut Self, cx: &mut Cx| {
                    s.dialog = None;
                    cx.focus("content");
                }));
                let mut confirm = button("dialog-ok", a, true).on_click(|s: &mut Self, cx: &mut Cx| s.confirm_dialog(cx));
                if destructive {
                    confirm = confirm.bg(t.destructive);
                }
                buttons = buttons.child(confirm);
            }
            None => {
                buttons = buttons.child(button("dialog-ok", "OK", true).on_click(|s: &mut Self, cx: &mut Cx| {
                    s.dialog = None;
                    cx.focus("content");
                }));
            }
        }
        let accent = t.accent;
        let card = div()
            .w(300.0)
            .flex_col()
            .items_center()
            .px(18.0)
            .pt(20.0)
            .pb(16.0)
            .gap(8.0)
            .rounded(14.0)
            .bg(t.panel)
            .border(t.menu_border, 0.5)
            .shadow(t.shadow, 30.0, 10.0)
            .occlude()
            .child(if matches!(dialog, Dialog::Error(_)) {
                canvas(|s, b| icons::folder(s, b, None)).size(56.0)
            } else {
                canvas(move |s, b| icons::trash(s, b, accent)).size(48.0)
            })
            .child(text(title).size(13.0).weight(Weight::Bold).w_full())
            .child(text(info).size(11.5).color(t.text_secondary).w_full())
            .child(buttons);
        div()
            .inset_0()
            .deferred()
            .bg(rgba(0x000000, if cx.dark { 0.35 } else { 0.15 }))
            .items_center()
            .justify_center()
            .on_mouse_down(|_: &mut Self, _: &mut Cx| {})
            .child(card)
    }

    fn quicklook_panel(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let ql = self.quicklook.as_ref().unwrap();
        let path = ql.path.clone();
        let entry = self.index.get(&path).map(|&i| self.entries[i].clone()).or_else(|| Entry::from_path(&path));
        let (w, h) = cx.window_size;
        let (max_w, max_h) = ((w - 100.0).max(320.0), (h - 120.0).max(240.0));
        let header_h = 38.0;
        let name = entry.as_ref().map_or_else(String::new, |e| e.name.clone());

        let (pw, ph, body): (f32, f32, AnyElement<Self>) = match &ql.preview {
            Some(Preview::Image(img)) => {
                let (iw, ih) = (img.width() as f32 / cx.scale.max(1.0), img.height() as f32 / cx.scale.max(1.0));
                let s = (max_w / iw).min((max_h - header_h) / ih).min(1.0f32.max(360.0 / iw.max(ih)));
                let (bw, bh) = ((iw * s).max(360.0), ih * s);
                (bw, bh + header_h, ui::image(img, Fit::Scale, 0.0).w(bw).h(bh).into())
            }
            Some(Preview::Pages(pages, total)) => {
                // Each page fits the panel's height; scroll for the next.
                let aspect = pages[0].width() as f32 / pages[0].height().max(1) as f32;
                let page_h = max_h - header_h - 32.0;
                let pw = (page_h * aspect + 32.0).min(max_w).max(360.0);
                let mut list = div().id("ql-pages").scroll_y().flex_1().w_full().flex_col().items_center().gap(14.0).p(16.0).bg(t.preview_bg);
                for img in pages {
                    let pw_in = (pw - 32.0).min(page_h * img.width() as f32 / img.height().max(1) as f32);
                    let ph_in = pw_in * img.height() as f32 / img.width().max(1) as f32;
                    list = list.child(div().shadow(rgba(0x000000, 0.2), 6.0, 2.0).child(ui::image(img, Fit::Scale, 0.0).w(pw_in).h(ph_in)));
                }
                if *total as usize > pages.len() {
                    list = list.child(text(format!("{} more pages", *total as usize - pages.len())).size(12.0).color(t.text_secondary));
                }
                (pw, max_h, list.into())
            }
            Some(Preview::Text(s)) => {
                let pw = max_w.min(720.0);
                let body = div()
                    .id("ql-text")
                    .scroll_y()
                    .flex_1()
                    .w_full()
                    .flex_col()
                    .p(18.0)
                    .bg(t.content)
                    .child(text(s.clone()).size(12.0).line_height(1.45).w_full());
                (pw, max_h, body.into())
            }
            other => {
                let loading = other.is_none();
                let mut info = div().flex_1().flex_row().items_center().justify_center().gap(22.0).px(30.0);
                if let Some(e) = &entry {
                    let mut text_col = div().flex_col().gap(4.0).min_w_0().child(text(e.name.clone()).size(17.0).weight(Weight::Bold));
                    let size = if e.is_dir { String::new() } else { format!(" \u{2013} {}", format_size(e.size)) };
                    text_col = text_col.child(text(format!("{}{}", e.kind_name, size)).size(12.0).color(t.text_secondary));
                    if let Some(m) = e.modified {
                        text_col = text_col.child(text(format!("Modified {}", fs::format_date_long(m))).size(12.0).color(t.text_secondary));
                    }
                    if loading {
                        text_col = text_col.child(text("Loading preview\u{2026}").size(12.0).color(t.text_tertiary));
                    }
                    info = info.child(self.file_icon(e, 128.0)).child(text_col);
                }
                (520.0f32.min(max_w), 260.0, info.into())
            }
        };

        let c = t.text_secondary;
        let open_path = path.clone();
        let header = div()
            .h(header_h)
            .shrink_0()
            .flex_row()
            .items_center()
            .px(10.0)
            .gap(8.0)
            .border_b(t.divider)
            .child(
                div()
                    .id("ql-close")
                    .size(24.0)
                    .rounded(12.0)
                    .hover_bg(t.hover)
                    .on_click(|s: &mut Self, _: &mut Cx| s.quicklook = None)
                    .child(canvas(move |s, b| icons::xmark(s, b, c)).size(24.0)),
            )
            .child(div().flex_1().min_w_0().justify_center().child(text(name).weight(Weight::Semibold).truncate()))
            .child(
                div()
                    .id("ql-open")
                    .h(24.0)
                    .px(10.0)
                    .rounded(6.0)
                    .bg(t.field)
                    .hover_bg(t.hover)
                    .items_center()
                    .on_click(move |s: &mut Self, cx: &mut Cx| {
                        s.quicklook = None;
                        s.open_entry(open_path.clone(), cx);
                    })
                    .child(text("Open").size(12.0)),
            );
        let panel_w = pw.max(320.0);
        let (x, y) = ((w - panel_w) / 2.0 + SIDEBAR_W / 2.0, ((h - ph) / 2.0).max(10.0));
        let x = x.min(w - panel_w - 10.0).max(10.0);
        let panel = div()
            .absolute(x, y)
            .w(panel_w)
            .h(ph)
            .flex_col()
            .rounded(12.0)
            .bg(t.panel)
            .border(t.menu_border, 0.5)
            .shadow(t.shadow, 30.0, 12.0)
            .clip()
            .occlude()
            .child(header)
            .child(div().flex_1().min_h(0.0).flex_col().items_center().justify_center().child(body));
        div().inset_0().deferred().child(panel)
    }
}

enum ColumnRole {
    /// A folder on the way to the current one (`active` is the current one).
    Chain { active: bool, selected_child: Option<PathBuf> },
    /// The contents of the selected folder.
    Next,
}

/// A photo or PDF page drawn as an icon: fitted, on a white mat with a soft shadow.
fn draw_thumbnail(scene: &mut Scene, b: Bounds, img: &Image, pdf: bool) {
    let inset = (b.w * 0.04).max(0.5);
    let (r, _) = ui::fit_rect(b.inset(inset), (img.width() as f32, img.height() as f32), Fit::Scale);
    let mat = if b.w >= 32.0 { (b.w / 40.0).max(1.0) } else { 0.0 };
    let frame = r.inset(-mat);
    if b.w >= 32.0 {
        scene.shadow(Bounds::new(frame.x, frame.y + b.w * 0.012, frame.w, frame.h), rgba(0x000000, 0.22), 1.0, b.w * 0.025);
    }
    scene.rect_bordered(frame, Color::WHITE, 1.0, rgba(0x000000, if pdf { 0.18 } else { 0.10 }), 0.5);
    scene.image(r, img, 0.0);
}

fn menu_overlay(menu: &Menu, cx: &Cx, t: &Theme) -> Div<FilesApp> {
    let (w, h) = cx.window_size;
    let width = 250.0;
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
                let hovered = item.enabled && cx.hovered(("menu-item", i));
                let fg = if !item.enabled {
                    t.text_tertiary
                } else if hovered {
                    Color::WHITE
                } else {
                    t.text
                };
                let shortcut_c = if hovered { rgba(0xffffff, 0.8) } else { t.text_tertiary };
                let mut row = div()
                    .id(("menu-item", i))
                    .h(24.0)
                    .pl(4.0)
                    .pr(10.0)
                    .rounded(5.0)
                    .flex_row()
                    .items_center()
                    .child(div().w(18.0).shrink_0().child_if(item.checked, || canvas(move |s, b| icons::check(s, b, fg)).size(12.0).into()))
                    .child(text(item.label.clone()).color(fg).truncate().flex_1())
                    .child_if(!item.shortcut.is_empty(), || text(item.shortcut).size(12.0).color(shortcut_c).shrink_0().into());
                if item.enabled {
                    let action: Action = action.clone();
                    row = row.hover_bg(t.menu_highlight).on_click(move |s: &mut FilesApp, cx: &mut Cx| s.run(action.clone(), cx));
                }
                panel = panel.child(row);
            }
        }
    }
    div()
        .inset_0()
        .deferred()
        .on_mouse_down(|s: &mut FilesApp, _| s.menu = None)
        .on_right_click(|s: &mut FilesApp, _| s.menu = None)
        .child(panel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry() {
        let (cols, w) = icon_layout(700.0);
        assert_eq!(cols, 6);
        assert!((w * cols as f32 - (700.0 - 2.0 * GRID_PAD)).abs() < 0.01);
        assert_eq!(visible_rows(0.0, 240.0, 4.0, 24.0, 100), (0, 11));
        assert_eq!(visible_rows(480.0, 240.0, 4.0, 24.0, 100), (19, 31));
        assert_eq!(visible_rows(0.0, 240.0, 4.0, 24.0, 3), (0, 3));
        assert_eq!(reveal_offset(100.0, 200.0, 50.0, 24.0), 50.0);
        assert_eq!(reveal_offset(100.0, 200.0, 300.0, 24.0), 124.0);
        assert_eq!(reveal_offset(100.0, 200.0, 150.0, 24.0), 100.0);
    }
}
