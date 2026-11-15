//! Control Center's expanded modules, like macOS: Wi-Fi (networks to join,
//! with the password asked for inline), Bluetooth (paired devices to
//! connect) and Sound (the output). Each has a back button, its switch, and
//! a way into the full settings at the bottom.

use std::time::Instant;

use ui::widgets::{spinner, spinner_phase};
use ui::*;

use super::control::{Glyph, PAD, WIDTH};
use super::theme::Theme;
use super::{Module, Panel, Shell, Slider, icons, launch};
use crate::system::{BtDevice, DeviceKind, Network, Output, Part, Request};

const INNER: f32 = WIDTH - 2.0 * PAD;
const ROW_H: f32 = 40.0;
const HEADER_H: f32 = 26.0;
/// The list scrolls past this.
const LIST_MAX: f32 = 380.0;

thread_local! {
    /// What the spinners turn relative to.
    static EPOCH: Instant = Instant::now();
}

impl Shell {
    /// Expands a module (or goes back to the main grid with `Module::Main`).
    pub(super) fn expand(&mut self, m: Module, cx: &mut Cx) {
        if self.module == m {
            return;
        }
        self.module = m;
        self.forget_password(cx);
        match m {
            Module::Wifi => {
                self.send(Request::Refresh(Part::Wifi));
                if self.status.wifi.enabled {
                    self.send(Request::ScanWifi);
                }
            }
            Module::Bluetooth => self.send(Request::Refresh(Part::Bluetooth)),
            Module::Sound => self.send(Request::Refresh(Part::Outputs)),
            Module::Main => {}
        }
        cx.scroll_to_top("cc-list");
        self.start_swap(m != Module::Main, cx);
    }

    /// Opens Quick Settings with a module expanded and nothing moving (for
    /// snapshots).
    pub fn expand_now(&mut self, m: Module, cx: &mut Cx) {
        self.expand(m, cx);
        self.swap = None;
    }

    pub(super) fn forget_password(&mut self, cx: &mut Cx) {
        self.wifi_pick = None;
        self.secret.clear();
        self.password.borrow_mut().set_text(&mut cx.text.fonts, "");
        if cx.is_focused("cc-password") {
            cx.blur();
        }
    }

    /// Something in an expanded module is busy and shows a spinner.
    pub(super) fn spinning(&self) -> bool {
        let st = &self.status;
        self.drawn == Panel::QuickSettings
            && match self.module {
                Module::Wifi => st.scanning || st.joining.is_some(),
                Module::Bluetooth => st.device_busy.is_some(),
                _ => false,
            }
    }

    fn spinner(&self, size: f32, t: &Theme) -> Div<Self> {
        div().size(size).shrink_0().child(spinner(size, spinner_phase(EPOCH.with(|e| *e)), t.secondary))
    }

    /// Opens the full settings for a module and closes the flyout.
    fn open_settings(&mut self, m: Module, cx: &mut Cx) {
        use crate::system::has;
        let kcm = match m {
            Module::Wifi => "kcm_networkmanagement",
            Module::Bluetooth => "kcm_bluetooth",
            _ => "kcm_pulseaudio",
        };
        let cmd = if has("kcmshell6") {
            format!("kcmshell6 {kcm}")
        } else if has("systemsettings") {
            format!("systemsettings {kcm}")
        } else if m == Module::Wifi && has("nm-connection-editor") {
            "nm-connection-editor".into()
        } else if m == Module::Sound && has("pavucontrol") {
            "pavucontrol".into()
        } else if m == Module::Bluetooth && has("blueman-manager") {
            "blueman-manager".into()
        } else {
            eprintln!("y7-shell: no settings app for {m:?}");
            return;
        };
        launch(&cmd);
        self.close(cx);
    }

    // -- the password field ---------------------------------------------------------

    /// Asks for a network's password inline, or joins it right away.
    fn pick_network(&mut self, n: Network, cx: &mut Cx) {
        if n.active || self.status.joining.is_some() {
            return;
        }
        if n.enterprise() {
            return self.open_settings(Module::Wifi, cx);
        }
        if n.secured() && !n.known {
            self.forget_password(cx);
            self.status.join_error = None;
            self.wifi_pick = Some(n);
            cx.focus("cc-password");
            return;
        }
        self.join(n, None);
    }

    /// Picks a network and types its password (for snapshots).
    pub fn type_password_now(&mut self, ssid: &str, password: &str, cx: &mut Cx) {
        self.status = self.services.status();
        if let Some(n) = self.status.networks.iter().find(|n| n.ssid == ssid).cloned() {
            self.pick_network(n, cx);
            self.secret = password.to_string();
            self.sync_password(cx);
        }
    }

    fn join(&mut self, n: Network, password: Option<String>) {
        // Shown at once; the worker confirms it a moment later.
        self.status.joining = Some(n.ssid.clone());
        self.status.join_error = None;
        self.send(Request::JoinWifi(n, password));
    }

    fn join_picked(&mut self, cx: &mut Cx) {
        if self.secret.is_empty() {
            return;
        }
        let Some(n) = self.wifi_pick.clone() else { return };
        let password = std::mem::take(&mut self.secret);
        self.join(n, Some(password));
        self.sync_password(cx);
    }

    /// Typing in the password field: it keeps the text and shows dots.
    pub(super) fn password_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if key.is_named(NamedKey::Enter) {
            self.join_picked(cx);
            return true;
        }
        if key.is_named(NamedKey::Backspace) {
            if key.ctrl {
                self.secret.clear();
            } else {
                self.secret.pop();
            }
        } else if key.ctrl && key.is_char('v') {
            let pasted = cx.clipboard.as_mut().and_then(|c| c.get_text().ok()).unwrap_or_default();
            self.secret.push_str(pasted.lines().next().unwrap_or(""));
        } else if let Some(typed) = key.text.as_ref().filter(|s| !key.ctrl && !key.alt && !s.chars().any(char::is_control)) {
            self.secret.push_str(typed);
        } else {
            return true;
        }
        // A new attempt clears the last one's complaint.
        self.status.join_error = None;
        self.sync_password(cx);
        true
    }

    fn sync_password(&mut self, cx: &mut Cx) {
        let dots = "\u{2022}".repeat(self.secret.chars().count());
        let mut p = self.password.borrow_mut();
        p.set_text(&mut cx.text.fonts, &dots);
        p.move_to_end();
        cx.request_redraw();
    }

    // -- views ------------------------------------------------------------------------

    pub(super) fn module_view(&mut self, m: Module, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let st = self.status.clone();
        let (title, switch): (&str, Option<(bool, fn(&mut Self))>) = match m {
            Module::Wifi => ("Wi-Fi", st.wifi.available.then_some((st.wifi.enabled, Self::toggle_wifi as fn(&mut Self)))),
            Module::Bluetooth => ("Bluetooth", st.bluetooth.map(|on| (on, Self::toggle_bluetooth as fn(&mut Self)))),
            _ => ("Sound", None),
        };
        let body = match m {
            Module::Wifi => self.wifi_list(cx, t),
            Module::Bluetooth => self.device_list(cx, t),
            _ => self.sound_list(cx, t),
        };
        let settings = match m {
            Module::Wifi => "Wi-Fi Settings\u{2026}",
            Module::Bluetooth => "Bluetooth Settings\u{2026}",
            _ => "Sound Settings\u{2026}",
        };
        div()
            .w(INNER)
            .flex_col()
            .child(self.module_header(title, switch, cx, t))
            .child(hairline(t, cx))
            .child(body)
            .child(hairline(t, cx))
            .child(self.menu_row("cc-settings", settings, cx, t).on_click(move |s: &mut Self, cx: &mut Cx| s.open_settings(m, cx)))
    }

    /// "‹  Wi-Fi                      ( o)"
    fn module_header(&self, title: &str, switch: Option<(bool, fn(&mut Self))>, cx: &Cx, t: &Theme) -> Div<Self> {
        let back_key = "cc-back";
        let ink = t.label;
        let back_bg = if cx.hovered(back_key) && cx.mouse_down {
            t.fill.alpha(2.6)
        } else if cx.hovered(back_key) {
            t.fill.alpha(2.0)
        } else {
            t.fill.alpha(1.2)
        };
        let back = div()
            .id(back_key)
            .size(26.0)
            .shrink_0()
            .rounded(13.0)
            .bg(back_bg)
            .items_center()
            .justify_center()
            .on_click(|s: &mut Self, cx: &mut Cx| s.expand(Module::Main, cx))
            .child(canvas(move |s, b| icons::chevron_sized(s, Bounds::new(b.x - 0.5, b.y, b.w, b.h), ink, false, 10.0, 1.8)).size(16.0));
        let mut row = div()
            .h(40.0)
            .pb(6.0)
            .flex_row()
            .items_center()
            .gap(10.0)
            .child(back)
            .child(text(title.to_string()).size(15.0).weight(Weight::Bold).flex_1());
        if let Some((on, f)) = switch {
            row = row.child(toggle_switch("cc-switch", on, t).on_click(move |s: &mut Self, _: &mut Cx| f(s)));
        }
        row
    }

    /// A full-width row of plain text, like "Wi-Fi Settings…".
    fn menu_row(&self, id: &'static str, label: &str, cx: &Cx, t: &Theme) -> Div<Self> {
        let bg = if cx.hovered(id) && cx.mouse_down {
            Some(t.hover.alpha(2.4))
        } else {
            cx.hovered(id).then_some(t.hover.alpha(1.5))
        };
        div()
            .id(id)
            .mt(6.0)
            .h(30.0)
            .px(8.0)
            .rounded(8.0)
            .flex_row()
            .items_center()
            .bg_opt(bg)
            .child(text(label.to_string()).size(13.0))
    }

    /// The list part of a module: scrolls when it's long.
    fn list(&self, content: Div<Self>, height: f32) -> Div<Self> {
        let h = height.min(LIST_MAX);
        let inner = content.w(INNER).pt(4.0).pb(2.0);
        if height <= LIST_MAX {
            return inner;
        }
        div().id("cc-list").w(INNER).h(h).flex_col().scroll_y().child(inner.shrink_0())
    }

    fn wifi_list(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let st = self.status.clone();
        if !st.wifi.enabled {
            return div().h(52.0).items_center().justify_center().child(text("Wi-Fi is off.").size(12.0).color(t.secondary));
        }
        let (known, other): (Vec<Network>, Vec<Network>) = st.networks.iter().cloned().partition(|n| n.known || n.active);
        let mut col = div().flex_col();
        let mut height = 10.0;
        let others = if known.is_empty() { "Networks" } else { "Other Networks" };
        if !known.is_empty() {
            col = col.child(section_title("Known Networks", None, t));
            height += HEADER_H;
            for n in known {
                height += self.network_height(&n);
                col = col.child(self.network_row(n, cx, t));
            }
        }
        let busy = st.scanning.then(|| self.spinner(12.0, t));
        col = col.child(section_title(others, busy, t));
        height += HEADER_H;
        if other.is_empty() {
            let note = if st.scanning { "Looking for networks\u{2026}" } else { "No other networks found." };
            col = col.child(div().h(30.0).pl(8.0).flex_row().items_center().child(text(note).size(12.0).color(t.secondary)));
            height += 30.0;
        }
        for n in other {
            height += self.network_height(&n);
            col = col.child(self.network_row(n, cx, t));
        }
        self.list(col, height)
    }

    fn network_height(&self, n: &Network) -> f32 {
        let picked = self.wifi_pick.as_ref().is_some_and(|p| p.ssid == n.ssid);
        let failed = self.status.join_error.as_ref().is_some_and(|(s, _)| *s == n.ssid);
        ROW_H + if picked { 76.0 } else if failed { 18.0 } else { 0.0 }
    }

    fn network_row(&mut self, n: Network, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let st = &self.status;
        let joining = st.joining.as_deref() == Some(n.ssid.as_str());
        let picked = self.wifi_pick.as_ref().is_some_and(|p| p.ssid == n.ssid);
        let error = st.join_error.as_ref().filter(|(s, _)| *s == n.ssid).map(|(_, e)| e.clone());
        let key = ("cc-net", n.ssid.clone());
        let strength = n.strength;
        // The unlit arcs are drawn opaque: translucent strokes overlap
        // into dots where their segments meet.
        let circle = if n.active { t.accent } else { over(self.circle_fill(t), t.panel) };
        let glyph: Glyph = Box::new(move |s, b, c| icons::signal(s, b, over(c, circle), over(c.alpha(0.3), circle), strength));
        let mut trailing = div().flex_row().items_center().gap(8.0);
        if joining {
            trailing = trailing.child(self.spinner(14.0, t));
        }
        if n.secured() {
            let ink = t.secondary;
            trailing = trailing.child(canvas(move |s, b| icons::padlock(s, b, ink)).w(12.0).h(16.0));
        }
        let detail = if n.active { Some("Connected".to_string()) } else { joining.then(|| "Joining\u{2026}".to_string()) };
        let net = n.clone();
        let row = self
            .device_row(key, glyph, n.ssid.clone(), detail, n.active, picked, cx, t)
            .child(trailing)
            .on_click(move |s: &mut Self, cx: &mut Cx| s.pick_network(net.clone(), cx));
        let mut col = div().flex_col().child(row);
        if picked {
            col = col.child(self.password_form(error.as_deref(), joining, cx, t));
        } else if let Some(e) = error {
            col = col.child(text(e).size(11.0).color(t.destructive).pl(46.0).h(18.0).truncate());
        }
        col
    }

    fn password_form(&mut self, error: Option<&str>, joining: bool, cx: &mut Cx, t: &Theme) -> Div<Self> {
        {
            let mut p = self.password.borrow_mut();
            p.set_styles(TextStyle { size: 13.0, line_height: 1.2, weight: Weight::Regular, color: t.label }, None);
            p.placeholder = "Password".into();
            p.placeholder_color = t.tertiary;
            p.caret_color = t.accent;
            p.selection_color = t.accent.alpha(0.3);
        }
        let focused = cx.is_focused("cc-password");
        let field = div()
            .h(28.0)
            .flex_1()
            .px(8.0)
            .rounded(7.0)
            .bg(if t.dark { t.fill } else { Color::WHITE })
            .border(if focused { t.accent.alpha(0.8) } else { t.border }, if focused { 1.5 } else { 0.5 })
            .flex_row()
            .items_center()
            .child(input("cc-password", &self.password).flex_1());
        let can_join = !self.secret.is_empty() && !joining;
        let join = button("cc-join", "Join", true, can_join, cx, t).on_click(|s: &mut Self, cx: &mut Cx| s.join_picked(cx));
        let cancel = button("cc-cancel", "Cancel", false, true, cx, t).on_click(|s: &mut Self, cx: &mut Cx| s.forget_password(cx));
        let note = error.map(|e| text(e.to_string()).size(11.0).color(t.destructive).truncate());
        div()
            .flex_col()
            .gap(8.0)
            .pl(46.0)
            .pr(8.0)
            .pb(10.0)
            .child(div().flex_row().child(field))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap(6.0)
                    .child(div().flex_1().min_w_0().children(note))
                    .child(cancel)
                    .child(join),
            )
    }

    fn device_list(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let st = self.status.clone();
        if st.bluetooth != Some(true) {
            let note = if st.bluetooth.is_none() { "No Bluetooth adapter." } else { "Bluetooth is off." };
            return div().h(52.0).items_center().justify_center().child(text(note).size(12.0).color(t.secondary));
        }
        let mut col = div().flex_col().child(section_title("Devices", None, t));
        let mut height = HEADER_H + 10.0;
        if st.devices.is_empty() {
            col = col.child(div().h(30.0).pl(8.0).flex_row().items_center().child(text("No paired devices.").size(12.0).color(t.secondary)));
            height += 30.0;
        }
        for d in st.devices {
            height += ROW_H;
            col = col.child(self.bt_row(d, cx, t));
        }
        self.list(col, height)
    }

    fn bt_row(&self, d: BtDevice, cx: &Cx, t: &Theme) -> Div<Self> {
        let busy = self.status.device_busy.as_deref() == Some(d.path.as_str());
        let detail = if busy {
            if d.connected { "Disconnecting\u{2026}" } else { "Connecting\u{2026}" }
        } else if d.connected {
            "Connected"
        } else {
            "Not Connected"
        };
        let mut trailing = div().flex_row().items_center().gap(4.0);
        if busy {
            trailing = trailing.child(self.spinner(14.0, t));
        } else if let Some(pct) = d.battery.filter(|_| d.connected) {
            let (ink, fill) = (t.secondary, if pct <= 20 { t.destructive } else { t.secondary });
            trailing = trailing
                .child(text(format!("{pct}%")).size(11.0).color(t.secondary).tabular())
                .child(canvas(move |s, b| icons::battery(s, b, ink, fill, pct, false)).w(28.0).h(16.0));
        }
        let (path, on) = (d.path.clone(), !d.connected);
        let key = ("cc-dev", d.path.clone());
        self.device_row(key, kind_glyph(d.kind), d.name, Some(detail.into()), d.connected, false, cx, t)
            .child(trailing)
            .on_click(move |s: &mut Self, _: &mut Cx| {
                if s.status.device_busy.is_none() {
                    s.status.device_busy = Some(path.clone());
                    s.send(Request::ConnectBt(path.clone(), on));
                }
            })
    }

    fn sound_list(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let st = self.status.clone();
        let mut col = div().flex_col();
        let mut height = 10.0;
        if let Some(v) = st.volume {
            col = col.child(div().px(4.0).pt(6.0).pb(8.0).child(self.slider_bar(Slider::Volume, if v.muted { 0.0 } else { v.level }, v.muted, t)));
            height += 38.0;
        }
        col = col.child(section_title("Output", None, t));
        height += HEADER_H;
        if st.outputs.is_empty() {
            col = col.child(div().h(30.0).pl(8.0).flex_row().items_center().child(text("No outputs.").size(12.0).color(t.secondary)));
            height += 30.0;
        }
        for o in st.outputs {
            height += ROW_H;
            col = col.child(self.output_row(o, cx, t));
        }
        self.list(col, height)
    }

    fn output_row(&self, o: Output, cx: &Cx, t: &Theme) -> Div<Self> {
        let name = o.name.clone();
        let key = ("cc-out", o.name.clone());
        let accent = t.accent;
        let mut row = self.device_row(key, kind_glyph(o.kind), o.description, None, o.default, false, cx, t);
        if o.default {
            row = row.child(canvas(move |s, b| icons::check(s, b, accent)).size(16.0));
        }
        row.on_click(move |s: &mut Self, _: &mut Cx| {
            for out in &mut s.status.outputs {
                out.default = out.name == name;
            }
            let outputs = s.status.outputs.clone();
            s.services.optimistic(|st| st.outputs = outputs);
            s.send(Request::SetOutput(name.clone()));
        })
    }

    /// The circle behind a row's icon when it's off.
    fn circle_fill(&self, t: &Theme) -> Color {
        t.fill.alpha(if t.dark { 1.8 } else { 1.6 })
    }

    /// A list row: an icon on a circle (accent when `on`), a name and an
    /// optional detail line. Callers add what goes at the right.
    #[allow(clippy::too_many_arguments)]
    fn device_row(
        &self,
        key: (&'static str, String),
        glyph: Glyph,
        name: String,
        detail: Option<String>,
        on: bool,
        selected: bool,
        cx: &Cx,
        t: &Theme,
    ) -> Div<Self> {
        let hovered = cx.hovered(key.clone());
        let bg = if hovered && cx.mouse_down {
            Some(t.hover.alpha(2.4))
        } else {
            (hovered || selected).then_some(t.hover.alpha(1.5))
        };
        let (circle, ink) = if on { (t.accent, Color::WHITE) } else { (self.circle_fill(t), t.label) };
        let mut labels = div().flex_col().flex_1().min_w_0().child(text(name).size(13.0).truncate());
        if let Some(d) = detail {
            labels = labels.child(text(d).size(11.0).color(t.secondary).truncate());
        }
        div()
            .id(key)
            .h(ROW_H)
            .px(6.0)
            .rounded(8.0)
            .flex_row()
            .items_center()
            .gap(10.0)
            .bg_opt(bg)
            .child(
                div()
                    .size(28.0)
                    .shrink_0()
                    .rounded(14.0)
                    .bg(circle)
                    .items_center()
                    .justify_center()
                    .child(canvas(move |s, b| glyph(s, b, ink)).size(20.0)),
            )
            .child(labels)
    }
}

/// A one-pixel separator, snapped to the pixel grid (a hairline between
/// two pixel rows blurs away).
fn hairline(t: &Theme, cx: &Cx) -> Div<Shell> {
    let (scale, color) = (cx.scale.max(1.0), t.separator);
    div().h(1.0).w(INNER).shrink_0().child(
        canvas(move |s, b| {
            let y = (b.y * scale).round() / scale;
            s.rect(Bounds::new(b.x, y, b.w, 1.0 / scale), color, 0.0);
        })
        .w(INNER)
        .h(1.0),
    )
}

/// `top` painted over the opaque `bottom`.
fn over(top: Color, bottom: Color) -> Color {
    let mix = |a: f32, b: f32| a * top.a + b * (1.0 - top.a);
    Color { r: mix(top.r, bottom.r), g: mix(top.g, bottom.g), b: mix(top.b, bottom.b), a: 1.0 }
}

fn section_title(title: &str, trailing: Option<Div<Shell>>, t: &Theme) -> Div<Shell> {
    let mut row = div()
        .h(HEADER_H)
        .pl(8.0)
        .pb(4.0)
        .flex_row()
        .items_end()
        .gap(6.0)
        .child(text(title.to_string()).size(11.0).weight(Weight::Semibold).color(t.secondary));
    if let Some(c) = trailing {
        row = row.child(div().pb(1.0).child(c));
    }
    row
}

fn kind_glyph(kind: DeviceKind) -> Glyph {
    match kind {
        DeviceKind::Headphones => Box::new(icons::headphones),
        DeviceKind::Speaker => Box::new(icons::speaker_box),
        DeviceKind::Mouse => Box::new(icons::mouse),
        DeviceKind::Keyboard => Box::new(icons::keyboard),
        DeviceKind::Phone => Box::new(icons::phone),
        DeviceKind::Computer => Box::new(icons::computer),
        DeviceKind::Gamepad => Box::new(icons::gamepad),
        DeviceKind::Other => Box::new(icons::bluetooth),
    }
}

/// A macOS switch: a capsule, accent when on, with a white knob.
fn toggle_switch(id: &'static str, on: bool, t: &Theme) -> Div<Shell> {
    let track = if on {
        t.accent
    } else if t.dark {
        rgba(0xffffff, 0.16)
    } else {
        rgba(0x000000, 0.12)
    };
    let shadow = t.shadow;
    div().id(id).w(34.0).h(20.0).shrink_0().child(
        canvas(move |s, b| {
            s.rect(b, track, b.h / 2.0);
            let d = b.h - 3.0;
            let x = if on { b.right() - d - 1.5 } else { b.x + 1.5 };
            let knob = Bounds::new(x, b.y + 1.5, d, d);
            s.shadow(knob, shadow.alpha(0.5), d / 2.0, 2.0);
            s.rect(knob, Color::WHITE, d / 2.0);
        })
        .w(34.0)
        .h(20.0),
    )
}

/// A small push button; `default` is accent-filled.
fn button(id: &'static str, label: &str, default: bool, enabled: bool, cx: &Cx, t: &Theme) -> Div<Shell> {
    let pressed = enabled && cx.hovered(id) && cx.mouse_down;
    let (bg, fg) = match (default, enabled) {
        (true, true) => (if pressed { t.accent.alpha(0.8) } else { t.accent }, Color::WHITE),
        (true, false) => (t.accent.alpha(0.4), Color::WHITE.alpha(0.7)),
        _ => (if pressed { t.fill.alpha(2.8) } else { t.fill.alpha(1.8) }, t.label),
    };
    div()
        .id(id)
        .h(24.0)
        .px(12.0)
        .rounded(6.0)
        .bg(bg)
        .items_center()
        .justify_center()
        .child(text(label.to_string()).size(12.0).weight(Weight::Medium).color(fg))
}
