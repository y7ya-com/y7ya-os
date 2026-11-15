//! The taskbar's model, apart from drawing: which buttons it shows in what
//! order, how wide they are, the pinned apps, and Show Desktop.
//!
//! Like KDE's icons-only task manager: one icon per app. Pinned apps keep
//! their place whether running or not; other running apps follow in the
//! order their first window opened.

use std::path::{Path, PathBuf};

use crate::apps::DesktopEntry;
use crate::toplevel::Window;

/// Pinned apps by desktop-file ID, kept one per line in a file under
/// `~/.local/share/y7-shell/` (`taskbar-pins`, `start-pins`).
#[derive(Clone, Debug, PartialEq)]
pub struct Pins {
    pub ids: Vec<String>,
    path: Option<PathBuf>,
}

impl Pins {
    pub const TASKBAR: [&str; 4] = ["files", "notes", "calendar", "calculator"];
    pub const START: [&str; 12] = [
        "files", "notes", "calendar", "reminders", "calculator", "textedit", "preview", "taskmanager", "assistant",
        "org.kde.konsole", "firefox", "org.kde.systemsettings",
    ];

    /// The saved pins; `defaults` until the user changes them.
    pub fn load<S: AsRef<str>>(path: Option<PathBuf>, defaults: &[S]) -> Pins {
        let ids = match path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(text) => text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from)
                .collect(),
            None => defaults.iter().map(|s| s.as_ref().to_string()).collect(),
        };
        Pins { ids, path }
    }

    /// Whether there are saved pins (rather than defaults).
    pub fn saved(path: Option<&Path>) -> bool {
        path.is_some_and(Path::exists)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.iter().any(|p| p == id)
    }

    /// Pins or unpins `id` (new pins go last) and saves.
    pub fn toggle(&mut self, id: &str) {
        if self.contains(id) {
            self.ids.retain(|p| p != id);
        } else {
            self.ids.push(id.to_string());
        }
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut text = self.ids.join("\n");
        text.push('\n');
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("y7-shell: couldn't save pins to {}: {e}", path.display());
        }
    }
}

// -- pins from KDE ---------------------------------------------------------------

/// Plasma's panel layout (read, never written).
pub fn plasma_appletsrc() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".config")))?;
    Some(config.join("plasma-org.kde.plasma.desktop-appletsrc"))
}

/// The launchers pinned to a Plasma task manager (icons-only or not), from
/// `plasma-org.kde.plasma.desktop-appletsrc`: `applications:foo.desktop`,
/// `file:///path`, ... The icons-only one wins if there are several.
pub fn kde_launchers(appletsrc: &str) -> Vec<String> {
    let mut group = "";
    // (applet group, plugin) and (configuration group, launchers).
    let mut plugins: Vec<(&str, &str)> = Vec::new();
    let mut launchers: Vec<(&str, &str)> = Vec::new();
    for line in appletsrc.lines().map(str::trim) {
        if line.starts_with('[') {
            group = line;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "plugin" => plugins.push((group, value.trim())),
            "launchers" => launchers.push((group, value.trim())),
            _ => {}
        }
    }
    let plugin_of = |config: &str| {
        let applet = config.strip_suffix("[Configuration][General]")?;
        plugins.iter().find(|(g, _)| *g == applet).map(|(_, p)| *p)
    };
    let pick = |want: &str| launchers.iter().find(|(g, l)| !l.is_empty() && plugin_of(g) == Some(want)).map(|(_, l)| *l);
    let Some(list) = pick("org.kde.plasma.icontasks").or_else(|| pick("org.kde.plasma.taskmanager")) else { return Vec::new() };
    list.split(',').map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// Desktop-file IDs for KDE launcher URLs, skipping any we can't place.
/// `applications:foo.desktop` is `foo`; `file:///x/bin/foo` is the app that
/// runs `/x/bin/foo`, else the one with ID `foo`, else one running `foo`.
pub fn resolve_launchers(launchers: &[String], apps: &[DesktopEntry]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in launchers {
        let found = if let Some(file) = l.strip_prefix("applications:") {
            let id = file.strip_suffix(".desktop").unwrap_or(file);
            apps.iter().find(|e| e.id == id)
        } else if let Some(path) = l.strip_prefix("file://") {
            let path = percent_decode(path);
            let name = path.rsplit('/').next().unwrap_or(&path);
            match name.strip_suffix(".desktop") {
                Some(id) => apps.iter().find(|e| e.id == id),
                None => apps
                    .iter()
                    .find(|e| e.program() == path)
                    .or_else(|| apps.iter().find(|e| e.id == name))
                    .or_else(|| apps.iter().find(|e| e.program() == name)),
            }
        } else {
            None
        };
        if let Some(e) = found.filter(|e| !out.contains(&e.id)) {
            out.push(e.id.clone());
        }
    }
    out
}

/// The taskbar's first pins: what's pinned in Plasma's panel, if anything.
pub fn kde_pins(apps: &[DesktopEntry]) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(plasma_appletsrc()?).ok()?;
    let pins = resolve_launchers(&kde_launchers(&text), apps);
    (!pins.is_empty()).then_some(pins)
}

/// `%20` and friends in a file URL.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 2 < b.len())
            .then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        match hex {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// -- buttons -----------------------------------------------------------------------

/// One taskbar button: an app, pinned or running or both.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Its desktop-file ID, if an app claims it.
    pub app: Option<String>,
    /// The first window's app id (names and icons for unclaimed windows).
    pub app_id: String,
    pub pinned: bool,
    /// Its windows, in the order they opened.
    pub windows: Vec<u64>,
}

impl Item {
    pub fn app(&self) -> Option<&str> {
        self.app.as_deref()
    }

    /// Stable across window changes, for element ids and tooltips.
    pub fn key(&self) -> String {
        match (&self.app, self.windows.first()) {
            (Some(a), _) => format!("app:{a}"),
            (None, _) if !self.app_id.is_empty() => format!("id:{}", self.app_id),
            (None, Some(w)) => format!("win:{w}"),
            (None, None) => String::new(),
        }
    }
}

/// The desktop-file ID behind a window, if any app claims it.
pub fn app_for(apps: &[DesktopEntry], app_id: &str) -> Option<String> {
    if app_id.is_empty() {
        return None;
    }
    // An exact ID match beats the looser ones, and listed apps beat hidden
    // helpers.
    apps.iter()
        .find(|e| e.id == app_id)
        .or_else(|| apps.iter().find(|e| !e.no_display && e.matches_app_id(app_id)))
        .or_else(|| apps.iter().find(|e| e.matches_app_id(app_id)))
        .map(|e| e.id.clone())
}

/// The buttons, in order. `windows` are in the order they opened.
pub fn items(pins: &[String], windows: &[Window], app_of: impl Fn(&Window) -> Option<String>) -> Vec<Item> {
    let mut out: Vec<Item> =
        pins.iter().map(|p| Item { app: Some(p.clone()), app_id: String::new(), pinned: true, windows: Vec::new() }).collect();
    for w in windows {
        let app = app_of(w);
        // Claimed windows group by app, others by app id; a window with
        // neither stands alone.
        let mine = |i: &Item| match &app {
            Some(a) => i.app.as_ref() == Some(a),
            None => i.app.is_none() && !w.app_id.is_empty() && i.app_id == w.app_id,
        };
        match out.iter_mut().find(|i| mine(i)) {
            Some(item) => {
                if item.windows.is_empty() {
                    item.app_id = w.app_id.clone();
                }
                item.windows.push(w.id);
            }
            None => out.push(Item { app, app_id: w.app_id.clone(), pinned: false, windows: vec![w.id] }),
        }
    }
    out
}

/// What clicking an app's button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// Nothing open: start the app.
    Launch,
    /// One window: bring it up, or minimize it if it's the active one.
    Toggle(u64),
    /// Several: bring up this one (the next after the active one, or the
    /// one used last).
    Activate(u64),
}

/// `recent` lists windows from least to most recently active.
pub fn click(windows: &[u64], active: Option<u64>, recent: &[u64]) -> Click {
    match windows {
        [] => Click::Launch,
        [one] => Click::Toggle(*one),
        _ => match active.and_then(|a| windows.iter().position(|w| *w == a)) {
            Some(i) => Click::Activate(windows[(i + 1) % windows.len()]),
            None => Click::Activate(recent.iter().rev().copied().find(|r| windows.contains(r)).unwrap_or(windows[0])),
        },
    }
}

/// An icon button (apps and Start).
pub const ICON_W: f32 = 40.0;
/// App buttons shrink to fit down to this.
pub const MIN_W: f32 = 28.0;
pub const GAP: f32 = 4.0;

/// Each of `n` app buttons' x and width, laid out from `x` in `width`.
pub fn layout(n: usize, x: f32, width: f32) -> Vec<(f32, f32)> {
    let gaps = n.saturating_sub(1) as f32 * GAP;
    // Whole points, so icons land on the pixel grid.
    let w = if n == 0 { ICON_W } else { ((width - gaps) / n as f32).floor().clamp(MIN_W, ICON_W) };
    (0..n).map(|i| (x + i as f32 * (w + GAP), w)).collect()
}

/// What Show Desktop does next: minimize what's showing, or bring back what it
/// hid last time if nothing has been shown since.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShowDesktop {
    /// The windows the last press minimized.
    hidden: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesktopAction {
    Minimize(Vec<u64>),
    Restore(Vec<u64>),
}

impl ShowDesktop {
    pub fn toggle(&mut self, windows: &[Window]) -> DesktopAction {
        let visible: Vec<u64> = windows.iter().filter(|w| !w.minimized).map(|w| w.id).collect();
        if visible.is_empty() {
            // Restore in the order they were stacked, so the top one ends up on top.
            let restore = std::mem::take(&mut self.hidden).into_iter().filter(|id| windows.iter().any(|w| w.id == *id)).collect();
            DesktopAction::Restore(restore)
        } else {
            // The active window goes last so it comes back on top.
            let mut order = visible.clone();
            if let Some(active) = windows.iter().find(|w| w.activated && !w.minimized) {
                order.retain(|id| *id != active.id);
                order.push(active.id);
            }
            self.hidden = order;
            DesktopAction::Minimize(visible)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: u64, app_id: &str) -> Window {
        Window { id, app_id: app_id.into(), title: format!("{app_id} {id}"), ..Default::default() }
    }

    fn ids(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|i| {
                let name = i.app.clone().unwrap_or_else(|| format!("?{}", i.app_id));
                let windows: Vec<String> = i.windows.iter().map(u64::to_string).collect();
                format!("{}{name}[{}]", if i.pinned { "pin:" } else { "" }, windows.join(","))
            })
            .collect()
    }

    #[test]
    fn one_button_per_app() {
        let pins: Vec<String> = ["files", "notes", "calendar"].map(String::from).to_vec();
        let mut windows = vec![win(1, "konsole"), win(2, "notes"), win(3, "firefox"), win(4, "konsole"), win(5, "notes"), win(6, "")];
        windows.push(win(7, "gimp"));
        windows.push(win(8, "gimp"));
        windows.push(win(9, ""));
        // Pretend nothing claims gimp.
        let app_of = |w: &Window| (!w.app_id.is_empty() && w.app_id != "gimp").then(|| w.app_id.clone());
        let items = items(&pins, &windows, app_of);
        assert_eq!(
            ids(&items),
            ["pin:files[]", "pin:notes[2,5]", "pin:calendar[]", "konsole[1,4]", "firefox[3]", "?[6]", "?gimp[7,8]", "?[9]"]
        );
        // Keys stay put as windows come and go.
        assert_eq!(items[1].key(), "app:notes");
        assert_eq!(items[6].key(), "id:gimp");
        assert_eq!(items[5].key(), "win:6");
        assert!(super::items(&[], &[], app_of).is_empty());
    }

    #[test]
    fn clicks_cycle_through_a_group() {
        assert_eq!(click(&[], None, &[]), Click::Launch);
        assert_eq!(click(&[4], Some(4), &[]), Click::Toggle(4));
        // The active one's group: the next window, wrapping around.
        assert_eq!(click(&[1, 2, 3], Some(1), &[]), Click::Activate(2));
        assert_eq!(click(&[1, 2, 3], Some(3), &[]), Click::Activate(1));
        // Another app is active: the one of this group used last.
        assert_eq!(click(&[1, 2, 3], Some(9), &[2, 9, 3, 1, 9]), Click::Activate(1));
        assert_eq!(click(&[1, 2, 3], None, &[]), Click::Activate(1));
    }

    #[test]
    fn matches_windows_to_apps() {
        let entry = |id: &str, name: &str, wm: &str| DesktopEntry {
            id: id.into(),
            name: name.into(),
            wm_class: wm.into(),
            exec: id.into(),
            ..Default::default()
        };
        let mut helper = entry("konsole-helper", "Konsole", "");
        helper.no_display = true;
        let apps = [helper, entry("org.kde.konsole", "Konsole", "konsole"), entry("notes", "Notes", "notes")];
        assert_eq!(app_for(&apps, "konsole").as_deref(), Some("org.kde.konsole"));
        assert_eq!(app_for(&apps, "org.kde.konsole").as_deref(), Some("org.kde.konsole"));
        assert_eq!(app_for(&apps, "notes").as_deref(), Some("notes"));
        assert_eq!(app_for(&apps, "gimp"), None);
        assert_eq!(app_for(&apps, ""), None);
        // A hidden app is still the app behind its own windows.
        let t3 = DesktopEntry {
            id: "com.t3tools.T3Code".into(),
            name: "T3 Code (Beta)".into(),
            exec: "/home/u/Downloads/T3-Code-0.1.7-x86_64.AppImage %U".into(),
            no_display: true,
            ..Default::default()
        };
        assert_eq!(app_for(&[t3], "t3code").as_deref(), Some("com.t3tools.T3Code"));
    }

    #[test]
    fn buttons_shrink_to_fit() {
        // Plenty of room: full-size squares.
        let l = layout(3, 100.0, 1000.0);
        assert_eq!(l, [(100.0, ICON_W), (100.0 + ICON_W + GAP, ICON_W), (100.0 + 2.0 * (ICON_W + GAP), ICON_W)]);
        // Tight: they share what's left, in whole points.
        let l = layout(7, 0.0, 300.0);
        assert_eq!(l[0].1, ((300.0 - 6.0 * GAP) / 7.0f32).floor());
        assert!(l[0].1 < ICON_W && l[6].0 + l[6].1 <= 300.0);
        // Never smaller than `MIN_W`.
        assert!(layout(40, 0.0, 400.0).iter().all(|(_, w)| *w == MIN_W));
        assert!(layout(0, 0.0, 400.0).is_empty());
    }

    const APPLETSRC: &str = "\
[Containments][2][Applets][3]
plugin=org.kde.plasma.kickoff

[Containments][2][Applets][5]
immutability=1
plugin=org.kde.plasma.icontasks

[Containments][2][Applets][5][Configuration][General]
launchers=applications:hibernate-to-windows.desktop,applications:helium.desktop,applications:com.t3tools.T3Code.desktop,applications:gone.desktop,preferred://browser,file:///home/u/.local/bin/notes,file:///opt/My%20Tool/tool,file:///home/u/.local/share/applications/files.desktop

[Containments][9][Applets][1]
plugin=org.kde.plasma.taskmanager

[Containments][9][Applets][1][Configuration][General]
launchers=applications:other.desktop
";

    #[test]
    fn pins_from_kde() {
        let launchers = kde_launchers(APPLETSRC);
        assert_eq!(launchers.len(), 8);
        assert_eq!(launchers[0], "applications:hibernate-to-windows.desktop");
        // Without an icons-only task manager, the plain one's.
        let plain = APPLETSRC.replace("plasma.icontasks", "plasma.pager");
        assert_eq!(kde_launchers(&plain), ["applications:other.desktop"]);
        assert!(kde_launchers("").is_empty());
        assert!(kde_launchers("[Containments][2][Applets][5][Configuration][General]\nlaunchers=applications:x.desktop\n").is_empty());

        let entry = |id: &str, exec: &str| DesktopEntry { id: id.into(), name: id.into(), exec: exec.into(), ..Default::default() };
        let apps = [
            entry("hibernate-to-windows", "hibernate"),
            entry("helium", "helium %U"),
            DesktopEntry { no_display: true, ..entry("com.t3tools.T3Code", "/x/T3.AppImage %U") },
            entry("notes", "notes"),
            entry("mytool", "'/opt/My Tool/tool'"),
            entry("tool-by-path", "/opt/My Tool/tool"),
            entry("files", "files"),
            entry("other", "other"),
        ];
        assert_eq!(
            resolve_launchers(&launchers, &apps),
            ["hibernate-to-windows", "helium", "com.t3tools.T3Code", "notes", "files"]
        );
        // A file launcher finds the app running that exact program first.
        let by_path = [entry("a", "/home/u/.local/bin/notes --x"), entry("notes", "notes")];
        assert_eq!(resolve_launchers(&["file:///home/u/.local/bin/notes".into()], &by_path), ["a"]);
        let by_program = [entry("b", "notes %U")];
        assert_eq!(resolve_launchers(&["file:///home/u/.local/bin/notes".into()], &by_program), ["b"]);
        assert_eq!(percent_decode("/a%20b/c%2"), "/a b/c%2");
    }

    #[test]
    fn pins_load_toggle_and_save() {
        let dir = std::env::temp_dir().join(format!("y7-pins-test-{}", std::process::id()));
        let path = dir.join("taskbar-pins");
        let _ = std::fs::remove_dir_all(&dir);
        let mut pins = Pins::load(Some(path.clone()), &Pins::TASKBAR);
        assert_eq!(pins.ids, Pins::TASKBAR);
        pins.toggle("notes");
        pins.toggle("org.kde.konsole");
        assert_eq!(pins.ids, ["files", "calendar", "calculator", "org.kde.konsole"]);
        let again = Pins::load(Some(path.clone()), &Pins::TASKBAR);
        assert_eq!(again.ids, pins.ids);
        // Unpinning everything sticks (an empty file isn't "use the defaults").
        for id in pins.ids.clone() {
            pins.toggle(&id);
        }
        assert!(Pins::load(Some(path.clone()), &Pins::TASKBAR).ids.is_empty());
        std::fs::write(&path, "# mine\nfiles\n\n  notes  \n").unwrap();
        assert_eq!(Pins::load::<&str>(Some(path), &[]).ids, ["files", "notes"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn show_desktop_round_trip() {
        let mut sd = ShowDesktop::default();
        let mut windows = vec![win(1, "a"), win(2, "b"), win(3, "c")];
        windows[0].activated = true;
        windows[2].minimized = true;
        assert_eq!(sd.toggle(&windows), DesktopAction::Minimize(vec![1, 2]));
        windows.iter_mut().for_each(|w| w.minimized = true);
        // Window 2 closed meanwhile; the active one comes back last (on top).
        windows.remove(1);
        assert_eq!(sd.toggle(&windows), DesktopAction::Restore(vec![1]));
        // Nothing to restore twice.
        assert_eq!(sd.toggle(&windows), DesktopAction::Restore(vec![]));
    }
}
