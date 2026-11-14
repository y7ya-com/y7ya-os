//! XDG autostart entries (`~/.config/autostart`, `/etc/xdg/autostart`).
//! A file in the user directory overrides a system file of the same name, so
//! disabling a system entry writes a user copy with `Hidden=true`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::desktop::entry_keys;

#[derive(Clone, Debug, PartialEq)]
pub struct StartupEntry {
    /// File name, e.g. `nm-applet.desktop`.
    pub file: String,
    pub name: String,
    pub comment: String,
    pub exec: String,
    pub enabled: bool,
    /// The file that takes effect: the user copy if there is one.
    pub path: PathBuf,
    /// Whether a system-wide file exists under this name.
    pub system: bool,
    pub user: bool,
}

pub struct Autostart {
    pub user_dir: PathBuf,
    pub system_dirs: Vec<PathBuf>,
}

impl Autostart {
    pub fn standard() -> Self {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_default();
        let dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
        Self {
            user_dir: config.join("autostart"),
            system_dirs: dirs.split(':').filter(|s| !s.is_empty()).map(|d| Path::new(d).join("autostart")).collect(),
        }
    }

    pub fn load(&self) -> Vec<StartupEntry> {
        let mut files: BTreeMap<String, (Option<PathBuf>, Option<PathBuf>)> = BTreeMap::new();
        // Earlier system dirs win over later ones.
        for dir in self.system_dirs.iter().rev() {
            for (name, path) in desktop_files(dir) {
                files.entry(name).or_default().1 = Some(path);
            }
        }
        for (name, path) in desktop_files(&self.user_dir) {
            files.entry(name).or_default().0 = Some(path);
        }
        let mut out: Vec<StartupEntry> = files
            .into_iter()
            .filter_map(|(file, (user, system))| {
                let path = user.clone().or(system.clone())?;
                let k = entry_keys(&std::fs::read_to_string(&path).ok()?);
                if k.get("Type").is_some_and(|t| t != "Application") {
                    return None;
                }
                let off = k.get("Hidden").is_some_and(|v| v == "true") || k.get("X-GNOME-Autostart-enabled").is_some_and(|v| v == "false");
                Some(StartupEntry {
                    name: k.get("Name").cloned().unwrap_or_else(|| file.trim_end_matches(".desktop").to_string()),
                    comment: k.get("Comment").cloned().unwrap_or_default(),
                    exec: k.get("Exec").cloned().unwrap_or_default(),
                    enabled: !off,
                    path,
                    system: system.is_some(),
                    user: user.is_some(),
                    file,
                })
            })
            .collect();
        out.sort_by_key(|e| e.name.to_lowercase());
        out
    }

    /// Turns an entry on or off by editing (or creating) its user copy.
    pub fn set_enabled(&self, e: &StartupEntry, enabled: bool) -> std::io::Result<()> {
        let user_path = self.user_dir.join(&e.file);
        let text = std::fs::read_to_string(&e.path)?;
        let mut text = set_key(&text, "Hidden", if enabled { "false" } else { "true" });
        if entry_keys(&text).contains_key("X-GNOME-Autostart-enabled") {
            text = set_key(&text, "X-GNOME-Autostart-enabled", if enabled { "true" } else { "false" });
        }
        std::fs::create_dir_all(&self.user_dir)?;
        let tmp = user_path.with_extension("desktop.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &user_path)
    }
}

fn desktop_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.ends_with(".desktop").then(|| (name, e.path()))
        })
        .collect()
}

/// Sets `key=value` in the `[Desktop Entry]` group, adding it if missing.
pub fn set_key(text: &str, key: &str, value: &str) -> String {
    let mut out = Vec::new();
    let mut in_entry = false;
    let mut done = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_entry && !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            in_entry = t == "[Desktop Entry]";
        } else if in_entry && t.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            if !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        if !in_entry {
            out.push("[Desktop Entry]".into());
        }
        out.push(format!("{key}={value}"));
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tm-autostart-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn set_key_replaces_or_adds_within_the_entry_group() {
        let t = "[Desktop Entry]\nName=A\nHidden=false\n[Desktop Action x]\nHidden=keep\n";
        assert_eq!(set_key(t, "Hidden", "true"), "[Desktop Entry]\nName=A\nHidden=true\n[Desktop Action x]\nHidden=keep\n");
        let t = "[Desktop Entry]\nName=A\n[Desktop Action x]\nName=B\n";
        assert_eq!(set_key(t, "Hidden", "true"), "[Desktop Entry]\nName=A\nHidden=true\n[Desktop Action x]\nName=B\n");
        assert_eq!(set_key("[Desktop Entry]\nName=A", "Hidden", "true"), "[Desktop Entry]\nName=A\nHidden=true\n");
    }

    #[test]
    fn disabling_a_system_entry_writes_a_user_override() {
        let root = tmp("override");
        let (user, system) = (root.join("user"), root.join("system"));
        std::fs::create_dir_all(&system).unwrap();
        let sys_text = "[Desktop Entry]\nType=Application\nName=Applet\nExec=applet\n";
        std::fs::write(system.join("applet.desktop"), sys_text).unwrap();
        std::fs::write(system.join("gnome.desktop"), "[Desktop Entry]\nType=Application\nName=Gnomey\nExec=g\nX-GNOME-Autostart-enabled=false\n").unwrap();
        let a = Autostart { user_dir: user.clone(), system_dirs: vec![system.clone()] };

        let entries = a.load();
        assert_eq!(entries.len(), 2);
        let applet = entries.iter().find(|e| e.file == "applet.desktop").unwrap().clone();
        assert!(applet.enabled && applet.system && !applet.user);
        assert!(!entries.iter().find(|e| e.file == "gnome.desktop").unwrap().enabled);

        a.set_enabled(&applet, false).unwrap();
        let after = a.load();
        let applet = after.iter().find(|e| e.file == "applet.desktop").unwrap().clone();
        assert!(!applet.enabled && applet.user);
        assert_eq!(applet.path, user.join("applet.desktop"));
        // The system file is untouched.
        assert_eq!(std::fs::read_to_string(system.join("applet.desktop")).unwrap(), sys_text);

        a.set_enabled(&applet, true).unwrap();
        assert!(a.load().iter().find(|e| e.file == "applet.desktop").unwrap().enabled);

        // Re-enabling a GNOME-style entry flips its own key too.
        let gnome = a.load().into_iter().find(|e| e.file == "gnome.desktop").unwrap();
        a.set_enabled(&gnome, true).unwrap();
        assert!(a.load().iter().find(|e| e.file == "gnome.desktop").unwrap().enabled);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
