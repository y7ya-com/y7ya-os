//! `y7-shell`: the y7ya-os taskbar, Start menu, Spotlight, Quick Settings and
//! notifications, in one long-running process on wlr-layer-shell.
//!
//! - `y7-shell` runs it (one per session).
//! - `y7-shell msg <command>` sends a command to the running shell; see `ipc.rs`.
//! - `y7-shell --snapshot <dir> [--light]` renders each surface to PNGs.
//!
//! `Y7_SHELL_FAKE=1` swaps the real machine for an in-memory one (no Wi-Fi
//! toggling or shutting down while testing).

mod app_icons;
mod apps;
mod calc;
mod desktops;
mod files;
mod ipc;
mod notify;
mod search;
mod shell;
mod system;
mod taskbar;
mod toplevel;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use shell::{Config, Panel, Shell};
use system::{FakeSystem, RealSystem, System};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("msg") => {
            let cmd = args[1..].join(" ");
            if let Err(e) = cmd.parse::<ipc::Command>() {
                eprintln!("y7-shell: {e}");
                std::process::exit(2);
            }
            match ipc::send(&ipc::socket_path(), &cmd) {
                Ok(reply) if reply == "ok" => {}
                Ok(reply) => {
                    eprintln!("y7-shell: {reply}");
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("y7-shell: the shell isn't running ({e})");
                    std::process::exit(1);
                }
            }
        }
        Some("--snapshot") => {
            let dir = PathBuf::from(args.get(1).expect("--snapshot needs a directory"));
            snapshots(&dir, !args.iter().any(|a| a == "--light"));
        }
        // What the real machine reports, read-only (to check the readers).
        Some("--status") => {
            let sys = RealSystem;
            println!("battery:    {:?}", sys.battery());
            println!("wifi:       {:?}", sys.wifi());
            println!("bluetooth:  {:?}", sys.bluetooth());
            println!("volume:     {:?}", sys.volume());
            println!("brightness: {:?}", sys.brightness());
            println!("media:      {:?}", sys.media());
        }
        Some("--help" | "-h") => {
            println!("usage: y7-shell [msg <command> | --snapshot <dir> [--light]]\n{}", ipc::USAGE);
        }
        _ => {
            if ipc::send(&ipc::socket_path(), "ping").is_ok() {
                eprintln!("y7-shell is already running");
                std::process::exit(1);
            }
            let system: Arc<dyn System> =
                if std::env::var("Y7_SHELL_FAKE").is_ok_and(|v| !v.is_empty() && v != "0") { Arc::new(FakeSystem::demo()) } else { Arc::new(RealSystem) };
            let cfg = Config { system, services: true, files_root: dirs::home_dir(), data_dir: data_dir() };
            ui::run_layer(shell::bar_options(), move |cx| Shell::new(cfg, cx));
        }
    }
}

/// `~/.local/share/y7-shell`, or `$Y7_SHELL_DATA_DIR`.
fn data_dir() -> Option<PathBuf> {
    std::env::var_os("Y7_SHELL_DATA_DIR").map(PathBuf::from).or_else(|| dirs::data_dir().map(|d| d.join("y7-shell")))
}

/// Renders every surface with demo content, for checking the look.
fn snapshots(dir: &std::path::Path, dark: bool) {
    std::fs::create_dir_all(dir).expect("snapshot dir");
    let suffix = if dark { "dark" } else { "light" };
    let screen = (1440.0, 900.0 - shell::BAR_H);
    let make = |cx: &mut ui::Cx| {
        // Default pins, not the user's.
        let cfg = Config { system: Arc::new(FakeSystem::demo()), services: false, files_root: None, data_dir: None };
        let mut s = Shell::new(cfg, cx);
        s.wait_for_status();
        demo_notifications(&s);
        s.windows = demo_windows();
        s.desktops = desktops::Desktops::fixed(demo_spaces());
        s.preload_icons();
        s
    };
    let shots: [(&str, &str, (f32, f32), Panel); 7] = [
        ("taskbar", shell::BAR, (1440.0, shell::BAR_H), Panel::None),
        ("start", shell::PANEL, screen, Panel::Start),
        ("spotlight", shell::PANEL, screen, Panel::Spotlight),
        ("quick-settings", shell::PANEL, screen, Panel::QuickSettings),
        ("calendar", shell::PANEL, screen, Panel::Calendar),
        ("confirm", shell::PANEL, screen, Panel::Confirm(system::SessionAction::ShutDown)),
        ("banners", shell::BANNERS, (388.0, 260.0), Panel::None),
    ];
    for (name, layer, size, panel) in shots {
        let path = dir.join(format!("{name}-{suffix}.png"));
        ui::snapshot_layer(layer, size, 2.0, dark, &path, make, |s, cx| {
            if panel != Panel::None {
                s.open_now(panel, cx);
            }
            if panel == Panel::Spotlight {
                s.set_query("te", cx);
            }
        });
        println!("{}", path.display());
    }
}

/// A few windows of apps most machines have, one active and one minimized.
fn demo_windows() -> Vec<toplevel::Window> {
    let w = |id, app_id: &str, title: &str| toplevel::Window { id, app_id: app_id.into(), title: title.into(), ..Default::default() };
    let mut windows = vec![
        w(1, "notes", "Notes"),
        w(2, "org.kde.konsole", "~ : zsh \u{2014} Konsole"),
        w(3, "files", "Documents"),
        w(4, "textedit", "Quarterly plan draft.txt"),
        w(5, "org.kde.konsole", "cargo build \u{2014} Konsole"),
    ];
    windows[1].activated = true;
    windows[3].minimized = true;
    windows
}

/// Three desktops, the first with a few windows.
fn demo_spaces() -> desktops::Spaces {
    let w = |rect: [f32; 4]| desktops::SpaceWindow { rect, ..Default::default() };
    desktops::Spaces {
        active: 0,
        screen: (1440.0, 856.0),
        spaces: vec![
            desktops::Space { windows: vec![w([80.0, 60.0, 760.0, 520.0]), w([520.0, 200.0, 820.0, 560.0])] },
            desktops::Space { windows: vec![w([300.0, 120.0, 900.0, 600.0])] },
            desktops::Space::default(),
        ],
    }
}

fn demo_notifications(s: &Shell) {
    let mut store = s.store.lock().unwrap();
    for (app, title, body, actions) in [
        ("Calendar", "Design review", "Today at 3:00 PM in Studio B", vec![]),
        ("Messages", "Maya", "Are we still on for dinner tonight? I booked the place by the river.", vec!["reply", "Reply"]),
    ] {
        store.notify(
            notify::Incoming {
                app: app.into(),
                title: title.into(),
                body: body.into(),
                actions: actions.into_iter().map(String::from).collect(),
                timeout_ms: -1,
                urgency: 1,
                ..Default::default()
            },
            Instant::now(),
        );
    }
}
