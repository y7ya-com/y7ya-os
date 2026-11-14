//! The Mac look for third-party apps (Qt/KDE, GTK, libadwaita, cursors).
//!
//! `desktop/session/theme.sh` installs it in `~/.local/share/y7ya-os/look`
//! (`Y7YA_LOOK_DIR` overrides it); [`env`] gives the environment that applies
//! it to an app y7ya-os starts. Only those apps see it, so the user's other
//! desktops (Plasma) keep their own settings:
//!
//! - `XDG_CONFIG_DIRS` gets a `kdeglobals` that locks fonts, colors, icons and
//!   the widget style over the user's own (KDE's kiosk `[$i]`), the Kvantum
//!   theme and Konsole's default profile.
//! - `XDG_DATA_DIRS` gets the WhiteSur GTK/icon themes and an icon theme
//!   standing in for the user's GTK icon theme name.
//! - `GTK_THEME` picks the GTK theme (light or dark) for GTK 3, GTK 4 and
//!   libadwaita, `XCURSOR_*` the cursors.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

const KDE_PLATFORM_THEME: &str = "/usr/lib/qt6/plugins/platformthemes/KDEPlasmaPlatformTheme6.so";

/// The installed look, if `theme.sh` finished installing it.
pub fn dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("Y7YA_LOOK_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::data_dir()?.join("y7ya-os/look"),
    };
    dir.join("ready").exists().then_some(dir)
}

/// Environment for an app started in the given appearance.
pub fn env(dark: bool) -> Vec<(&'static str, OsString)> {
    let mut env = Vec::new();
    // Qt apps only load KDE's platform theme (fonts, colors, icons, dialogs)
    // when told to, since the desktop isn't KDE.
    if Path::new(KDE_PLATFORM_THEME).exists() {
        env.push(("QT_QPA_PLATFORMTHEME", "kde".into()));
    }
    let Some(look) = dir() else { return env };
    let mode = look.join(if dark { "dark" } else { "light" });
    env.push(("XDG_CONFIG_DIRS", prepend("XDG_CONFIG_DIRS", &look, &[mode.join("config")], "/etc/xdg")));
    let data = [mode.join("share"), look.join("share")];
    env.push(("XDG_DATA_DIRS", prepend("XDG_DATA_DIRS", &look, &data, "/usr/local/share:/usr/share")));
    env.push(("GTK_THEME", if dark { "y7ya-os-dark" } else { "y7ya-os-light" }.into()));
    // libadwaita would follow the user's other desktop (portal) instead, and
    // in its dark mode WhiteSur's stylesheet loses its backgrounds; the GTK
    // theme already carries the dark colors.
    env.push(("ADW_DEBUG_COLOR_SCHEME", "prefer-light".into()));
    env.extend(cursor_env(&look));
    env
}

/// The cursor theme, for apps and for the desktop's own pointer.
pub fn cursor_env(look: &Path) -> Vec<(&'static str, OsString)> {
    let size = std::env::var_os("XCURSOR_SIZE").unwrap_or_else(|| "24".into());
    // Only our directory, so whatever theme name a toolkit asks for gets ours.
    vec![("XCURSOR_THEME", "WhiteSur-cursors".into()), ("XCURSOR_PATH", look.join("cursors").into()), ("XCURSOR_SIZE", size)]
}

/// `ours` in front of the search path in `var` (or its spec default), minus
/// any look directories already there from the other appearance.
fn prepend(var: &str, look: &Path, ours: &[PathBuf], default: &str) -> OsString {
    let current = std::env::var_os(var).filter(|v| !v.is_empty()).unwrap_or_else(|| default.into());
    let rest = std::env::split_paths(&current).filter(|p| !p.starts_with(look));
    std::env::join_paths(ours.iter().cloned().chain(rest)).unwrap_or(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepend_replaces_old_look_dirs() {
        let look = Path::new("/home/u/.local/share/y7ya-os/look");
        // Safety: this test is the only one touching this variable.
        unsafe { std::env::set_var("Y7_TEST_DIRS", "/home/u/.local/share/y7ya-os/look/dark/share:/usr/share") };
        let got = prepend("Y7_TEST_DIRS", look, &[look.join("light/share")], "/x");
        assert_eq!(got, "/home/u/.local/share/y7ya-os/look/light/share:/usr/share");
        assert_eq!(prepend("Y7_TEST_UNSET", look, &[look.join("a")], "/etc/xdg"), "/home/u/.local/share/y7ya-os/look/a:/etc/xdg");
    }
}
