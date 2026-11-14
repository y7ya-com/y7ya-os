//! The session confirmation dialog, and who's signed in.

use ui::*;

use super::theme::Theme;
use super::{Shell, User, icons};
use crate::system::SessionAction;

impl Shell {
    /// "Are you sure you want to shut down?", centered over a dimmed screen.
    pub(super) fn confirm(&self, action: SessionAction, w: f32, h: f32, t: &Theme, reveal: f32) -> Div<Self> {
        let (title, button) = match action {
            SessionAction::Restart => ("Are you sure you want to restart your computer now?", "Restart"),
            SessionAction::ShutDown => ("Are you sure you want to shut down your computer now?", "Shut Down"),
            _ => ("Are you sure you want to log out?", "Log Out"),
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
            if primary { d.bg(t.accent).hover_bg(t.accent.alpha(0.88)) } else { d.bg(t.fill).hover_bg(t.hover.alpha(2.0)) }
        };
        div()
            .absolute((w - card_w) / 2.0, (h - 230.0) / 2.5 + (1.0 - reveal) * 6.0)
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
            .opacity(reveal)
            .child(app_mark(t, 52.0))
            .child(text(title).weight(Weight::Bold).w_full().line_height(1.35))
            .child(text("Open apps will be closed.").size(11.0).color(t.secondary).w_full())
            .child(
                div()
                    .flex_row()
                    .gap(8.0)
                    .w_full()
                    .mt(4.0)
                    .child(button_el("confirm-cancel", "Cancel", false).on_click(|s: &mut Self, cx: &mut Cx| s.close(cx)))
                    .child(button_el("confirm-ok", button, true).on_click(move |s: &mut Self, cx: &mut Cx| s.session(action, cx))),
            )
    }
}

/// The system's mark on an accent tile.
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

/// The user's full name (from the password database) and picture.
pub fn current_user() -> User {
    let login = std::env::var("USER").unwrap_or_default();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let name = full_name(&passwd, &login).unwrap_or_else(|| {
        let mut c = login.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    });
    let home = dirs::home_dir().unwrap_or_default();
    let avatar = [home.join(".face.icon"), home.join(".face"), format!("/var/lib/AccountsService/icons/{login}").into()]
        .into_iter()
        .find(|p| p.is_file())
        .and_then(|p| Image::load(&p, 128));
    User { name, avatar }
}

/// The GECOS name for `login` in `/etc/passwd`'s text.
fn full_name(passwd: &str, login: &str) -> Option<String> {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        let name = f.get(4)?.split(',').next()?.trim();
        (f.first() == Some(&login) && !name.is_empty()).then(|| name.to_string())
    })
}

/// The user's picture, or their initials on a tinted circle.
pub fn avatar<S: 'static>(user: &User, size: f32, t: &Theme) -> Div<S> {
    if let Some(img) = &user.avatar {
        return div().size(size).shrink_0().child(image(img, Fit::Cover, size / 2.0).size(size));
    }
    let initials: String = user.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).flat_map(char::to_uppercase).collect();
    div()
        .size(size)
        .shrink_0()
        .rounded(size / 2.0)
        .bg(if t.dark { rgb(0x636366) } else { rgb(0x8e8e93) })
        .items_center()
        .justify_center()
        .child(text(initials).size(size * 0.4).weight(Weight::Semibold).color(Color::WHITE))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_full_names() {
        let passwd = "root:x:0:0::/root:/bin/bash\ny7ya:x:1000:1000:Yahya Ali,,,:/home/y7ya:/bin/zsh\nbob:x:1001:1001::/home/bob:/bin/sh\n";
        assert_eq!(super::full_name(passwd, "y7ya").as_deref(), Some("Yahya Ali"));
        assert_eq!(super::full_name(passwd, "bob"), None);
        assert_eq!(super::full_name(passwd, "nobody"), None);
    }
}
