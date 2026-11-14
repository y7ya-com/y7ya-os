//! Installed applications, from `.desktop` files in the XDG data dirs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesktopEntry {
    /// The desktop-file ID: `org.kde.konsole` for `org.kde.konsole.desktop`.
    pub id: String,
    pub name: String,
    pub generic_name: String,
    pub comment: String,
    pub keywords: Vec<String>,
    pub exec: String,
    pub icon: String,
    pub wm_class: String,
}

impl DesktopEntry {
    /// The command line with field codes (%f, %U...) removed.
    pub fn command(&self) -> String {
        let mut out = String::new();
        let mut chars = self.exec.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('%') => out.push('%'),
                    _ => {}
                }
            } else {
                out.push(c);
            }
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Whether this entry is the app behind a window with `app_id`.
    pub fn matches_app_id(&self, app_id: &str) -> bool {
        let short = |s: &str| s.rsplit('.').next().unwrap_or(s).to_lowercase();
        let a = app_id.to_lowercase();
        self.id.to_lowercase() == a
            || self.wm_class.to_lowercase() == a
            || short(&self.id) == short(app_id)
            || self.name.to_lowercase() == a
    }
}

/// Parses the `[Desktop Entry]` group. Returns `None` for entries that aren't
/// launchable apps or that ask to be hidden.
pub fn parse(id: &str, text: &str) -> Option<DesktopEntry> {
    let mut e = DesktopEntry { id: id.to_string(), ..Default::default() };
    let mut in_group = false;
    let mut kind = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Type" => kind = value.to_string(),
            "Name" => e.name = unescape(value),
            "GenericName" => e.generic_name = unescape(value),
            "Comment" => e.comment = unescape(value),
            "Keywords" => e.keywords = value.split(';').filter(|k| !k.is_empty()).map(str::to_string).collect(),
            "Exec" => e.exec = value.to_string(),
            "Icon" => e.icon = value.to_string(),
            "StartupWMClass" => e.wm_class = value.to_string(),
            "NoDisplay" | "Hidden" if value == "true" => return None,
            "OnlyShowIn" => {
                let ours = ["KDE", "y7ya-os"];
                if !value.split(';').any(|d| ours.contains(&d)) {
                    return None;
                }
            }
            "NotShowIn" if value.split(';').any(|d| d == "y7ya-os") => return None,
            _ => {}
        }
    }
    (kind == "Application" && !e.name.is_empty() && !e.exec.is_empty()).then_some(e)
}

fn unescape(s: &str) -> String {
    s.replace("\\s", " ").replace("\\n", " ").replace("\\t", " ").replace("\\\\", "\\")
}

/// `$XDG_DATA_HOME/applications` then each `$XDG_DATA_DIRS/applications`.
pub fn dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")));
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(system.split(':').filter(|s| !s.is_empty()).map(PathBuf::from))
        .map(|d| d.join("applications"))
        .collect()
}

/// All apps, sorted by name. Earlier dirs win when two have the same ID.
pub fn load(dirs: &[PathBuf]) -> Vec<DesktopEntry> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for dir in dirs {
        scan(dir, dir, &mut seen, &mut out);
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    out
}

fn scan(root: &Path, dir: &Path, seen: &mut HashSet<String>, out: &mut Vec<DesktopEntry>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            scan(root, &path, seen, out);
            continue;
        }
        if path.extension().is_none_or(|e| e != "desktop") {
            continue;
        }
        // IDs of files in subdirectories join the parts with '-'.
        let rel = path.strip_prefix(root).unwrap_or(&path).with_extension("");
        let id = rel.to_string_lossy().replace('/', "-");
        // A hidden or invalid entry still shadows the same ID further down.
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(e) = std::fs::read_to_string(&path).ok().and_then(|t| parse(&id, &t)) {
            out.push(e);
        }
    }
}

/// A display name for a window's app id: the matching app's name, or a
/// tidied-up id ("org.kde.dolphin" → "Dolphin").
pub fn display_name(apps: &[DesktopEntry], app_id: &str) -> String {
    if let Some(e) = apps.iter().find(|e| e.matches_app_id(app_id)) {
        return e.name.clone();
    }
    let short = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut c = short.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KONSOLE: &str = "[Desktop Entry]\nType=Application\nName=Konsole\nName[de]=Konsole DE\nGenericName=Terminal\n\
        Exec=konsole %U\nIcon=utilities-terminal\nKeywords=terminal;shell;console;\nStartupWMClass=konsole\n\
        [Desktop Action NewWindow]\nName=Open a New Window\nExec=konsole --new\n";

    #[test]
    fn parses_entries() {
        let e = parse("org.kde.konsole", KONSOLE).unwrap();
        assert_eq!(e.name, "Konsole");
        assert_eq!(e.generic_name, "Terminal");
        assert_eq!(e.exec, "konsole %U");
        assert_eq!(e.command(), "konsole");
        assert_eq!(e.keywords, ["terminal", "shell", "console"]);
        assert!(e.matches_app_id("org.kde.konsole"));
        assert!(e.matches_app_id("konsole"));
        assert!(!e.matches_app_id("dolphin"));

        let hidden = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n";
        assert_eq!(parse("x", hidden), None);
        let link = "[Desktop Entry]\nType=Link\nName=X\nURL=https://x\n";
        assert_eq!(parse("x", link), None);
        let gnome_only = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nOnlyShowIn=GNOME;\n";
        assert_eq!(parse("x", gnome_only), None);
        let e = parse("y", "[Desktop Entry]\nType=Application\nName=Y\nExec=env A=1 y --flag=%%x %f\n").unwrap();
        assert_eq!(e.command(), "env A=1 y --flag=%x");
    }

    #[test]
    fn loads_dirs_with_precedence() {
        let base = std::env::temp_dir().join(format!("y7-apps-test-{}", std::process::id()));
        let (user, system) = (base.join("user"), base.join("system"));
        std::fs::create_dir_all(user.join("sub")).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        let entry = |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec={name}\n");
        std::fs::write(user.join("notes.desktop"), entry("My Notes")).unwrap();
        std::fs::write(system.join("notes.desktop"), entry("Notes")).unwrap();
        std::fs::write(system.join("zed.desktop"), entry("Zed")).unwrap();
        std::fs::write(user.join("sub/tool.desktop"), entry("Tool")).unwrap();
        std::fs::write(user.join("gone.desktop"), "[Desktop Entry]\nType=Application\nName=G\nExec=g\nHidden=true\n").unwrap();
        std::fs::write(system.join("gone.desktop"), entry("Gone")).unwrap();

        let apps = load(&[user, system]);
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["My Notes", "Tool", "Zed"]);
        assert_eq!(apps[1].id, "sub-tool");
        assert_eq!(display_name(&apps, "notes"), "My Notes");
        assert_eq!(display_name(&apps, "org.kde.dolphin"), "Dolphin");
        std::fs::remove_dir_all(base).unwrap();
    }
}
