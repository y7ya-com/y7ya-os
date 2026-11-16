//! Starts the user's login apps the way other desktops do (the XDG autostart
//! spec): `.desktop` files in `~/.config/autostart`, then `/etc/xdg/autostart`,
//! with the user's copy of a file overriding the system one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::state::Desk;

/// Entries the desktop already does itself (the Control Center has Wi-Fi and
/// printers are in System Settings), or starts on its own.
const COVERED: &[&str] = &["nm-applet", "system-config-printer-applet", "y7-syncd", "y7-shell", "clipboard"];

/// KDE's helpers that are marked KDE-only but that this desktop wants too.
const KDE_HELPERS: &[&str] = &["polkit-kde-authentication-agent-1.desktop"];

pub fn start(desk: &Desk) {
    for argv in commands(&dirs()) {
        desk.spawn_argv(&argv);
    }
}

fn dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let system = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
    std::iter::once(config).chain(system.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)).map(|d| d.join("autostart")).collect()
}

/// The command lines to run, from autostart directories in priority order.
fn commands(dirs: &[PathBuf]) -> Vec<Vec<String>> {
    // By file name, so the first directory's copy wins.
    let mut entries: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(dir) else { continue };
        for e in read.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".desktop") {
                entries.entry(name).or_insert(e.path());
            }
        }
    }
    entries
        .iter()
        .filter_map(|(name, path)| {
            let text = std::fs::read_to_string(path).ok()?;
            wanted(name, &text)
        })
        .collect()
}

/// The command for one entry, if it should start in this desktop.
fn wanted(name: &str, text: &str) -> Option<Vec<String>> {
    let mut keys = BTreeMap::new();
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if let (true, Some((k, v))) = (in_entry, line.split_once('=')) {
            keys.entry(k.trim()).or_insert(v.trim());
        }
    }
    let get = |k: &str| keys.get(k).copied();
    let listed = |k: &str| get(k).is_some_and(|v| v.split(';').any(|d| d.eq_ignore_ascii_case("y7ya-os")));
    if get("Hidden") == Some("true") || get("X-GNOME-Autostart-enabled") == Some("false") || listed("NotShowIn") {
        return None;
    }
    if get("OnlyShowIn").is_some() && !listed("OnlyShowIn") && !KDE_HELPERS.contains(&name) {
        return None;
    }
    if get("TryExec").is_some_and(|t| !installed(t)) {
        return None;
    }
    let argv = split_exec(get("Exec")?);
    let program = Path::new(argv.first()?).file_name()?.to_string_lossy().into_owned();
    // `env FOO=1 program …` names the program later on.
    let real = argv.iter().skip_while(|a| *a == "env" || a.contains('=')).next().map(|a| a.rsplit('/').next().unwrap_or(a));
    if COVERED.contains(&program.as_str()) || real.is_some_and(|r| COVERED.contains(&r)) {
        return None;
    }
    Some(argv)
}

fn installed(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).exists();
    }
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).exists()))
}

/// Splits an `Exec=` value into arguments: double quotes group, a backslash
/// escapes, and field codes (`%f`, `%U`, …) are dropped since nothing is opened.
fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' => cur.extend(chars.next()),
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        args.push(cur);
    }
    args.retain(|a| !(a.len() == 2 && a.starts_with('%')));
    args.iter_mut().for_each(|a| *a = a.replace("%%", "%"));
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(body: &str) -> String {
        format!("[Desktop Entry]\nType=Application\n{body}\n[Desktop Action x]\nExec=nope\n")
    }

    #[test]
    fn follows_the_spec() {
        assert_eq!(wanted("a.desktop", &entry("Exec=foo --bar %U")), Some(vec!["foo".into(), "--bar".into()]));
        assert_eq!(wanted("a.desktop", &entry("Exec=foo\nHidden=true")), None);
        assert_eq!(wanted("a.desktop", &entry("Exec=foo\nOnlyShowIn=KDE;")), None);
        assert!(wanted("a.desktop", &entry("Exec=foo\nOnlyShowIn=KDE;y7ya-os;")).is_some());
        assert_eq!(wanted("a.desktop", &entry("Exec=foo\nNotShowIn=y7ya-os")), None);
        assert_eq!(wanted("a.desktop", &entry("Exec=foo\nX-GNOME-Autostart-enabled=false")), None);
        assert_eq!(wanted("a.desktop", &entry("Exec=foo\nTryExec=/no/such/program")), None);
        assert!(wanted("polkit-kde-authentication-agent-1.desktop", &entry("Exec=/usr/lib/polkit\nOnlyShowIn=KDE;")).is_some());
        // Things the desktop covers itself.
        assert_eq!(wanted("nm-applet.desktop", &entry("Exec=nm-applet")), None);
        assert_eq!(wanted("s.desktop", &entry("Exec=/home/me/.local/bin/y7-syncd")), None);
        assert_eq!(
            wanted("e.desktop", &entry("Exec=env A=b easyeffects --service-mode")),
            Some(vec!["env".into(), "A=b".into(), "easyeffects".into(), "--service-mode".into()])
        );
    }

    #[test]
    fn splits_exec_lines() {
        assert_eq!(split_exec(r#""/opt/My App/run" --x "" a\ b 100%%"#), vec!["/opt/My App/run", "--x", "", "a b", "100%"]);
    }

    #[test]
    fn user_entries_override_system_ones() {
        let root = std::env::temp_dir().join(format!("y7-autostart-{}", std::process::id()));
        let (user, system) = (root.join("user"), root.join("system"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(system.join("a.desktop"), entry("Exec=system-a")).unwrap();
        std::fs::write(user.join("a.desktop"), entry("Exec=user-a")).unwrap();
        std::fs::write(system.join("b.desktop"), entry("Exec=system-b")).unwrap();
        let got = commands(&[user, system]);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(got, vec![vec!["user-a".to_string()], vec!["system-b".to_string()]]);
    }
}
