//! Sorting processes into "Apps" (things with windows) and "Background
//! processes", the way Windows Task Manager does.
//!
//! Each process gets an app key: the installed application its executable,
//! name or systemd launch unit resolves to. Helpers without one inherit their
//! parent's key when they were launched in the same unit (browser renderers,
//! shells in a terminal tab). Which keys count as apps comes from the
//! compositor's window list when it shares one ([`super::toplevel`]); otherwise
//! from whether the app's processes have a display in their environment.

use std::collections::{HashMap, HashSet};

use super::desktop::AppIndex;
use super::process::Process;
use super::toplevel::Toplevel;

#[derive(Clone, Debug, PartialEq)]
pub struct AppGroup {
    pub key: String,
    pub name: String,
    /// Window titles, when the compositor shares them.
    pub titles: Vec<String>,
    pub pids: Vec<i32>,
}

#[derive(Default)]
pub struct Grouping {
    pub apps: Vec<AppGroup>,
    pub background: Vec<i32>,
}

pub fn group(procs: &[Process], index: &AppIndex, windows: Option<&[Toplevel]>) -> Grouping {
    let by_pid: HashMap<i32, &Process> = procs.iter().map(|p| (p.pid, p)).collect();

    // A window app_id that names no installed app still matches processes by name.
    let raw_ids: HashSet<String> = windows
        .unwrap_or_default()
        .iter()
        .filter(|w| index.resolve(&w.app_id).is_none())
        .map(|w| w.app_id.to_lowercase())
        .collect();

    let own_key = |p: &Process| -> Option<String> {
        if p.kernel {
            return None;
        }
        let exe_name = p.exe.as_ref().and_then(|e| e.file_name()).map(|n| n.to_string_lossy().to_string());
        let candidates = exe_name.iter().chain(std::iter::once(&p.name)).chain(p.app_id.iter());
        for c in candidates {
            if let Some(i) = index.resolve(c) {
                let app = index.get(i);
                if !app.hidden {
                    return Some(app.id.clone());
                }
            }
            if raw_ids.contains(&c.to_lowercase()) {
                return Some(format!("raw:{}", c.to_lowercase()));
            }
        }
        None
    };

    // Resolve keys parent-first so inheritance sees the parent's final key.
    let mut keys: HashMap<i32, Option<String>> = HashMap::new();
    fn key_of(
        pid: i32,
        by_pid: &HashMap<i32, &Process>,
        keys: &mut HashMap<i32, Option<String>>,
        own: &dyn Fn(&Process) -> Option<String>,
        depth: u32,
    ) -> Option<String> {
        if let Some(k) = keys.get(&pid) {
            return k.clone();
        }
        let p = by_pid.get(&pid)?;
        let mut k = own(p);
        if k.is_none() && depth < 64 {
            if let Some(parent) = by_pid.get(&p.ppid) {
                if parent.app_id == p.app_id {
                    k = key_of(p.ppid, by_pid, keys, own, depth + 1);
                }
            }
        }
        keys.insert(pid, k.clone());
        k
    }
    for p in procs {
        key_of(p.pid, &by_pid, &mut keys, &own_key, 0);
    }

    let windowed: Option<HashMap<String, Vec<String>>> = windows.map(|ws| {
        let mut m: HashMap<String, Vec<String>> = HashMap::new();
        for w in ws {
            let key = match index.resolve(&w.app_id) {
                Some(i) => index.get(i).id.clone(),
                None => format!("raw:{}", w.app_id.to_lowercase()),
            };
            m.entry(key).or_default().push(w.title.clone());
        }
        m
    });

    let mut members: HashMap<String, Vec<i32>> = HashMap::new();
    for p in procs {
        if let Some(Some(k)) = keys.get(&p.pid) {
            members.entry(k.clone()).or_default().push(p.pid);
        }
    }
    let mut out = Grouping::default();
    let mut grouped: HashSet<i32> = HashSet::new();
    for (key, mut pids) in members {
        let is_app = match &windowed {
            Some(w) => w.contains_key(&key),
            None => pids.iter().any(|pid| by_pid[pid].gui),
        };
        if !is_app {
            continue;
        }
        // Parents before children; otherwise oldest first.
        pids.sort_by_key(|pid| (pids_depth(*pid, &by_pid), by_pid[pid].start_ticks));
        // Apps are per user: another account's processes (say, a session
        // started with `su` in a terminal tab) stay in the background.
        let uid = by_pid[&pids[0]].uid;
        pids.retain(|pid| by_pid[pid].uid == uid);
        let name = match key.strip_prefix("raw:") {
            Some(raw) => title_case(raw),
            None => index.resolve(&key).map(|i| index.get(i).name.clone()).unwrap_or_else(|| key.clone()),
        };
        let titles = windowed.as_ref().and_then(|w| w.get(&key).cloned()).unwrap_or_default();
        grouped.extend(&pids);
        out.apps.push(AppGroup { key, name, titles, pids });
    }
    out.apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.background = procs.iter().filter(|p| !p.kernel && !grouped.contains(&p.pid)).map(|p| p.pid).collect();
    out
}

fn pids_depth(pid: i32, by_pid: &HashMap<i32, &Process>) -> u32 {
    let mut d = 0;
    let mut cur = pid;
    while let Some(p) = by_pid.get(&cur) {
        if d > 64 || p.ppid == cur {
            break;
        }
        cur = p.ppid;
        d += 1;
    }
    d
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: i32, ppid: i32, name: &str, gui: bool, app_id: Option<&str>) -> Process {
        Process {
            pid,
            ppid,
            name: name.into(),
            cmdline: name.into(),
            exe: Some(format!("/usr/bin/{name}").into()),
            uid: 1000,
            state: 'S',
            threads: 1,
            start_ticks: pid as u64,
            cpu: 0.0,
            cpu_ticks: 0,
            mem: 0,
            disk: 0.0,
            kernel: false,
            gui,
            app_id: app_id.map(Into::into),
        }
    }

    fn index() -> AppIndex {
        let mut ix = AppIndex::default();
        ix.add("org.kde.konsole", "[Desktop Entry]\nType=Application\nName=Konsole\nExec=konsole\n");
        ix.add("firefox", "[Desktop Entry]\nType=Application\nName=Firefox\nExec=/usr/lib/firefox/firefox %u\n");
        ix.add("kded", "[Desktop Entry]\nType=Application\nName=KDED\nExec=kded6\nNoDisplay=true\n");
        ix
    }

    fn sample() -> Vec<Process> {
        let konsole = Some("org.kde.konsole");
        vec![
            proc(1, 0, "systemd", false, None),
            proc(10, 1, "konsole", true, konsole),
            proc(11, 10, "zsh", true, konsole),
            proc(12, 11, "cargo", true, konsole),
            proc(20, 1, "firefox", true, Some("firefox")),
            proc(21, 20, "Isolated Web Co", true, Some("firefox")),
            proc(30, 1, "kded6", true, None),
            proc(40, 1, "pipewire", false, None),
            // Launched from Konsole but in its own unit: not Konsole's.
            proc(50, 11, "mystery", true, Some("other")),
        ]
    }

    #[test]
    fn groups_by_app_without_a_window_list() {
        let g = group(&sample(), &index(), None);
        let names: Vec<&str> = g.apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Firefox", "Konsole"]);
        assert_eq!(g.apps[1].pids, [10, 11, 12]);
        assert_eq!(g.apps[0].pids, [20, 21]);
        let mut bg = g.background.clone();
        bg.sort();
        assert_eq!(bg, [1, 30, 40, 50]);
    }

    #[test]
    fn window_list_decides_what_is_an_app() {
        let windows = [Toplevel { app_id: "org.kde.konsole".into(), title: "~ : zsh".into() }, Toplevel { app_id: "pipewire".into(), title: "x".into() }];
        let g = group(&sample(), &index(), Some(&windows));
        let names: Vec<&str> = g.apps.iter().map(|a| a.name.as_str()).collect();
        // Firefox has no window here; pipewire matched by name.
        assert_eq!(names, ["Konsole", "Pipewire"]);
        assert_eq!(g.apps[0].titles, ["~ : zsh"]);
        assert!(g.background.contains(&20));
    }
}
