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
    /// `NoDisplay=true`: launchable and found by search, but not listed in
    /// All Apps (helpers, and apps like AppImages that hide themselves).
    pub no_display: bool,
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

    /// The program `command` runs, as written (`/opt/x/bin/foo`, `foo`).
    pub fn program(&self) -> String {
        self.command().split_whitespace().next().unwrap_or("").to_string()
    }

    /// Whether this entry is the app behind a window with `app_id`.
    pub fn matches_app_id(&self, app_id: &str) -> bool {
        let short = |s: &str| s.rsplit('.').next().unwrap_or(s).to_lowercase();
        let a = app_id.to_lowercase();
        if self.id.to_lowercase() == a
            || self.wm_class.to_lowercase() == a
            || short(&self.id) == short(app_id)
            || self.name.to_lowercase() == a
        {
            return true;
        }
        // Looser, for entries that don't name their window class (Electron
        // AppImages): "T3 Code (Beta)" or `T3-Code-0.1.7-x86_64.AppImage`
        // for a window called `t3code`. Short ids are too easy to collide.
        let a = squash(app_id);
        a.len() >= 4 && (squash(self.name.split(" (").next().unwrap_or("")) == a || program_stem(&self.program()).is_some_and(|p| p == a))
    }
}

/// Lowercase letters and digits only: "T3 Code" → "t3code".
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A program's name without its path, extension or trailing version
/// (`/x/T3-Code-0.1.7-x86_64.AppImage` → "t3code"), squashed. `None` for
/// launchers that run something else.
fn program_stem(program: &str) -> Option<String> {
    let file = program.rsplit('/').next().unwrap_or(program);
    let file = file.strip_suffix(".AppImage").or_else(|| file.strip_suffix(".appimage")).unwrap_or(file);
    const WRAPPERS: [&str; 10] = ["env", "sh", "bash", "flatpak", "snap", "python", "python3", "java", "xdg-open", "gtk-launch"];
    if WRAPPERS.contains(&file) {
        return None;
    }
    // Cut at a dash before a digit: "T3-Code-0.1.7-x86_64" → "T3-Code".
    let bytes = file.as_bytes();
    let end = (1..bytes.len()).find(|&i| bytes[i - 1] == b'-' && bytes[i].is_ascii_digit()).map_or(file.len(), |i| i - 1);
    Some(squash(&file[..end]))
}

/// Parses the `[Desktop Entry]` group. Returns `None` for entries that aren't
/// launchable apps or that are deleted (`Hidden=true`) or meant for other
/// desktops; `NoDisplay=true` ones come back marked `no_display`.
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
            "Hidden" if value == "true" => return None,
            "NoDisplay" => e.no_display = value == "true",
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
    let mut matching = apps.iter().filter(|e| e.matches_app_id(app_id));
    if let Some(e) = matching.clone().find(|e| !e.no_display).or_else(|| matching.next()) {
        return e.name.clone();
    }
    let short = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut c = short.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The Start menu's "All apps": apps alphabetically under letter headers,
/// with names that don't start with a letter under "#" first. `NoDisplay`
/// apps aren't listed.
pub fn by_letter(apps: &[DesktopEntry]) -> Vec<(String, Vec<&DesktopEntry>)> {
    let header = |e: &DesktopEntry| match e.name.chars().find(|c| !c.is_whitespace()) {
        Some(c) if c.is_alphabetic() => c.to_uppercase().collect::<String>(),
        _ => "#".to_string(),
    };
    let mut sorted: Vec<&DesktopEntry> = apps.iter().filter(|e| !e.no_display).collect();
    sorted.sort_by_cached_key(|e| (header(e) != "#", header(e), e.name.to_lowercase()));
    let mut out: Vec<(String, Vec<&DesktopEntry>)> = Vec::new();
    for e in sorted {
        let h = header(e);
        match out.last_mut() {
            Some((last, list)) if *last == h => list.push(e),
            _ => out.push((h, vec![e])),
        }
    }
    out
}

/// The entry with desktop-file ID `id`.
pub fn by_id<'a>(apps: &'a [DesktopEntry], id: &str) -> Option<&'a DesktopEntry> {
    apps.iter().find(|e| e.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_letter() {
        let e = |name: &str| DesktopEntry { id: name.to_lowercase(), name: name.into(), exec: "x".into(), ..Default::default() };
        let apps = [e("notes"), e("Calendar"), e("7-Zip"), e("Calculator"), e("Écran"), e("Files"), e("[x] tool")];
        let groups: Vec<(String, Vec<&str>)> =
            by_letter(&apps).into_iter().map(|(h, l)| (h, l.into_iter().map(|e| e.name.as_str()).collect())).collect();
        let want: Vec<(String, Vec<&str>)> = vec![
            ("#".into(), vec!["7-Zip", "[x] tool"]),
            ("C".into(), vec!["Calculator", "Calendar"]),
            ("F".into(), vec!["Files"]),
            ("N".into(), vec!["notes"]),
            ("É".into(), vec!["Écran"]),
        ];
        assert_eq!(groups, want);
        assert_eq!(by_id(&apps, "files").map(|e| e.name.as_str()), Some("Files"));
        assert!(by_letter(&[]).is_empty());
    }

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

        // NoDisplay apps still parse (search finds them); Hidden ones are gone.
        let no_display = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n";
        assert!(parse("x", no_display).unwrap().no_display);
        assert!(!e.no_display);
        let hidden = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nHidden=true\n";
        assert_eq!(parse("x", hidden), None);
        let link = "[Desktop Entry]\nType=Link\nName=X\nURL=https://x\n";
        assert_eq!(parse("x", link), None);
        let gnome_only = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nOnlyShowIn=GNOME;\n";
        assert_eq!(parse("x", gnome_only), None);
        let e = parse("y", "[Desktop Entry]\nType=Application\nName=Y\nExec=env A=1 y --flag=%%x %f\n").unwrap();
        assert_eq!(e.command(), "env A=1 y --flag=%x");
        assert_eq!(e.program(), "env");
    }

    /// T3 Code's own entry: hidden from menus, no StartupWMClass, an
    /// AppImage with a version in its name. Its Electron window is `t3code`.
    const T3: &str = "[Desktop Entry]\nType=Application\nName=T3 Code (Beta)\n\
        Exec=/home/u/Downloads/T3-Code-0.1.7-x86_64.AppImage %U\nIcon=/home/u/.local/share/icons/t3.png\nNoDisplay=true\n";

    #[test]
    fn matches_unlabeled_windows() {
        let t3 = parse("com.t3tools.T3Code", T3).unwrap();
        assert!(t3.no_display);
        assert_eq!(t3.program(), "/home/u/Downloads/T3-Code-0.1.7-x86_64.AppImage");
        assert!(t3.matches_app_id("t3code"));
        assert!(t3.matches_app_id("T3Code"));
        // By the AppImage's name alone, and by the name alone.
        let by_exec = DesktopEntry { id: "x".into(), name: "Something".into(), ..t3.clone() };
        assert!(by_exec.matches_app_id("t3-code"));
        let by_name = parse("y", "[Desktop Entry]\nType=Application\nName=Granola Notes (Beta)\nExec=/opt/g/run\n").unwrap();
        assert!(by_name.matches_app_id("granolanotes"));
        assert!(!by_name.matches_app_id("run"));
        // Nothing loose for short ids, wrappers or other apps.
        assert!(!t3.matches_app_id("t3"));
        assert!(!t3.matches_app_id("code"));
        let wrapped = parse("z", "[Desktop Entry]\nType=Application\nName=Zed\nExec=env A=1 zed\n").unwrap();
        assert!(!wrapped.matches_app_id("env"));
        assert_eq!(program_stem("/usr/bin/org.kde.dolphin"), Some("orgkdedolphin".into()));
        assert_eq!(program_stem("flatpak"), None);
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
        std::fs::write(user.join("helper.desktop"), "[Desktop Entry]\nType=Application\nName=Helper\nExec=h\nNoDisplay=true\n").unwrap();
        std::fs::write(system.join("gone.desktop"), entry("Gone")).unwrap();

        let apps = load(&[user, system]);
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Helper", "My Notes", "Tool", "Zed"]);
        assert_eq!(apps[2].id, "sub-tool");
        // All Apps leaves the NoDisplay one out.
        let listed: Vec<&str> = by_letter(&apps).into_iter().flat_map(|(_, l)| l).map(|e| e.name.as_str()).collect();
        assert_eq!(listed, ["My Notes", "Tool", "Zed"]);
        assert_eq!(display_name(&apps, "notes"), "My Notes");
        assert_eq!(display_name(&apps, "org.kde.dolphin"), "Dolphin");
        std::fs::remove_dir_all(base).unwrap();
    }
}
