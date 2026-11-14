//! The taskbar (Windows 10's layout, macOS's materials), the overlay that
//! holds whatever flyout is open, and the taskbar buttons' menu.

use ui::*;

use super::spotlight::letter_tile;
use super::theme::Theme;
use super::{BAR_H, MARGIN, Panel, SLIDE, Shell, icons};
use crate::taskbar::{self, GAP, ICON_W, Item};
use crate::toplevel::WindowAction;

/// Buttons are pills this tall, centered in the bar.
const PILL_H: f32 = 36.0;
const PILL_Y: f32 = (BAR_H - PILL_H) / 2.0;
const ICON: f32 = 24.0;
/// The Show Desktop sliver at the far right.
const SLIVER_W: f32 = 8.0;
/// Below this a window button drops its title.
const LABEL_MIN_W: f32 = 88.0;
const MENU_W: f32 = 212.0;
const TRAY_SLOT: f32 = 28.0;

const TIME: TextStyle = TextStyle { size: 12.0, line_height: 1.25, weight: Weight::Medium, color: Color::BLACK };
const DATE: TextStyle = TextStyle { size: 11.0, line_height: 1.25, weight: Weight::Regular, color: Color::BLACK };

/// "7:52 PM" over "11/14/2026".
pub fn clock_lines(now: &chrono::DateTime<chrono::Local>) -> (String, String) {
    (now.format("%-I:%M %p").to_string(), now.format("%-m/%-d/%Y").to_string())
}

#[derive(Clone, Copy)]
enum MenuAction {
    New,
    TogglePin,
    Close,
}

impl Shell {
    /// A pill-shaped taskbar button: `on` is its open/active look.
    fn bar_button(&self, id: impl std::hash::Hash + Copy, w: f32, on: bool, cx: &Cx, t: &Theme) -> Div<Self> {
        let pressed = cx.mouse_down && cx.hovered(id);
        let bg = match (on, pressed) {
            (_, true) => t.bar_press,
            (true, false) => t.bar_active,
            (false, false) => Color::TRANSPARENT,
        };
        let mut d = div()
            .id(id)
            .w(w)
            .h(PILL_H)
            .rounded(8.0)
            .items_center()
            .justify_center()
            .bg(bg)
            .hover_bg(if pressed { t.bar_press } else if on { t.bar_active } else { t.bar_hover });
        if on && !t.dark && !pressed {
            // A white pill on a light bar needs an edge.
            d = d.border(t.border.alpha(0.5), 0.5).shadow(t.shadow.alpha(0.25), 2.0, 0.5);
        }
        d
    }

    fn app_icon(&self, app: Option<&str>, app_id: &str, name: &str, size: f32) -> AnyElement<Self> {
        match self.icon_for(app, app_id) {
            Some(img) => image(&img, Fit::Scale, 0.0).size(size).shrink_0().into(),
            None => letter_tile(name, size).into(),
        }
    }

    pub(super) fn taskbar(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (w, _) = cx.window_size;
        let open = self.panel;

        // Left: Start, search, then the app buttons from `taskbar::layout`.
        let start_x = 4.0;
        let search_x = start_x + ICON_W + GAP;
        let items_x = search_x + ICON_W + 12.0;

        let start_on = open == Panel::Start;
        let tile = if start_on || cx.hovered("tb-start") { t.accent } else { t.label };
        let hole = if t.dark { rgb(0x1e1e20) } else { rgb(0xf2f2f4) };
        let start = self
            .bar_button("tb-start", ICON_W, start_on, cx, t)
            .absolute(start_x, PILL_Y)
            .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.open(Panel::Start, cx))
            .child(canvas(move |s, b| icons::start_mark(s, b, tile, hole)).size(20.0));
        let ink = t.label;
        let search = self
            .bar_button("tb-search", ICON_W, open == Panel::Spotlight, cx, t)
            .absolute(search_x, PILL_Y)
            .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.open(Panel::Spotlight, cx))
            .child(canvas(move |s, b| icons::magnifier(s, b, ink, 15.0)).size(20.0));

        let right = self.tray_and_clock(cx, t);
        let right_w = self.right_width(cx);
        let items = self.items();
        let slots = taskbar::layout(&items, items_x, (w - right_w - 12.0 - items_x).max(0.0));
        let active = self.active_window();
        let mut rects = Vec::new();
        let mut buttons = Vec::new();
        for (i, (item, (x, bw))) in items.iter().zip(slots).enumerate() {
            if let Item::Window { id, .. } = item {
                rects.push((*id, [x as i32, PILL_Y as i32, bw as i32, PILL_H as i32]));
            }
            buttons.push(self.item_button(i, item, x, bw, active, cx, t));
        }
        self.send_rects(rects, cx);

        div()
            .w_full()
            .h(BAR_H)
            .bg(t.bar)
            .child(div().absolute(0.0, 0.0).w(w).h(1.0 / cx.scale.max(1.0)).bg(t.bar_edge))
            .child(start)
            .child(search)
            .children(buttons)
            .child(right.absolute(w - right_w, 0.0))
    }

    #[allow(clippy::too_many_arguments)]
    fn item_button(&self, i: usize, item: &Item, x: f32, w: f32, active: Option<u64>, cx: &Cx, t: &Theme) -> Div<Self> {
        let id = ("tb-item", i);
        match item {
            Item::Pinned { app } => {
                let name = self.app(app).map(|e| e.name.clone()).unwrap_or_default();
                let (a, b) = (app.clone(), app.clone());
                self.bar_button(id, w, false, cx, t)
                    .absolute(x, PILL_Y)
                    .on_click(move |s: &mut Self, cx: &mut Cx| s.launch_app(&a, cx))
                    .on_middle_click(move |s: &mut Self, cx: &mut Cx| s.launch_app(&b, cx))
                    .on_right_click({
                        let item = item.clone();
                        move |s: &mut Self, cx: &mut Cx| s.open_task_menu(item.clone(), x, cx)
                    })
                    .child(self.app_icon(Some(app), "", &name, ICON))
            }
            Item::Window { id: win, app } => {
                let Some(window) = self.window(*win) else { return div() };
                let is_active = active == Some(*win);
                let name = app.as_deref().and_then(|a| self.app(a)).map(|e| e.name.clone());
                let name = name.unwrap_or_else(|| crate::apps::display_name(&self.apps, &window.app_id));
                let title = if window.title.is_empty() { name.clone() } else { window.title.clone() };
                let icon = self.app_icon(app.as_deref(), &window.app_id, &name, ICON);
                let labeled = w >= LABEL_MIN_W;
                let mut content = div().flex_row().items_center().gap(8.0).h(PILL_H);
                content = if labeled {
                    content.w(w).px(8.0).child(icon).child(text(title).size(12.5).truncate().flex_1().min_w_0())
                } else {
                    content.justify_center().w(w).child(icon)
                };
                if window.minimized {
                    content = content.opacity(0.5);
                }
                // The underline: accent and long for the active window, a
                // short quiet dash for the others.
                let (line_w, line_c) = if is_active {
                    (if labeled { w - 24.0 } else { 16.0 }, t.accent)
                } else {
                    (8.0, t.bar_running)
                };
                let underline = div()
                    .absolute((w - line_w) / 2.0, PILL_H - 4.0)
                    .w(line_w)
                    .h(2.5)
                    .rounded(1.25)
                    .bg(if window.minimized { line_c.alpha(0.6) } else { line_c });
                let (win, new_app) = (*win, app.clone());
                self.bar_button(id, w, is_active, cx, t)
                    .absolute(x, PILL_Y)
                    .on_click(move |s: &mut Self, cx: &mut Cx| s.window_clicked(win, cx))
                    .on_middle_click(move |s: &mut Self, cx: &mut Cx| {
                        if let Some(a) = &new_app {
                            s.launch_app(a, cx);
                        }
                    })
                    .on_right_click({
                        let item = item.clone();
                        move |s: &mut Self, cx: &mut Cx| s.open_task_menu(item.clone(), x, cx)
                    })
                    .child(content)
                    .child(underline)
            }
        }
    }

    /// The tray icons that are showing, for sizing.
    fn tray_count(&self) -> usize {
        let st = &self.status;
        [self.store.lock().unwrap().dnd, st.wifi.available, st.volume.is_some(), st.battery.is_some()]
            .into_iter()
            .filter(|b| *b)
            .count()
    }

    fn tray_width(&self) -> f32 {
        12.0 + self.tray_count().max(1) as f32 * TRAY_SLOT
    }

    fn clock_width(&self, cx: &mut Cx) -> f32 {
        let (time, date) = clock_lines(&chrono::Local::now());
        let tw = cx.text.measure(&time, &TIME, None, false).0;
        let dw = cx.text.measure(&date, &DATE, None, false).0;
        (tw.max(dw) + 20.0).ceil()
    }

    /// Everything right of the app buttons: tray, clock and Show Desktop.
    fn right_width(&self, cx: &mut Cx) -> f32 {
        self.tray_width() + GAP + self.clock_width(cx) + 4.0 + SLIVER_W
    }

    fn tray_and_clock(&self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let ink = t.label;
        let dim = ink.alpha(0.3);
        let st = self.status.clone();
        let mut tray = self
            .bar_button("tb-tray", self.tray_width(), self.panel == Panel::QuickSettings, cx, t)
            .flex_row()
            .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.open(Panel::QuickSettings, cx));
        let slot = |c: AnyElement<Self>| div().w(TRAY_SLOT).h(PILL_H).items_center().justify_center().child(c);
        if self.store.lock().unwrap().dnd {
            tray = tray.child(slot(canvas(move |s, b| icons::moon(s, b, ink)).size(20.0).into()));
        }
        if st.wifi.available {
            let w = st.wifi.clone();
            tray = tray.child(slot(
                canvas(move |s, b| icons::wifi(s, b, ink, dim, w.strength, w.enabled && w.ssid.is_some())).size(20.0).into(),
            ));
        }
        if let Some(v) = st.volume {
            // The waves reach right of center; nudge the glyph left.
            tray = tray.child(slot(
                canvas(move |s, b| icons::speaker(s, Bounds::new(b.x - 2.0, b.y, b.w, b.h), ink, v.level, v.muted)).size(20.0).into(),
            ));
        }
        if let Some(bat) = st.battery {
            let fill = if bat.charging {
                t.success
            } else if bat.percent <= 20 {
                t.destructive
            } else {
                ink
            };
            tray = tray.child(
                div().w(TRAY_SLOT).h(PILL_H).items_center().justify_center().child(
                    canvas(move |s, b| {
                        // The 23pt-wide glyph, scaled into the slot.
                        let c = b.center();
                        icons::battery(s, Bounds::new(c.x - 13.0, c.y - 10.0, 26.0, 20.0), ink, fill, bat.percent, bat.charging)
                    })
                    .w(24.0)
                    .h(20.0),
                ),
            );
        }

        let (time, date) = clock_lines(&chrono::Local::now());
        let clock = self
            .bar_button("tb-clock", self.clock_width(cx), self.panel == Panel::Calendar, cx, t)
            .flex_col()
            .items_end()
            .px(10.0)
            .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.open(Panel::Calendar, cx))
            .child(text(time).size(TIME.size).line_height(TIME.line_height).weight(TIME.weight))
            .child(text(date).size(DATE.size).line_height(DATE.line_height).color(t.secondary));

        let sliver = div()
            .id("tb-desktop")
            .w(SLIVER_W)
            .h(BAR_H)
            .hover_bg(t.bar_hover)
            .on_click(|s: &mut Self, cx: &mut Cx| s.toggle_desktop(cx))
            .child(div().w(1.0 / cx.scale.max(1.0)).h(BAR_H).bg(t.bar_edge));

        div()
            .flex_row()
            .items_center()
            .h(BAR_H)
            .gap(GAP)
            .child(tray)
            .child(clock)
            .child(sliver.ml(4.0 - GAP))
    }

    // -- the overlay ------------------------------------------------------------

    /// The overlay above the taskbar: whatever is open, over an invisible
    /// backdrop that closes it when clicked. Flyouts sit at the bottom.
    pub(super) fn panel_view(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (w, h) = cx.window_size;
        let reveal = self.reveal();
        let slide = (1.0 - reveal) * SLIDE;
        let root = div()
            .id("backdrop")
            .w(w)
            .h(h)
            .flex_col()
            .justify_end()
            .pb(MARGIN - slide)
            .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.close(cx));
        match self.drawn {
            Panel::None => root,
            Panel::Start => {
                let menu = self.start_context_menu(cx, t);
                root.items_start().pl(MARGIN).child(self.start_menu_view(cx, t).opacity(reveal)).children(menu)
            }
            Panel::QuickSettings => root.items_end().pr(MARGIN).child(self.quick_settings(cx, t).opacity(reveal)),
            Panel::Calendar => root.items_end().pr(MARGIN).child(self.calendar_flyout(cx, t).opacity(reveal)),
            Panel::TaskMenu => {
                let x = self.task_menu.as_ref().map_or(0.0, |(_, x)| *x);
                root.items_start().pl(x.clamp(MARGIN, (w - MENU_W - MARGIN).max(MARGIN))).child(self.task_menu_view(cx, t).opacity(reveal))
            }
            Panel::Spotlight => div().id("backdrop").w(w).h(h).on_mouse_down(|s: &mut Self, cx: &mut Cx| s.close(cx)).child(
                self.spotlight(cx, t).opacity(reveal),
            ),
            Panel::Confirm(action) => div()
                .id("backdrop")
                .w(w)
                .h(h)
                // Dialogs dim the screen a little, like a sheet.
                .bg(Color::BLACK.alpha(if t.dark { 0.25 } else { 0.12 } * reveal))
                .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.close(cx))
                .child(self.confirm(action, w, h, t, reveal)),
        }
    }

    // -- a button's menu --------------------------------------------------------

    fn open_task_menu(&mut self, item: Item, x: f32, cx: &mut Cx) {
        self.task_menu = Some((item, x));
        self.panel = Panel::None;
        self.open(Panel::TaskMenu, cx);
    }

    fn task_menu_view(&self, cx: &Cx, t: &Theme) -> Div<Self> {
        let Some((item, _)) = &self.task_menu else { return div() };
        let app = item.app().and_then(|a| self.app(a));
        let pinned = item.app().is_some_and(|a| self.pins.contains(a));
        let mut rows: Vec<Option<(String, MenuAction)>> = Vec::new();
        if let Some(e) = app {
            rows.push(Some((if matches!(item, Item::Pinned { .. }) { format!("Open {}", e.name) } else { "New Window".into() }, MenuAction::New)));
            rows.push(Some((if pinned { "Unpin from Taskbar" } else { "Pin to Taskbar" }.into(), MenuAction::TogglePin)));
        }
        if matches!(item, Item::Window { .. }) {
            if !rows.is_empty() {
                rows.push(None);
            }
            rows.push(Some(("Close Window".into(), MenuAction::Close)));
        }
        let mut menu = div()
            .w(MENU_W)
            .flex_col()
            .p(5.0)
            .rounded(10.0)
            .bg(t.menu)
            .border(t.border, 0.5)
            .shadow(t.shadow, 24.0, 8.0)
            .occlude();
        for (i, row) in rows.into_iter().enumerate() {
            let Some((label, action)) = row else {
                menu = menu.child(div().h(1.0).mx(10.0).mt(5.0).mb(5.0).bg(t.separator));
                continue;
            };
            let id = ("task-menu-row", i);
            let fg = if cx.hovered(id) { Color::WHITE } else { t.label };
            menu = menu.child(
                div()
                    .id(id)
                    .h(24.0)
                    .px(10.0)
                    .rounded(5.0)
                    .items_center()
                    .hover_bg(t.accent)
                    .on_click(move |s: &mut Self, cx: &mut Cx| s.task_menu_action(action, cx))
                    .child(text(label).color(fg)),
            );
        }
        menu
    }

    fn task_menu_action(&mut self, action: MenuAction, cx: &mut Cx) {
        let Some((item, _)) = self.task_menu.clone() else { return };
        match action {
            MenuAction::New => {
                if let Some(a) = item.app() {
                    self.launch_app(&a.to_string(), cx);
                }
            }
            MenuAction::TogglePin => {
                if let Some(a) = item.app() {
                    self.pins.toggle(a);
                }
                self.close(cx);
            }
            MenuAction::Close => {
                self.close(cx);
                if let Item::Window { id, .. } = item {
                    self.act(id, WindowAction::Close);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    #[test]
    fn clock_format() {
        let t = chrono::Local.with_ymd_and_hms(2026, 11, 14, 19, 52, 0).unwrap();
        assert_eq!(super::clock_lines(&t), ("7:52 PM".into(), "11/14/2026".into()));
        let t = chrono::Local.with_ymd_and_hms(2026, 1, 6, 9, 5, 0).unwrap();
        assert_eq!(super::clock_lines(&t), ("9:05 AM".into(), "1/6/2026".into()));
    }
}
