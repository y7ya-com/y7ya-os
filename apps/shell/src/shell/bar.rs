//! The taskbar (KDE's layout: Start, the pager, one icon per app, the tray;
//! macOS's materials), the overlay that holds whatever flyout is open, the
//! taskbar buttons' menu and tooltip.

use std::time::{Duration, Instant};

use ui::layer::{Anchor, Layer, LayerOptions};
use ui::*;

use super::spotlight::letter_tile;
use super::theme::Theme;
use super::{BAR_H, Hover, MARGIN, Panel, SLIDE, Shell, TOOLTIP, TOOLTIP_DELAY, icons};
use crate::desktops::Spaces;
use crate::taskbar::{self, GAP, ICON_W, Item};
use crate::toplevel::WindowAction;

/// Buttons are pills this tall, centered in the bar.
const PILL_H: f32 = 36.0;
const PILL_Y: f32 = (BAR_H - PILL_H) / 2.0;
const ICON: f32 = 24.0;
/// The Show Desktop sliver at the far right.
const SLIVER_W: f32 = 8.0;
/// The pager's desktops are this tall, as wide as the screen's shape says.
const SPACE_H: f32 = 28.0;
/// Wheel travel that moves the pager one desktop (one notch).
const SPACE_STEP: f32 = 48.0;
const MENU_W: f32 = 212.0;
const TRAY_SLOT: f32 = 28.0;

const TIME: TextStyle = TextStyle { size: 12.0, line_height: 1.25, weight: Weight::Medium, color: Color::BLACK };
const DATE: TextStyle = TextStyle { size: 11.0, line_height: 1.25, weight: Weight::Regular, color: Color::BLACK };
const TIP: TextStyle = TextStyle { size: 12.0, line_height: 1.25, weight: Weight::Regular, color: Color::BLACK };
/// The tooltip's text is padded this much, and its surface this much more
/// for the shadow.
const TIP_PAD: f32 = 8.0;
const TIP_SHADOW: f32 = 6.0;
const TIP_MAX_W: f32 = 360.0;

/// "7:52 PM" over "11/14/2026".
pub fn clock_lines(now: &chrono::DateTime<chrono::Local>) -> (String, String) {
    (now.format("%-I:%M %p").to_string(), now.format("%-m/%-d/%Y").to_string())
}

#[derive(Clone, Copy)]
enum MenuAction {
    New,
    TogglePin,
    /// Every window of the button's app.
    Close,
}

impl Shell {
    /// A pill-shaped taskbar button: `on` is its open/active look.
    fn bar_button(&self, id: impl std::hash::Hash + Clone, w: f32, on: bool, cx: &Cx, t: &Theme) -> Div<Self> {
        let pressed = cx.mouse_down && cx.hovered(id.clone());
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

        // Left: Start, the pager, then the app buttons from `taskbar::layout`.
        let start_x = 4.0;
        let start_on = open == Panel::Start;
        let tile = if start_on || cx.hovered("tb-start") { t.accent } else { t.label };
        let hole = if t.dark { rgb(0x1e1e20) } else { rgb(0xf2f2f4) };
        let start = self
            .bar_button("tb-start", ICON_W, start_on, cx, t)
            .absolute(start_x, PILL_Y)
            .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.open(Panel::Start, cx))
            .child(canvas(move |s, b| icons::start_mark(s, b, tile, hole)).size(20.0));

        let pager_x = start_x + ICON_W + 8.0;
        let pager = self.desktops.state().map(|st| self.pager(&st, pager_x, cx, t));
        self.pager_span = pager.as_ref().map(|(_, pw)| (pager_x, pager_x + pw));
        let items_x = match &pager {
            Some((_, pw)) => pager_x + pw + 8.0,
            None => start_x + ICON_W + 12.0,
        };

        let right = self.tray_and_clock(cx, t);
        let right_w = self.right_width(cx);
        let items = self.items();
        let slots = taskbar::layout(items.len(), items_x, (w - right_w - 12.0 - items_x).max(0.0));
        let active = self.active_window();
        let mut rects = Vec::new();
        let mut buttons = Vec::new();
        let mut hovered = None;
        for (item, (x, bw)) in items.iter().zip(slots) {
            // Every window of an app minimizes into its button.
            rects.extend(item.windows.iter().map(|id| (*id, [x as i32, PILL_Y as i32, bw as i32, PILL_H as i32])));
            if cx.hovered(("tb-app", item.key())) {
                hovered = Some((item.key(), self.item_label(item), x + bw / 2.0));
            }
            buttons.push(self.item_button(item, x, bw, active, cx, t));
        }
        self.send_rects(rects, cx);
        self.set_hover(hovered, w);
        self.sync_layers(cx);

        div()
            .w_full()
            .h(BAR_H)
            .bg(t.bar)
            .child(div().absolute(0.0, 0.0).w(w).h(1.0 / cx.scale.max(1.0)).bg(t.bar_edge))
            .child(start)
            .children(pager.map(|(p, _)| p))
            .children(buttons)
            .child(right.absolute(w - right_w, 0.0))
    }

    // -- the pager --------------------------------------------------------------

    /// One small screen per desktop with its windows outlined, the current
    /// one marked; returns it and its width.
    fn pager(&self, st: &Spaces, x: f32, cx: &Cx, t: &Theme) -> (Div<Self>, f32) {
        let (sw, sh) = (st.screen.0.max(1.0), st.screen.1.max(1.0));
        let cell_w = (SPACE_H * sw / sh).round().clamp(SPACE_H, SPACE_H * 3.0);
        let (kx, ky) = (cell_w / sw, SPACE_H / sh);
        let hair = 1.0 / cx.scale.max(1.0);
        let (win_fill, win_edge) = if t.dark { (rgb(0x4a4a4e), rgba(0xffffff, 0.45)) } else { (rgb(0xffffff), rgba(0x000000, 0.35)) };
        let mut pager = div().absolute(x, (BAR_H - SPACE_H) / 2.0).h(SPACE_H).flex_row().gap(GAP);
        for (i, space) in st.spaces.iter().enumerate() {
            let on = i == st.active;
            let id = ("tb-space", i);
            let mut cell = div()
                .id(id)
                .w(cell_w)
                .h(SPACE_H)
                .rounded(5.0)
                .clip()
                .bg(if on { t.accent.alpha(if t.dark { 0.28 } else { 0.16 }) } else { t.fill })
                .hover_bg(if on { t.accent.alpha(if t.dark { 0.34 } else { 0.22 }) } else { t.bar_hover.alpha(2.0) })
                .on_click(move |s: &mut Self, cx: &mut Cx| {
                    s.close(cx);
                    s.desktops.switch(i);
                });
            for w in space.windows.iter().filter(|w| !w.minimized) {
                let [wx, wy, ww, wh] = w.rect;
                cell = cell.child(
                    div()
                        .absolute((wx * kx).round(), (wy * ky).round())
                        .w((ww * kx).round().max(3.0))
                        .h((wh * ky).round().max(3.0))
                        .rounded(1.5)
                        .bg(win_fill)
                        .border(if on { t.accent.alpha(0.8) } else { win_edge }, hair.max(0.75)),
                );
            }
            // The border goes over the windows: a last child the cell's size.
            let edge = if on { (t.accent, 1.5) } else { (t.separator.alpha(1.5), hair.max(0.75)) };
            cell = cell.child(div().absolute(0.0, 0.0).w(cell_w).h(SPACE_H).rounded(5.0).border(edge.0, edge.1));
            pager = pager.child(cell);
        }
        let n = st.spaces.len() as f32;
        (pager, n * cell_w + (n - 1.0).max(0.0) * GAP)
    }

    /// The wheel over the pager steps through desktops (up: the previous
    /// one). Returns false when the pointer isn't over it.
    pub(super) fn pager_scrolled(&mut self, dy: f32, cx: &mut Cx) -> bool {
        let Some((x0, x1)) = self.pager_span else { return false };
        if cx.mouse.x < x0 || cx.mouse.x > x1 {
            return false;
        }
        let Some(st) = self.desktops.state() else { return false };
        if self.pager_scroll * dy < 0.0 {
            self.pager_scroll = 0.0;
        }
        self.pager_scroll += dy;
        if self.pager_scroll.abs() >= SPACE_STEP {
            let back = self.pager_scroll > 0.0;
            self.pager_scroll = 0.0;
            let n = if back { st.active.checked_sub(1) } else { Some(st.active + 1).filter(|n| *n < st.spaces.len()) };
            if let Some(n) = n {
                self.desktops.switch(n);
            }
        }
        true
    }

    // -- app buttons ------------------------------------------------------------

    /// An app's name, or the window's title if it has just the one.
    fn item_label(&self, item: &Item) -> String {
        let name = match item.app().and_then(|a| self.app(a)) {
            Some(e) => e.name.clone(),
            None => crate::apps::display_name(&self.apps, &item.app_id),
        };
        match item.windows.as_slice() {
            [one] => self.window(*one).map(|w| w.title.clone()).filter(|t| !t.is_empty()).unwrap_or(name),
            [] => name,
            more => format!("{name} \u{2014} {} windows", more.len()),
        }
    }

    fn item_button(&self, item: &Item, x: f32, w: f32, active: Option<u64>, cx: &Cx, t: &Theme) -> Div<Self> {
        let id = ("tb-app", item.key());
        let name = match item.app().and_then(|a| self.app(a)) {
            Some(e) => e.name.clone(),
            None => crate::apps::display_name(&self.apps, &item.app_id),
        };
        let windows: Vec<_> = item.windows.iter().filter_map(|id| self.window(*id)).collect();
        let is_active = active.is_some_and(|a| item.windows.contains(&a));
        let all_minimized = !windows.is_empty() && windows.iter().all(|w| w.minimized);
        let icon_size = ICON.min(w - 8.0);
        let mut icon = div().size(icon_size).child(self.app_icon(item.app(), &item.app_id, &name, icon_size));
        if all_minimized {
            icon = icon.opacity(0.5);
        }
        let mut button = self
            .bar_button(id, w, is_active, cx, t)
            .absolute(x, PILL_Y)
            .on_click({
                let item = item.clone();
                move |s: &mut Self, cx: &mut Cx| s.item_clicked(&item, cx)
            })
            .on_middle_click({
                let app = item.app.clone();
                move |s: &mut Self, cx: &mut Cx| {
                    if let Some(a) = &app {
                        s.launch_app(a, cx);
                    }
                }
            })
            .on_right_click({
                let item = item.clone();
                move |s: &mut Self, cx: &mut Cx| s.open_task_menu(item.clone(), x, cx)
            })
            .child(icon);
        // Running: a short dash under the icon (two for several windows),
        // longer and accent for the active app.
        if !windows.is_empty() {
            let (seg, color) = match (is_active, windows.len() > 1) {
                (true, false) => (16.0, t.accent),
                (true, true) => (7.0, t.accent),
                (false, false) => (6.0, t.bar_running),
                (false, true) => (4.0, t.bar_running),
            };
            let color = if all_minimized { color.alpha(0.6) } else { color };
            let count = windows.len().min(2) as f32;
            let total = seg * count + 2.5 * (count - 1.0);
            for k in 0..count as usize {
                button = button.child(
                    div()
                        .absolute(((w - total) / 2.0 + k as f32 * (seg + 2.5)).round(), PILL_H - 4.0)
                        .w(seg)
                        .h(2.5)
                        .rounded(1.25)
                        .bg(color),
                );
            }
        }
        button
    }

    // -- the tooltip ------------------------------------------------------------

    /// Notes which app button the pointer is on. Once one tooltip is up,
    /// moving to the next button shows its tooltip right away.
    fn set_hover(&mut self, hovered: Option<(String, String, f32)>, bar_w: f32) {
        let Some((key, label, x)) = hovered else {
            self.hover = None;
            return;
        };
        let x = x.clamp(0.0, bar_w);
        if let Some(h) = self.hover.as_mut().filter(|h| h.key == key) {
            (h.label, h.x, h.bar_w) = (label, x, bar_w);
            return;
        }
        let since = if self.shown_tooltip.is_some() {
            Instant::now().checked_sub(TOOLTIP_DELAY).unwrap_or_else(Instant::now)
        } else {
            Instant::now()
        };
        self.hover = Some(Hover { key, label, x, bar_w, since });
    }

    /// Where the tooltip surface goes, if one is due.
    pub(super) fn tooltip_options(&self, cx: &mut Cx) -> Option<LayerOptions> {
        let h = self.hover.as_ref().filter(|h| h.since.elapsed() >= TOOLTIP_DELAY - Duration::from_millis(5))?;
        if self.drawn != Panel::None {
            return None;
        }
        let tw = cx.text.measure(&h.label, &TIP, None, false).0;
        let w = (tw.min(TIP_MAX_W) + 2.0 * TIP_PAD + 2.0 * TIP_SHADOW).ceil();
        let left = (h.x - w / 2.0).clamp(0.0, (h.bar_w - w).max(0.0));
        Some(LayerOptions {
            layer: Layer::Overlay,
            anchor: Anchor::BOTTOM | Anchor::LEFT,
            size: (w as u32, (24.0 + 2.0 * TIP_SHADOW) as u32),
            namespace: TOOLTIP.into(),
            margin: (0, 0, 0, left.round() as i32),
            ..Default::default()
        })
    }

    pub(super) fn tooltip(&self, t: &Theme) -> Div<Self> {
        let Some(h) = &self.hover else { return div() };
        div().p(TIP_SHADOW).child(
            div()
                .h(24.0)
                .px(TIP_PAD)
                .items_center()
                .rounded(6.0)
                .bg(t.menu)
                .border(t.border, 0.5)
                .shadow(t.shadow.alpha(0.5), 6.0, 1.0)
                .child(text(h.label.clone()).size(TIP.size).line_height(TIP.line_height).truncate().max_w(TIP_MAX_W)),
        )
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
            rows.push(Some((if item.windows.is_empty() { format!("Open {}", e.name) } else { "New Window".into() }, MenuAction::New)));
            rows.push(Some((if pinned { "Unpin from Taskbar" } else { "Pin to Taskbar" }.into(), MenuAction::TogglePin)));
        }
        if !item.windows.is_empty() {
            if !rows.is_empty() {
                rows.push(None);
            }
            let label = if item.windows.len() > 1 { "Close All Windows" } else { "Close Window" };
            rows.push(Some((label.into(), MenuAction::Close)));
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
                for id in item.windows {
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
