//! `y7-shell`: the y7ya-os menu bar, Spotlight, Control Center and
//! notifications, in one long-running process on wlr-layer-shell.
//!
//! - `y7-shell` runs it (one per session).
//! - `y7-shell msg <command>` sends a command to the running shell; see `ipc.rs`.
//! - `y7-shell --snapshot <dir> [--light]` renders each surface to PNGs.
//!
//! `Y7_SHELL_FAKE=1` swaps the real machine for an in-memory one (no Wi-Fi
//! toggling or shutting down while testing).

mod apps;
mod calc;
mod files;
mod ipc;
mod notify;
mod search;
mod shell;
mod system;
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
            let cfg = Config { system, services: true, files_root: dirs::home_dir() };
            ui::run_layer(shell::bar_options(), move |cx| Shell::new(cfg, cx));
        }
    }
}

/// Renders every surface with demo content, for checking the look.
fn snapshots(dir: &std::path::Path, dark: bool) {
    std::fs::create_dir_all(dir).expect("snapshot dir");
    let suffix = if dark { "dark" } else { "light" };
    let screen = (1440.0, 900.0 - shell::BAR_H);
    let make = |cx: &mut ui::Cx| {
        let cfg = Config { system: Arc::new(FakeSystem::demo()), services: false, files_root: None };
        let mut s = Shell::new(cfg, cx);
        s.wait_for_status();
        demo_notifications(&s);
        s
    };
    let shots: [(&str, &str, (f32, f32), Panel); 5] = [
        ("menubar", shell::BAR, (1440.0, shell::BAR_H), Panel::None),
        ("spotlight", shell::PANEL, screen, Panel::Spotlight),
        ("control-center", shell::PANEL, screen, Panel::ControlCenter),
        ("notification-center", shell::PANEL, screen, Panel::NotificationCenter),
        ("banners", shell::BANNERS, (388.0, 260.0), Panel::None),
    ];
    for (name, layer, size, panel) in shots {
        let path = dir.join(format!("{name}-{suffix}.png"));
        ui::snapshot_layer(layer, size, 2.0, dark, &path, make, |s, cx| {
            if panel != Panel::None {
                s.open(panel, cx);
            }
            if panel == Panel::Spotlight {
                s.set_query("te", cx);
            }
        });
        println!("{}", path.display());
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
