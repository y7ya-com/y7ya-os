//! The Start menu: a search field over "All apps" and a grid of pinned apps
//! (Windows 10's arrangement), with the user and power buttons at the bottom.
//! Typing searches with Spotlight's engine.

use ui::*;

use super::dialogs::avatar;
use super::spotlight::letter_tile;
use super::theme::Theme;
use super::{Panel, Shell, icons};
use crate::apps;
use crate::system::SessionAction;

const WIDTH: f32 = 600.0;
const BODY_H: f32 = 440.0;
const LIST_W: f32 = 232.0;
const COLUMNS: usize = 4;
const TILE_W: f32 = 80.0;
const TILE_H: f32 = 80.0;
const ROW_H: f32 = 32.0;
const FOOTER_H: f32 = 56.0;
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
        let body_h = BODY_H.min(h - 180.0).max(160.0);
        {
            let mut q = self.query.borrow_mut();
            q.set_styles(TextStyle { size: 14.0, line_height: 1.25, weight: Weight::Regular, color: t.label }, None);
            q.placeholder = "Search apps, settings and documents".into();
            q.placeholder_color = t.tertiary;
            q.caret_color = t.accent;
            q.selection_color = t.accent.alpha(0.3);
        }
        let secondary = t.secondary;
        let field = div()
            .h(36.0)
            .mx(16.0)
            .mt(16.0)
            .mb(12.0)
            .rounded(10.0)
            .bg(t.fill)
            .border(t.separator, 0.5)
            .flex_row()
            .items_center()
            .px(12.0)
            .gap(8.0)
            .child(canvas(move |s, b| icons::magnifier(s, b, secondary, 15.0)).size(20.0).shrink_0())
            .child(input("start-input", &self.query).flex_1());

        let searching = !self.query.borrow().is_empty();
        let body = if searching {
            div().h(body_h).px(8.0).flex_col().child(self.results_list(body_h, t))
        } else {
            div()
                .h(body_h)
                .flex_row()
                .child(self.all_apps(body_h, cx, t))
                .child(div().w(1.0 / cx.scale.max(1.0)).h(body_h - 16.0).bg(t.separator))
                .child(self.pinned_grid(cx, t))
        };

        div()
            .id("start")
            .w(WIDTH)
            .flex_col()
            .rounded(14.0)
            .bg(t.panel)
            .border(t.border, 0.5)
            .shadow(t.shadow, 30.0, 10.0)
            .clip()
            .occlude()
            .on_mouse_down(|s: &mut Self, _: &mut Cx| s.start_menu = None)
            .child(field)
            .child(body)
            .child(div().h(1.0 / cx.scale.max(1.0)).w_full().bg(t.separator))
            .child(self.start_footer(cx, t))
    }

    fn section_title(label: &str, t: &Theme) -> AnyElement<Self> {
        text(label.to_string()).size(11.0).weight(Weight::Semibold).color(t.secondary).into()
    }

    fn all_apps(&self, body_h: f32, cx: &Cx, t: &Theme) -> Div<Self> {
        let mut list = div().flex_col().px(8.0).pb(8.0);
        for (letter, entries) in apps::by_letter(&self.apps) {
            list = list.child(
                div()
                    .h(28.0)
                    .px(8.0)
                    .items_end()
                    .pb(4.0)
                    .child(text(letter).size(12.0).weight(Weight::Semibold).color(t.accent)),
            );
            for e in entries {
                let (a, b) = (e.id.clone(), e.id.clone());
                let id = ("start-app", e.id.clone());
                let hot = cx.hovered(id.clone()) || self.start_menu.as_ref().is_some_and(|(m, _)| *m == e.id);
                list = list.child(
                    div()
                        .id(id)
                        .h(ROW_H)
                        .px(8.0)
                        .rounded(7.0)
                        .flex_row()
                        .items_center()
                        .gap(10.0)
                        .bg_opt(hot.then_some(t.hover.alpha(1.6)))
                        .on_click(move |s: &mut Self, cx: &mut Cx| s.launch_app(&a, cx))
                        .on_right_click(move |s: &mut Self, cx: &mut Cx| s.start_menu = Some((b.clone(), cx.mouse)))
                        .child(self.start_icon(&e.id, 20.0))
                        .child(text(e.name.clone()).truncate().flex_1().min_w_0()),
                );
            }
        }
        div()
            .w(LIST_W)
            .h(body_h)
            .flex_col()
            .child(div().h(20.0).px(16.0).child(Self::section_title("All Apps", t)))
            .child(div().id("start-all").h(body_h - 20.0).scroll_y().child(list))
    }

    /// An app's icon, or its letter tile.
    pub(super) fn start_icon(&self, id: &str, size: f32) -> AnyElement<Self> {
        let name = self.app(id).map(|e| e.name.clone()).unwrap_or_default();
        match self.icon_for(Some(id), id) {
            Some(img) => image(&img, Fit::Scale, 0.0).size(size).shrink_0().into(),
            None => letter_tile(&name, size).into(),
        }
    }

    fn pinned_grid(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let pins: Vec<String> = self.start_pins.ids.iter().filter(|id| self.app(id).is_some()).cloned().collect();
        let mut grid = div().flex_col().gap(4.0);
        for row in pins.chunks(COLUMNS) {
            let mut r = div().flex_row().gap(4.0);
            for id in row {
                let Some(e) = self.app(id) else { continue };
                let (a, b) = (id.clone(), id.clone());
                let key = ("start-pin", id.clone());
                let hot = cx.hovered(key.clone()) || self.start_menu.as_ref().is_some_and(|(m, _)| m == id);
                r = r.child(
                    div()
                        .id(key)
                        .w(TILE_W)
                        .h(TILE_H)
                        .rounded(10.0)
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(6.0)
                        .bg_opt(hot.then_some(t.hover.alpha(1.6)))
                        .on_click(move |s: &mut Self, cx: &mut Cx| s.launch_app(&a, cx))
                        .on_right_click(move |s: &mut Self, cx: &mut Cx| s.start_menu = Some((b.clone(), cx.mouse)))
                        .child(self.start_icon(id, 40.0))
                        .child(text(e.name.clone()).size(11.5).truncate().max_w(TILE_W - 8.0)),
                );
            }
            grid = grid.child(r);
        }
        if pins.is_empty() {
            grid = grid.child(text("Right-click an app to pin it here.").size(12.0).color(t.secondary).mt(8.0));
        }
        div()
            .flex_1()
            .flex_col()
            .pl(16.0)
            .pr(12.0)
            .child(div().h(20.0).px(4.0).child(Self::section_title("Pinned", t)))
            .child(grid)
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
        let mut row = div().flex_row().items_center().gap(4.0);
        if let Some(label) = hovered {
            row = row.child(text(label).size(12.0).color(t.secondary).mr(4.0));
        }
        let ink = t.label;
        for (_, p, glyph) in buttons {
            row = row.child(
                div()
                    .id(("start-power", p as usize))
                    .size(32.0)
                    .rounded(8.0)
                    .items_center()
                    .justify_center()
                    .hover_bg(t.hover.alpha(1.8))
                    .on_click(move |s: &mut Self, cx: &mut Cx| s.power(p, cx))
                    .child(canvas(move |s, b| glyph(s, b, ink)).size(20.0)),
            );
        }
        div()
            .h(FOOTER_H)
            .px(16.0)
            .flex_row()
            .items_center()
            .justify_between()
            .bg(t.fill.alpha(0.6))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(10.0)
                    .child(avatar(&self.user, 28.0, t))
                    .child(text(self.user.name.clone()).weight(Weight::Medium)),
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
