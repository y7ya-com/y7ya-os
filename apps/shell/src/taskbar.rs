//! The taskbar's model, apart from drawing: which buttons it shows in what
//! order, how wide they are, the pinned apps, and Show Desktop.
//!
//! Like Windows 10 with "never combine": pinned apps come first, and a pinned
//! app that's running shows its windows in its place. Other apps' windows
//! follow, each app's windows kept together, apps in the order they opened.

use std::path::PathBuf;

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
    pub fn load(path: Option<PathBuf>, defaults: &[&str]) -> Pins {
        let ids = match path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(text) => text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from)
                .collect(),
            None => defaults.iter().map(|s| s.to_string()).collect(),
        };
        Pins { ids, path }
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

/// One taskbar button.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A pinned app with no windows: an icon that launches it.
    Pinned { app: String },
    /// A window: icon and title. `app` is its desktop-file ID, if known.
    Window { id: u64, app: Option<String> },
}

impl Item {
    pub fn app(&self) -> Option<&str> {
        match self {
            Item::Pinned { app } => Some(app),
            Item::Window { app, .. } => app.as_deref(),
        }
    }
}

/// The desktop-file ID behind a window, if any app claims it.
pub fn app_for(apps: &[DesktopEntry], app_id: &str) -> Option<String> {
    if app_id.is_empty() {
        return None;
    }
    // An exact ID match beats the looser ones.
    apps.iter()
        .find(|e| e.id == app_id)
        .or_else(|| apps.iter().find(|e| e.matches_app_id(app_id)))
        .map(|e| e.id.clone())
}

/// The buttons, in order. `windows` are in the order they opened.
pub fn items(pins: &[String], windows: &[Window], app_of: impl Fn(&Window) -> Option<String>) -> Vec<Item> {
    let tagged: Vec<(u64, Option<String>)> = windows.iter().map(|w| (w.id, app_of(w))).collect();
    let mut out = Vec::new();
    for pin in pins {
        let mine: Vec<Item> = tagged
            .iter()
            .filter(|(_, app)| app.as_deref() == Some(pin))
            .map(|(id, app)| Item::Window { id: *id, app: app.clone() })
            .collect();
        if mine.is_empty() {
            out.push(Item::Pinned { app: pin.clone() });
        } else {
            out.extend(mine);
        }
    }
    // The rest, grouped by app in order of each app's first window. Windows
    // no app claims each stand alone.
    let mut groups: Vec<(Option<String>, Vec<u64>)> = Vec::new();
    for (id, app) in &tagged {
        if app.as_ref().is_some_and(|a| pins.contains(a)) {
            continue;
        }
        match groups.iter_mut().find(|(g, _)| g.is_some() && g == app) {
            Some((_, ids)) => ids.push(*id),
            None => groups.push((app.clone(), vec![*id])),
        }
    }
    for (app, ids) in groups {
        out.extend(ids.into_iter().map(|id| Item::Window { id, app: app.clone() }));
    }
    out
}

/// An icon-only button (pinned apps, Start, search).
pub const ICON_W: f32 = 40.0;
/// Window buttons shrink from this to fit, down to `ICON_W`.
pub const WINDOW_MAX_W: f32 = 180.0;
pub const GAP: f32 = 4.0;

/// Each button's x and width, for buttons laid out from `x` in `width`.
pub fn layout(items: &[Item], x: f32, width: f32) -> Vec<(f32, f32)> {
    let windows = items.iter().filter(|i| matches!(i, Item::Window { .. })).count();
    let pinned = items.len() - windows;
    let gaps = items.len().saturating_sub(1) as f32 * GAP;
    let room = width - gaps - pinned as f32 * ICON_W;
    // Whole points, so labels and icons land on the pixel grid.
    let window_w = if windows == 0 { 0.0 } else { (room / windows as f32).floor().clamp(ICON_W, WINDOW_MAX_W) };
    let mut at = x;
    items
        .iter()
        .map(|i| {
            let w = if matches!(i, Item::Window { .. }) { window_w } else { ICON_W };
            let r = (at, w);
            at += w + GAP;
            r
        })
        .collect()
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
            .map(|i| match i {
                Item::Pinned { app } => format!("pin:{app}"),
                Item::Window { id, app } => format!("{}#{id}", app.as_deref().unwrap_or("?")),
            })
            .collect()
    }

    #[test]
    fn pinned_first_and_windows_grouped() {
        let pins: Vec<String> = ["files", "notes", "calendar"].map(String::from).to_vec();
        let windows = [win(1, "konsole"), win(2, "notes"), win(3, "firefox"), win(4, "konsole"), win(5, "notes"), win(6, "")];
        let app_of = |w: &Window| (!w.app_id.is_empty()).then(|| w.app_id.clone());
        let items = items(&pins, &windows, app_of);
        assert_eq!(
            ids(&items),
            ["pin:files", "notes#2", "notes#5", "pin:calendar", "konsole#1", "konsole#4", "firefox#3", "?#6"]
        );
        // Unclaimed windows don't group with each other.
        let windows = [win(1, ""), win(2, "x"), win(3, "")];
        assert_eq!(ids(&super::items(&[], &windows, app_of)), ["?#1", "x#2", "?#3"]);
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
        let apps = [entry("org.kde.konsole", "Konsole", "konsole"), entry("notes", "Notes", "notes")];
        assert_eq!(app_for(&apps, "konsole").as_deref(), Some("org.kde.konsole"));
        assert_eq!(app_for(&apps, "org.kde.konsole").as_deref(), Some("org.kde.konsole"));
        assert_eq!(app_for(&apps, "notes").as_deref(), Some("notes"));
        assert_eq!(app_for(&apps, "gimp"), None);
        assert_eq!(app_for(&apps, ""), None);
    }

    #[test]
    fn buttons_shrink_to_fit() {
        let pin = |a: &str| Item::Pinned { app: a.into() };
        let w = |id| Item::Window { id, app: None };
        let items = [pin("a"), w(1), w(2)];
        // Plenty of room: full-width window buttons.
        let l = layout(&items, 100.0, 1000.0);
        assert_eq!(l, [(100.0, ICON_W), (100.0 + ICON_W + GAP, WINDOW_MAX_W), (100.0 + ICON_W + WINDOW_MAX_W + 2.0 * GAP, WINDOW_MAX_W)]);
        // Tight: windows share what's left, in whole points.
        let l = layout(&items, 0.0, 300.0);
        assert_eq!(l[1].1, ((300.0 - 2.0 * GAP - ICON_W) / 2.0f32).floor());
        assert!(l[2].0 + l[2].1 <= 300.0);
        // Never narrower than an icon.
        let many: Vec<Item> = (0..40).map(w).collect();
        assert!(layout(&many, 0.0, 400.0).iter().all(|(_, w)| *w == ICON_W));
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
        assert_eq!(Pins::load(Some(path), &[]).ids, ["files", "notes"]);
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
