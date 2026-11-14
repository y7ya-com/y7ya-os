//! The menu bar, its menus, and the dialogs they open.

use ui::*;

use super::theme::Theme;
use super::{BAR_H, MenuKind, Panel, Shell, icons, launch};
use crate::system::{Request, SessionAction};
use crate::toplevel::WindowAction;

#[derive(Clone, Copy)]
enum Action {
    About,
    Settings,
    Session(SessionAction),
    Confirm(SessionAction),
    Window(WindowAction),
}

struct Item {
    label: String,
    shortcut: &'static str,
    /// `None` draws a separator.
    action: Option<Action>,
}

fn item(label: impl Into<String>, shortcut: &'static str, action: Action) -> Item {
    Item { label: label.into(), shortcut, action: Some(action) }
}

fn separator() -> Item {
    Item { label: String::new(), shortcut: "", action: None }
}

const MENU_W: f32 = 236.0;

impl Shell {
    fn app_name(&self) -> String {
        match &self.active {
            Some(w) if !w.app_id.is_empty() => crate::apps::display_name(&self.apps, &w.app_id),
            Some(w) if !w.title.is_empty() => w.title.clone(),
            _ => "Desktop".into(),
        }
    }

    pub(super) fn menubar(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let ink = t.bar_text;
        let open = self.panel;
        let title = |id: &'static str, kind: MenuKind, slot: usize, content: AnyElement<Self>| {
            let x = self.menu_x[slot].clone();
            div()
                .id(id)
                .h(22.0)
                .px(9.0)
                .rounded(6.0)
                .items_center()
                .bg_opt((open == Panel::Menu(kind)).then_some(t.bar_highlight))
                .on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.open(Panel::Menu(kind), cx))
                // Remember where the title is so the menu opens under it.
                .child(canvas(move |_, b| x.set(b.x)).inset_0())
                .child(content)
        };

        let logo = title("menu-system", MenuKind::System, 0, canvas(move |s, b| icons::logo(s, b, ink)).size(16.0).into());
        let app = title(
            "menu-app",
            MenuKind::App,
            1,
            text(self.app_name()).weight(Weight::Bold).color(ink).into(),
        );
        let left = div().flex_row().items_center().gap(2.0).child(logo).child(app);

        let st = &self.status;
        let dim = ink.alpha(0.3);
        let mut right = div().flex_row().items_center().gap(2.0);
        let status_item = |id: &'static str, w: f32, active: bool, panel: Option<Panel>| {
            let d = div()
                .id(id)
                .h(22.0)
                .w(w)
                .rounded(6.0)
                .items_center()
                .justify_center()
                .bg_opt(active.then_some(t.bar_highlight));
            match panel {
                Some(p) => d.on_mouse_down(move |s: &mut Self, cx: &mut Cx| s.open(p, cx)),
                None => d,
            }
        };
        if self.store.lock().unwrap().dnd {
            right = right.child(
                status_item("status-focus", 28.0, false, Some(Panel::ControlCenter))
                    .child(canvas(move |s, b| icons::moon(s, b, ink)).size(18.0)),
            );
        }
        if let Some(v) = st.volume {
            right = right.child(
                status_item("status-volume", 32.0, false, Some(Panel::ControlCenter))
                    .child(canvas(move |s, b| icons::speaker(s, b, ink, v.level, v.muted)).size(20.0)),
            );
        }
        if let Some(bat) = st.battery {
            let fill = if bat.charging {
                t.success
            } else if bat.percent <= 20 {
                t.destructive
            } else {
                ink
            };
            right = right.child(
                status_item("status-battery", 38.0, false, Some(Panel::ControlCenter))
                    .child(canvas(move |s, b| icons::battery(s, b, ink, fill, bat.percent, bat.charging)).w(30.0).h(20.0)),
            );
        }
        if st.wifi.available {
            let w = st.wifi.clone();
            right = right.child(
                status_item("status-wifi", 32.0, false, Some(Panel::ControlCenter)).child(
                    canvas(move |s, b| icons::wifi(s, b, ink, dim, w.strength, w.enabled && w.ssid.is_some())).size(20.0),
                ),
            );
        }
        right = right
            .child(
                status_item("status-spotlight", 30.0, open == Panel::Spotlight, Some(Panel::Spotlight))
                    .child(canvas(move |s, b| icons::magnifier(s, b, ink, 14.0)).size(18.0)),
            )
            .child(
                status_item("status-cc", 32.0, open == Panel::ControlCenter, Some(Panel::ControlCenter))
                    .child(canvas(move |s, b| icons::control_center(s, b, ink)).size(20.0)),
            )
            .child(
                div()
                    .id("status-clock")
                    .h(22.0)
                    .px(8.0)
                    .rounded(6.0)
                    .items_center()
                    .bg_opt((open == Panel::NotificationCenter).then_some(t.bar_highlight))
                    .on_mouse_down(|s: &mut Self, cx: &mut Cx| s.open(Panel::NotificationCenter, cx))
                    .child(text(clock_text(&chrono::Local::now())).color(ink).weight(Weight::Medium)),
            );

        let _ = cx;
        div()
            .w_full()
            .h(BAR_H)
            .bg(t.bar)
            .flex_row()
            .items_center()
            .justify_between()
            .px(6.0)
            .child(left)
            .child(right)
    }

    /// The overlay under the menu bar: whatever panel is open, over an
    /// invisible backdrop that closes it when clicked.
    pub(super) fn panel_view(&mut self, cx: &mut Cx, t: &Theme) -> Div<Self> {
        let (w, h) = cx.window_size;
        let content = match self.panel {
            Panel::None => None,
            Panel::Menu(kind) => Some(self.menu(kind, cx, t)),
            Panel::Spotlight => Some(self.spotlight(cx, t)),
            Panel::ControlCenter => Some(self.control_center(cx, t)),
            Panel::NotificationCenter => Some(self.notification_center(cx, t)),
            Panel::Confirm(action) => Some(self.confirm(action, w, h, t)),
            Panel::About => Some(self.about(w, h, t)),
        };
        let mut root = div().id("backdrop").w(w).h(h).on_mouse_down(|s: &mut Self, cx: &mut Cx| s.close(cx));
        if matches!(self.panel, Panel::Confirm(_) | Panel::About) {
            // Dialogs dim the screen a little, like a sheet.
            root = root.bg(Color::BLACK.alpha(if t.dark { 0.25 } else { 0.12 }));
        }
        root.children(content)
    }

    fn menu(&self, kind: MenuKind, cx: &Cx, t: &Theme) -> Div<Self> {
        let user = std::env::var("USER").unwrap_or_default();
        let user = {
            let mut c = user.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        };
        let items = match kind {
            MenuKind::System => vec![
                item("About This Computer", "", Action::About),
                separator(),
                item("System Settings\u{2026}", "", Action::Settings),
                separator(),
                item("Sleep", "", Action::Session(SessionAction::Sleep)),
                item("Restart\u{2026}", "", Action::Confirm(SessionAction::Restart)),
                item("Shut Down\u{2026}", "", Action::Confirm(SessionAction::ShutDown)),
                separator(),
                item("Lock Screen", "\u{2303}\u{2318}Q", Action::Session(SessionAction::Lock)),
                item(format!("Log Out {user}\u{2026}"), "\u{21e7}\u{2318}Q", Action::Confirm(SessionAction::LogOut)),
            ],
            MenuKind::App => {
                let name = self.app_name();
                if self.active.is_none() {
                    vec![item("About This Computer", "", Action::About)]
                } else {
                    vec![
                        item(format!("Hide {name}"), "", Action::Window(WindowAction::Minimize)),
                        item("Zoom", "", Action::Window(WindowAction::ToggleMaximize)),
                        separator(),
                        item(format!("Quit {name}"), "\u{2318}Q", Action::Window(WindowAction::Close)),
                    ]
                }
            }
        };
        let x = self.menu_x[if kind == MenuKind::System { 0 } else { 1 }].get();
        let mut menu = div()
            .absolute(x.max(4.0), 2.0)
            .w(MENU_W)
            .flex_col()
            .p(5.0)
            .rounded(8.0)
            .bg(t.menu)
            .border(t.border, 0.5)
            .shadow(t.shadow, 24.0, 8.0)
            .occlude();
        for (i, it) in items.into_iter().enumerate() {
            let Some(action) = it.action else {
                menu = menu.child(div().h(1.0).mx(10.0).mt(5.0).mb(5.0).bg(t.separator));
                continue;
            };
            let id = ("menu-row", i);
            let hot = cx.hovered(id);
            let (fg, dim) = if hot { (Color::WHITE, Color::WHITE.alpha(0.8)) } else { (t.label, t.secondary) };
            menu = menu.child(
                div()
                    .id(id)
                    .h(22.0)
                    .px(10.0)
                    .rounded(4.0)
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .hover_bg(t.accent)
                    .on_click(move |s: &mut Self, cx: &mut Cx| s.menu_action(action, cx))
                    .child(text(it.label).color(fg))
                    .child(text(it.shortcut).color(dim)),
            );
        }
        menu
    }

    fn menu_action(&mut self, action: Action, cx: &mut Cx) {
        match action {
            Action::About => self.open(Panel::About, cx),
            Action::Settings => {
                self.close(cx);
                launch("assistant");
            }
            Action::Session(a) => self.session(a, cx),
            Action::Confirm(a) => self.open(Panel::Confirm(a), cx),
            Action::Window(a) => {
                self.close(cx);
                if let Some(t) = &self.toplevels {
                    t.act(a);
                }
            }
        }
    }

    fn confirm(&self, action: SessionAction, w: f32, h: f32, t: &Theme) -> Div<Self> {
        let (title, detail, button) = match action {
            SessionAction::Restart => ("Are you sure you want to restart your computer now?", "Open apps will be closed.", "Restart"),
            SessionAction::ShutDown => {
                ("Are you sure you want to shut down your computer now?", "Open apps will be closed.", "Shut Down")
            }
            _ => ("Are you sure you want to log out?", "Open apps will be closed.", "Log Out"),
        };
        let card_w = 272.0;
        let button_el = |id: &'static str, label: &'static str, primary: bool| {
            let d = div()
                .id(id)
                .flex_1()
                .h(28.0)
                .rounded(7.0)
                .items_center()
                .justify_center()
                .child(text(label).weight(Weight::Medium).color(if primary { Color::WHITE } else { t.label }));
            if primary { d.bg(t.accent) } else { d.bg(t.fill).hover_bg(t.hover) }
        };
        div()
            .absolute((w - card_w) / 2.0, (h - 230.0) / 2.5)
            .w(card_w)
            .flex_col()
            .items_center()
            .gap(10.0)
            .p(16.0)
            .rounded(14.0)
            .bg(t.menu)
            .border(t.border, 0.5)
            .shadow(t.shadow, 30.0, 10.0)
            .occlude()
            .child(app_mark(t, 52.0))
            .child(text(title).weight(Weight::Bold).w_full().line_height(1.35))
            .child(text(detail).size(11.0).color(t.secondary).w_full())
            .child(
                div()
                    .flex_row()
                    .gap(8.0)
                    .w_full()
                    .mt(4.0)
                    .child(button_el("confirm-cancel", "Cancel", false).on_click(|s: &mut Self, cx: &mut Cx| s.close(cx)))
                    .child(
                        button_el("confirm-ok", button, true)
                            .on_click(move |s: &mut Self, cx: &mut Cx| s.session(action, cx)),
                    ),
            )
    }

    fn about(&self, w: f32, h: f32, t: &Theme) -> Div<Self> {
        let info = machine_info();
        let card_w = 300.0;
        let mut rows = div().flex_col().gap(4.0).w_full().mt(6.0);
        for (k, v) in info {
            rows = rows.child(
                div()
                    .flex_row()
                    .gap(8.0)
                    .child(text(k).size(11.0).color(t.secondary).w(80.0))
                    .child(text(v).size(11.0).flex_1()),
            );
        }
        div()
            .absolute((w - card_w) / 2.0, (h - 340.0) / 2.5)
            .w(card_w)
            .flex_col()
            .items_center()
            .gap(6.0)
            .px(24.0)
            .py(22.0)
            .rounded(14.0)
            .bg(t.menu)
            .border(t.border, 0.5)
            .shadow(t.shadow, 30.0, 10.0)
            .occlude()
            .child(app_mark(t, 76.0))
            .child(text("y7ya-os").size(22.0).weight(Weight::Bold).mt(8.0))
            .child(text(format!("Version {}", env!("CARGO_PKG_VERSION"))).size(11.0).color(t.secondary))
            .child(rows)
    }

    /// Hands a request to the system worker.
    pub(super) fn send(&self, r: Request) {
        self.services.send(r);
    }
}

/// The system's mark on a gradient-free accent tile.
pub fn app_mark<S: 'static>(t: &Theme, size: f32) -> Div<S> {
    let ink = Color::WHITE;
    div()
        .size(size)
        .rounded(size * 0.23)
        .bg(if t.dark { rgb(0x3a3a8c) } else { rgb(0x4a4ad0) })
        .items_center()
        .justify_center()
        .child(canvas(move |s, b| icons::logo(s, b, ink)).size(size * 0.62))
}

/// "Fri Nov 14  7:52 PM", like macOS.
pub fn clock_text(now: &chrono::DateTime<chrono::Local>) -> String {
    now.format("%a %b %-d  %-I:%M %p").to_string()
}

fn machine_info() -> Vec<(&'static str, String)> {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let host = read("/etc/hostname").trim().to_string();
    let cpu = read("/proc/cpuinfo")
        .lines()
        .find_map(|l| l.strip_prefix("model name").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim().to_string()))
        .unwrap_or_default();
    let mem_kb: f64 = read("/proc/meminfo")
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0.0);
    let kernel = read("/proc/sys/kernel/osrelease").trim().to_string();
    let mut out = vec![("Computer", host), ("Processor", cpu), ("Memory", format!("{:.0} GB", mem_kb / 1048576.0)), ("Kernel", kernel)];
    out.retain(|(_, v)| !v.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    #[test]
    fn clock_format() {
        let t = chrono::Local.with_ymd_and_hms(2026, 11, 14, 19, 52, 0).unwrap();
        assert_eq!(super::clock_text(&t), "Sat Nov 14  7:52 PM");
        let t = chrono::Local.with_ymd_and_hms(2026, 11, 6, 9, 5, 0).unwrap();
        assert_eq!(super::clock_text(&t), "Fri Nov 6  9:05 AM");
    }
}
