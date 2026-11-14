//! `.desktop` files: parsing their `[Desktop Entry]` group, and an index of
//! installed applications for naming and grouping processes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Key/value pairs of the `[Desktop Entry]` group (unlocalized keys only).
pub fn entry_keys(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut in_entry = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry && !line.starts_with('#') {
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                if !k.contains('[') {
                    out.insert(k.to_string(), v.trim().to_string());
                }
            }
        }
    }
    out
}

/// The program an `Exec=` line runs, as a bare file name: skips `env` and its
/// assignments and strips quotes and directories.
pub fn exec_binary(exec: &str) -> Option<String> {
    let mut words = exec.split_whitespace().map(|w| w.trim_matches('"'));
    let mut w = words.next()?;
    if Path::new(w).file_name().is_some_and(|n| n == "env") {
        w = words.find(|w| !w.contains('=') && !w.starts_with('-'))?;
    }
    let name = Path::new(w).file_name()?.to_string_lossy().to_string();
    (!name.is_empty() && !name.starts_with('%')).then_some(name)
}

#[derive(Clone, Debug)]
pub struct DesktopApp {
    /// File name without `.desktop`, e.g. `org.kde.dolphin`.
    pub id: String,
    pub name: String,
    pub binary: Option<String>,
    pub wm_class: Option<String>,
    /// Hidden from launchers (`NoDisplay`) or a terminal program: never an "app".
    pub hidden: bool,
}

/// Installed applications, looked up by the many names a process or window may go by.
#[derive(Default)]
pub struct AppIndex {
    apps: Vec<DesktopApp>,
    by_name: HashMap<String, usize>,
}

pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| dirs_home().map(|h| h.join(".local/share"))) {
        dirs.push(d);
    }
    let sys = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    dirs.push("/var/lib/flatpak/exports/share".into());
    dirs
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

impl AppIndex {
    pub fn load() -> Self {
        let dirs: Vec<PathBuf> = data_dirs().into_iter().map(|d| d.join("applications")).collect();
        Self::from_dirs(&dirs)
    }

    /// Earlier directories win, as in the XDG spec.
    pub fn from_dirs(dirs: &[PathBuf]) -> Self {
        let mut index = AppIndex::default();
        let mut seen = std::collections::HashSet::new();
        for dir in dirs {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for e in rd.flatten() {
                let path = e.path();
                let Some(id) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".desktop")) else { continue };
                if !seen.insert(id.to_string()) {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&path) {
                    index.add(id, &text);
                }
            }
        }
        index
    }

    pub fn add(&mut self, id: &str, text: &str) {
        let k = entry_keys(text);
        if k.get("Type").is_some_and(|t| t != "Application") {
            return;
        }
        let truthy = |key: &str| k.get(key).is_some_and(|v| v == "true");
        let app = DesktopApp {
            id: id.to_string(),
            name: k.get("Name").cloned().unwrap_or_else(|| id.to_string()),
            binary: k.get("Exec").and_then(|e| exec_binary(e)),
            wm_class: k.get("StartupWMClass").cloned(),
            hidden: truthy("NoDisplay") || truthy("Hidden") || truthy("Terminal"),
        };
        let i = self.apps.len();
        // Most specific first; `or_insert` keeps the first claim on a name.
        let short = id.rsplit('.').next().unwrap_or(id).to_lowercase();
        let mut names = vec![id.to_lowercase()];
        names.extend(app.wm_class.as_ref().map(|c| c.to_lowercase()));
        names.push(short);
        names.extend(app.binary.as_ref().map(|b| b.to_lowercase()));
        for n in names {
            let slot = self.by_name.entry(n).or_insert(i);
            // A visible app beats a hidden helper that happens to share a name.
            if *slot != i && self.apps[*slot].hidden && !app.hidden {
                *slot = i;
            }
        }
        self.apps.push(app);
    }

    pub fn get(&self, i: usize) -> &DesktopApp {
        &self.apps[i]
    }

    /// Finds the app for a window's app_id, a cgroup app id, or a program name.
    pub fn resolve(&self, name: &str) -> Option<usize> {
        let n = name.to_lowercase();
        let n = n.strip_suffix(".desktop").unwrap_or(&n);
        self.by_name.get(n).or_else(|| self.by_name.get(n.rsplit('.').next().unwrap_or(n))).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_exec() {
        let text = "[Desktop Entry]\nName=Dolphin\nName[de]=Delfin\nExec=dolphin %u\n# comment\n[Desktop Action new]\nExec=other\n";
        let k = entry_keys(text);
        assert_eq!(k["Name"], "Dolphin");
        assert_eq!(k["Exec"], "dolphin %u");
        assert_eq!(k.len(), 2);
        assert_eq!(exec_binary("/usr/lib/firefox/firefox %u").as_deref(), Some("firefox"));
        assert_eq!(exec_binary("env GDK_BACKEND=x11 \"/opt/app/bin/app\" --flag").as_deref(), Some("app"));
        assert_eq!(exec_binary("").as_deref(), None);
    }

    #[test]
    fn index_resolves_many_names() {
        let mut ix = AppIndex::default();
        ix.add("org.kde.dolphin", "[Desktop Entry]\nType=Application\nName=Dolphin\nExec=dolphin %u\n");
        ix.add("org.chromium.Chromium", "[Desktop Entry]\nType=Application\nName=Chromium\nExec=/usr/bin/chromium\nStartupWMClass=chromium\n");
        ix.add("htop", "[Desktop Entry]\nType=Application\nName=Htop\nExec=htop\nTerminal=true\n");
        let name = |n: &str| ix.resolve(n).map(|i| ix.get(i).name.clone());
        assert_eq!(name("org.kde.dolphin").as_deref(), Some("Dolphin"));
        assert_eq!(name("dolphin").as_deref(), Some("Dolphin"));
        assert_eq!(name("Chromium").as_deref(), Some("Chromium"));
        assert_eq!(name("chromium").as_deref(), Some("Chromium"));
        assert!(ix.get(ix.resolve("htop").unwrap()).hidden);
        assert_eq!(name("nothing"), None);
    }
}
