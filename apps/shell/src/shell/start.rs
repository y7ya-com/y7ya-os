//! The Start menu, arranged like Windows 11's: a search field on top, a
//! grid of pinned apps with an "All apps" list one click away, recently
//! used files, and the user with the power buttons at the bottom. Typing
//! searches with Spotlight's engine.

use ui::*;

use super::dialogs::avatar;
use super::spotlight::letter_tile;
use super::theme::Theme;
use super::{Panel, Shell, StartView, icons, launch, shell_quote};
use crate::apps;
use crate::recent::RecentFile;
use crate::system::SessionAction;

const WIDTH: f32 = 640.0;
/// The content's side margins, and its width inside them.
const PAD: f32 = 28.0;
const INNER: f32 = WIDTH - 2.0 * PAD;
const BODY_H: f32 = 414.0;
const COLUMNS: usize = 6;
const TILE_W: f32 = INNER / COLUMNS as f32;
const TILE_H: f32 = 86.0;
const ICON: f32 = 44.0;
/// At most this many rows of pins show; more scroll.
const PIN_ROWS: usize = 3;
const SECTION_H: f32 = 36.0;
const RECENT_H: f32 = 50.0;
const ROW_H: f32 = 34.0;
const LETTER_H: f32 = 30.0;
const FOOTER_H: f32 = 62.0;
const CTX_W: f32 = 196.0;

type Glyph = fn(&mut Scene, Bounds, Color);

#[derive(Clone, Copy)]
enum Power {
    Lock,
    Sleep,
    LogOut,
    Restart,
    ShutDown,
}

impl Shell {
    pub(super) fn start_menu_view(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (_, h) = cx.window_size;
        let body_h = BODY_H.min(h - 200.0).max(200.0);
        {
            let mut q = self.query.borrow_mut();
            q.set_styles(TextStyle { size: 13.5, line_height: 1.25, weight: Weight::Regular, color: t.label }, None);
            q.placeholder = "Search for apps, settings and documents".into();
            q.placeholder_color = t.tertiary;
            q.caret_color = t.accent;
            q.selection_color = t.accent.alpha(0.3);
        }
        let secondary = t.secondary;
        let field = div()
            .h(36.0)
            .w(INNER)
            .rounded(18.0)
            .bg(if t.dark { t.fill } else { Color::WHITE })
            .border(if t.dark { t.separator } else { t.border }, 0.5)
            .flex_row()
            .items_center()
            .px(14.0)
            .gap(8.0)
            .child(canvas(move |s, b| icons::magnifier(s, b, secondary, 14.0)).size(18.0).shrink_0())
            .child(input("start-input", &self.query).flex_1());

        let searching = !self.query.borrow().is_empty();
        let body = if searching {
            div().w(INNER + 16.0).h(body_h).flex_col().child(self.results_list(body_h, t))
        } else {
            let view = match self.start_view {
                StartView::Home => self.start_home(body_h, cx, t),
                StartView::AllApps => self.all_apps(body_h, cx, t),
            };
            self.swapping(view, INNER)
        };

        div()
            .id("start")
            .w(WIDTH)
            .flex_col()
            .items_center()
            .rounded(16.0)
            .bg(t.panel)
            .border(t.border, 0.5)
            .shadow(t.shadow, 30.0, 10.0)
            .clip()
            .occlude()
            .on_mouse_down(|s: &mut Self, _: &mut Cx| s.start_menu = None)
            .child(div().pt(24.0).pb(14.0).child(field))
            .child(div().h(body_h).clip().child(body))
            .child(self.start_footer(cx, t))
    }

    /// A section's title, with an optional button at its right.
    fn section(title: &str, button: Option<Div<Self>>, t: &Theme) -> Div<Self> {
        div()
            .w(INNER)
            .h(SECTION_H)
            .flex_row()
            .items_center()
            .justify_between()
            .pl(8.0)
            .child(text(title.to_string()).size(13.0).weight(Weight::Semibold).color(t.label))
            .children(button)
    }

    /// "All apps ›" and "‹ Back": a small capsule button.
    fn nav_pill(&self, id: &'static str, label: &str, forward: bool, cx: &Cx, t: &Theme) -> Div<Self> {
        let pressed = cx.hovered(id) && cx.mouse_down;
        let bg = if pressed { t.fill.alpha(2.6) } else if cx.hovered(id) { t.fill.alpha(2.0) } else { t.fill.alpha(1.3) };
        let ink = t.label;
        let chevron = canvas(move |s, b| icons::chevron_sized(s, b, ink, forward, 8.0, 1.5)).w(8.0).h(12.0);
        let label = text(label.to_string()).size(12.0).weight(Weight::Medium);
        let mut pill = div().id(id).h(24.0).rounded(12.0).bg(bg).flex_row().items_center().gap(5.0);
        pill = if forward { pill.pl(11.0).pr(9.0).child(label).child(chevron) } else { pill.pl(9.0).pr(11.0).child(chevron).child(label) };
        let view = if forward { StartView::AllApps } else { StartView::Home };
        pill.on_click(move |s: &mut Self, cx: &mut Cx| s.show_start_view(view, cx))
    }

    pub(super) fn show_start_view(&mut self, view: StartView, cx: &mut Cx) {
        if self.start_view == view {
            return;
        }
        self.start_view = view;
        self.start_sel = None;
        self.start_menu = None;
        if view == StartView::AllApps {
            cx.scroll_to_top("start-all");
        }
        self.start_swap(view == StartView::AllApps, cx);
    }

    /// Shows All apps without the animation (for snapshots).
    pub fn show_all_apps_now(&mut self, cx: &mut Cx) {
        self.show_start_view(StartView::AllApps, cx);
        self.swap = None;
    }

    fn pinned_ids(&self) -> Vec<String> {
        self.start_pins.ids.iter().filter(|id| self.app(id).is_some()).cloned().collect()
    }

    fn start_home(&self, body_h: f32, cx: &Cx, t: &Theme) -> Div<Self> {
        let pins = self.pinned_ids();
        let rows = pins.len().div_ceil(COLUMNS).clamp(1, PIN_ROWS);
        let mut grid = div().w(INNER).flex_col().shrink_0();
        for (r, row) in pins.chunks(COLUMNS).enumerate() {
            let tiles = row.iter().enumerate().map(|(c, id)| self.pin_tile(id, r * COLUMNS + c, cx, t));
            grid = grid.child(div().flex_row().children(tiles));
        }
        if pins.is_empty() {
            grid = grid.child(text("Right-click an app to pin it here.").size(12.0).color(t.secondary).pl(8.0).pt(8.0));
        }
        let all = self.nav_pill("start-all-apps", "All apps", true, cx, t);
        let pinned_h = rows as f32 * TILE_H;
        let recent_h = body_h - SECTION_H - pinned_h - 14.0 - SECTION_H;
        div()
            .w(INNER)
            .h(body_h)
            .flex_col()
            .child(Self::section("Pinned", Some(all), t))
            .child(div().id("start-pins").w(INNER).h(pinned_h).flex_col().scroll_y().child(grid))
            .child(div().h(14.0))
            .child(Self::section("Recent", None, t))
            .child(self.recent_grid(recent_h, cx, t))
    }

    fn pin_tile(&self, id: &str, i: usize, cx: &Cx, t: &Theme) -> Div<Self> {
        let name = self.app(id).map(|e| e.name.clone()).unwrap_or_default();
        let (a, b) = (id.to_string(), id.to_string());
        let key = ("start-pin", id.to_string());
        let hovered = cx.hovered(key.clone());
        let hot = hovered || self.start_sel == Some(i) || self.start_menu.as_ref().is_some_and(|(m, _)| m == id);
        let bg = if hovered && cx.mouse_down { Some(t.hover.alpha(2.6)) } else { hot.then_some(t.hover.alpha(1.6)) };
        div()
            .id(key)
            .w(TILE_W)
            .h(TILE_H)
            .rounded(12.0)
            .flex_col()
            .items_center()
            .pt(12.0)
            .gap(7.0)
            .bg_opt(bg)
            .on_click(move |s: &mut Self, cx: &mut Cx| s.launch_app(&a, cx))
            .on_right_click(move |s: &mut Self, cx: &mut Cx| s.start_menu = Some((b.clone(), cx.mouse)))
            .child(self.start_icon(id, ICON))
            .child(text(name).size(12.0).truncate().max_w(TILE_W - 12.0))
    }

    fn recent_grid(&self, max_h: f32, cx: &Cx, t: &Theme) -> Div<Self> {
        let files = self.recent_files.lock().unwrap().clone();
        let rows = ((max_h / RECENT_H).floor() as usize).max(1);
        if files.is_empty() {
            return div()
                .w(INNER)
                .h(max_h)
                .pl(8.0)
                .pt(6.0)
                .child(text("Files you open will show up here.").size(12.0).color(t.secondary));
        }
        let now = chrono::Utc::now();
        let files: Vec<RecentFile> = files.into_iter().take(rows * 2).collect();
        let mut grid = div().w(INNER).flex_col();
        for (r, pair) in files.chunks(2).enumerate() {
            let cells = pair.iter().enumerate().map(|(c, f)| self.recent_row(f.clone(), r * 2 + c, now, cx, t));
            grid = grid.child(div().flex_row().children(cells));
        }
        grid
    }

    fn recent_row(&self, f: RecentFile, i: usize, now: chrono::DateTime<chrono::Utc>, cx: &Cx, t: &Theme) -> Div<Self> {
        let key = ("start-recent", i);
        let hovered = cx.hovered(key);
        let bg = if hovered && cx.mouse_down { Some(t.hover.alpha(2.6)) } else { hovered.then_some(t.hover.alpha(1.6)) };
        let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let path = f.path.to_string_lossy().into_owned();
        div()
            .id(key)
            .w(INNER / 2.0)
            .h(RECENT_H)
            .rounded(10.0)
            .flex_row()
            .items_center()
            .px(8.0)
            .gap(12.0)
            .bg_opt(bg)
            .on_click(move |s: &mut Self, cx: &mut Cx| {
                launch(&format!("xdg-open {}", shell_quote(&path)));
                s.close(cx);
            })
            .child(file_icon(&f, 32.0, cx.scale, t))
            .child(
                div()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(text(name).size(13.0).truncate())
                    .child(text(crate::recent::ago(f.used, now)).size(11.0).color(t.secondary).truncate()),
            )
    }

    /// The apps listed in All apps, in order.
    fn listed_apps(&self) -> Vec<String> {
        apps::by_letter(&self.apps).into_iter().flat_map(|(_, list)| list.into_iter().map(|e| e.id.clone())).collect()
    }

    fn all_apps(&self, body_h: f32, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut list = div().w(INNER).flex_col().shrink_0().pb(8.0);
        let mut i = 0;
        for (letter, entries) in apps::by_letter(&self.apps) {
            list = list.child(
                div()
                    .w(INNER)
                    .h(LETTER_H)
                    .shrink_0()
                    .pl(10.0)
                    .pb(5.0)
                    .justify_end()
                    .flex_col()
                    .child(text(letter).size(12.0).weight(Weight::Semibold).color(t.secondary)),
            );
            for e in entries {
                let (a, b) = (e.id.clone(), e.id.clone());
                let id = ("start-app", e.id.clone());
                let hovered = cx.hovered(id.clone());
                let hot = hovered || self.start_sel == Some(i) || self.start_menu.as_ref().is_some_and(|(m, _)| *m == e.id);
                let bg = if hovered && cx.mouse_down { Some(t.hover.alpha(2.6)) } else { hot.then_some(t.hover.alpha(1.6)) };
                list = list.child(
                    div()
                        .id(id)
                        .w(INNER)
                        .h(ROW_H)
                        .shrink_0()
                        .px(8.0)
                        .rounded(8.0)
                        .flex_row()
                        .items_center()
                        .gap(12.0)
                        .bg_opt(bg)
                        .on_click(move |s: &mut Self, cx: &mut Cx| s.launch_app(&a, cx))
                        .on_right_click(move |s: &mut Self, cx: &mut Cx| s.start_menu = Some((b.clone(), cx.mouse)))
                        .child(self.start_icon(&e.id, 24.0))
                        .child(text(e.name.clone()).size(13.0).truncate().flex_1().min_w_0()),
                );
                i += 1;
            }
        }
        let back = self.nav_pill("start-back", "Back", false, cx, t);
        div()
            .w(INNER)
            .h(body_h)
            .flex_col()
            .child(Self::section("All apps", Some(back), t))
            .child(div().id("start-all").w(INNER).h(body_h - SECTION_H).flex_col().scroll_y().child(list))
    }

    /// Arrow keys and Enter in Start while not searching: through the pins
    /// (a grid) or All apps (a list).
    pub(super) fn start_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        let ids = match self.start_view {
            StartView::Home => self.pinned_ids(),
            StartView::AllApps => self.listed_apps(),
        };
        let n = ids.len();
        if n == 0 {
            return false;
        }
        let row = if self.start_view == StartView::Home { COLUMNS } else { 1 };
        let next = match (self.start_sel, key) {
            (_, k) if k.is_named(NamedKey::Enter) => {
                if let Some(id) = self.start_sel.and_then(|i| ids.get(i)).cloned() {
                    self.launch_app(&id, cx);
                }
                return true;
            }
            (None, k) if [NamedKey::ArrowDown, NamedKey::ArrowRight, NamedKey::ArrowUp, NamedKey::ArrowLeft].iter().any(|a| k.is_named(*a)) => 0,
            (Some(i), k) if k.is_named(NamedKey::ArrowRight) && row > 1 => (i + 1).min(n - 1),
            (Some(i), k) if k.is_named(NamedKey::ArrowLeft) && row > 1 => i.saturating_sub(1),
            (Some(i), k) if k.is_named(NamedKey::ArrowDown) => if i + row < n { i + row } else { i },
            (Some(i), k) if k.is_named(NamedKey::ArrowUp) => i.checked_sub(row).unwrap_or(i),
            _ => return false,
        };
        self.start_sel = Some(next);
        self.reveal_selected(next, cx);
        true
    }

    /// Scrolls Pinned or All apps so the picked app shows.
    fn reveal_selected(&self, i: usize, cx: &mut Cx) {
        let (id, top, h, view_h) = match self.start_view {
            StartView::Home => ("start-pins", (i / COLUMNS) as f32 * TILE_H, TILE_H, PIN_ROWS as f32 * TILE_H),
            StartView::AllApps => {
                let mut y = 0.0;
                let mut seen = 0;
                for (_, entries) in apps::by_letter(&self.apps) {
                    y += LETTER_H;
                    if i < seen + entries.len() {
                        y += (i - seen) as f32 * ROW_H;
                        break;
                    }
                    seen += entries.len();
                    y += entries.len() as f32 * ROW_H;
                }
                let body_h = BODY_H.min(cx.window_size.1 - 200.0).max(200.0);
                ("start-all", y, ROW_H, body_h - SECTION_H)
            }
        };
        let scrolled = cx.scroll_y(id);
        if top < scrolled + if i == 0 { LETTER_H } else { 0.0 } {
            cx.scroll_to(id, if self.start_view == StartView::AllApps { top - LETTER_H } else { top });
        } else if top + h > scrolled + view_h {
            cx.scroll_to(id, top + h - view_h);
        }
    }

    /// An app's icon, or its letter tile.
    pub(super) fn start_icon(&self, id: &str, size: f32) -> AnyElement<Self> {
        let name = self.app(id).map(|e| e.name.clone()).unwrap_or_default();
        match self.icon_for(Some(id), id) {
            Some(img) => image(&img, Fit::Scale, 0.0).size(size).shrink_0().into(),
            None => letter_tile(&name, size).into(),
        }
    }

    fn start_footer(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let buttons: [(&str, Power, Glyph); 5] = [
            ("Lock", Power::Lock, icons::lock),
            ("Sleep", Power::Sleep, icons::moon),
            ("Log Out\u{2026}", Power::LogOut, icons::log_out),
            ("Restart\u{2026}", Power::Restart, icons::restart),
            ("Shut Down\u{2026}", Power::ShutDown, icons::power),
        ];
        let hovered = buttons.iter().find(|(_, p, _)| cx.hovered(("start-power", *p as usize))).map(|(l, ..)| *l);
        let mut row = div().flex_row().items_center().gap(2.0);
        if let Some(label) = hovered {
            row = row.child(text(label).size(12.0).color(t.secondary).mr(8.0));
        }
        let ink = t.label.alpha(0.8);
        for (_, p, glyph) in buttons {
            let key = ("start-power", p as usize);
            let bg = if cx.hovered(key) && cx.mouse_down {
                Some(t.hover.alpha(2.6))
            } else {
                cx.hovered(key).then_some(t.hover.alpha(1.8))
            };
            row = row.child(
                div()
                    .id(key)
                    .size(34.0)
                    .rounded(9.0)
                    .items_center()
                    .justify_center()
                    .bg_opt(bg)
                    .on_click(move |s: &mut Self, cx: &mut Cx| s.power(p, cx))
                    .child(canvas(move |s, b| glyph(s, b, ink)).size(20.0)),
            );
        }
        div()
            .w_full()
            .h(FOOTER_H)
            .mt(10.0)
            .px(PAD - 4.0)
            .flex_row()
            .items_center()
            .justify_between()
            .bg(if t.dark { rgba(0x000000, 0.14) } else { rgba(0x000000, 0.035) })
            .border(t.separator, 0.0)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(10.0)
                    .pl(4.0)
                    .child(avatar(&self.user, 32.0, t))
                    .child(text(self.user.name.clone()).size(13.0).weight(Weight::Medium)),
            )
            .child(row)
    }

    fn power(&mut self, p: Power, cx: &mut Cx) {
        match p {
            Power::Lock => self.session(SessionAction::Lock, cx),
            Power::Sleep => self.session(SessionAction::Sleep, cx),
            Power::LogOut => self.open(Panel::Confirm(SessionAction::LogOut), cx),
            Power::Restart => self.open(Panel::Confirm(SessionAction::Restart), cx),
            Power::ShutDown => self.open(Panel::Confirm(SessionAction::ShutDown), cx),
        }
    }

    /// An app's right-click menu in Start, at the pointer (panel coordinates).
    pub(super) fn start_context_menu(&self, cx: &Cx, t: &Theme) -> Option<Div<Self>> {
        let (id, at) = self.start_menu.clone()?;
        let (w, h) = cx.window_size;
        let rows: [(String, fn(&mut Self, &str)); 2] = [
            (
                if self.start_pins.contains(&id) { "Unpin from Start" } else { "Pin to Start" }.into(),
                |s, id| s.start_pins.toggle(id),
            ),
            (
                if self.pins.contains(&id) { "Unpin from Taskbar" } else { "Pin to Taskbar" }.into(),
                |s, id| s.pins.toggle(id),
            ),
        ];
        let menu_h = 10.0 + 24.0 * rows.len() as f32;
        let mut menu = div()
            .absolute(at.x.min(w - CTX_W - 8.0), at.y.min(h - menu_h - 8.0))
            .w(CTX_W)
            .flex_col()
            .p(5.0)
            .rounded(10.0)
            .bg(t.menu)
            .border(t.border, 0.5)
            .shadow(t.shadow, 24.0, 8.0)
            .occlude();
        for (i, (label, f)) in rows.into_iter().enumerate() {
            let key = ("start-ctx", i);
            let fg = if cx.hovered(key) { Color::WHITE } else { t.label };
            let id = id.clone();
            menu = menu.child(
                div()
                    .id(key)
                    .h(24.0)
                    .px(10.0)
                    .rounded(5.0)
                    .items_center()
                    .hover_bg(t.accent)
                    .on_click(move |s: &mut Self, _: &mut Cx| {
                        f(s, &id);
                        s.start_menu = None;
                    })
                    .child(text(label).color(fg)),
            );
        }
        Some(menu)
    }
}

/// A recent file's icon: a folder, or a page tinted and labeled for its kind.
fn file_icon<S: 'static>(f: &RecentFile, size: f32, scale: f32, t: &Theme) -> Div<S> {
    if f.is_dir {
        let blue = rgb(0x5aa9f7);
        return div().size(size).shrink_0().child(canvas(move |s, b| icons::folder(s, b, blue)).size(size));
    }
    let ext = f.path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let tint = rgb(match ext.as_str() {
        "pdf" => 0xe5484d,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "heic" | "avif" => 0x30a46c,
        "mp4" | "mkv" | "webm" | "mov" | "mp3" | "flac" | "ogg" | "wav" | "m4a" => 0xd6409f,
        "zip" | "tar" | "gz" | "xz" | "zst" | "7z" | "rar" | "deb" | "appimage" => 0xa18072,
        "doc" | "docx" | "odt" | "rtf" | "md" | "txt" => 0x3e63dd,
        "xls" | "xlsx" | "ods" | "csv" => 0x30a46c,
        "ppt" | "pptx" | "odp" | "key" => 0xf76b15,
        _ => 0x8e8e93,
    });
    let (paper, edge) = if t.dark { (rgb(0xf2f2f4), rgba(0x000000, 0.3)) } else { (Color::WHITE, rgba(0x000000, 0.16)) };
    // Too small to read on a 1x screen.
    let label: String = if scale >= 1.5 { ext.chars().take(4).collect::<String>().to_uppercase() } else { String::new() };
    div()
        .size(size)
        .shrink_0()
        .child(canvas(move |s, b| icons::page(s, b, paper, edge, tint)).size(size))
        .child_if(!label.is_empty(), || {
            div()
                // Over the page's band (see `icons::page`).
                .absolute(0.0, size * 0.638)
                .w(size)
                .h(size * 0.172)
                .items_center()
                .justify_center()
                .child(text(label.clone()).size(size * 0.16).weight(Weight::Bold).color(Color::WHITE).line_height(1.0))
                .into()
        })
}
