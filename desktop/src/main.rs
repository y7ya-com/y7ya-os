//! y7ya-os desktop: a Wayland compositor with Mac-style windows.
//!
//! `y7-desktop --session` is the real desktop, started from the login screen.
//! `y7-desktop` runs nested in a window inside your current desktop.
//! `y7-desktop --headless [WxH] [--scale S]` renders offscreen and reads test
//! commands from stdin (see `backend/headless.rs`).
//! `-c <command>` starts a program once the desktop is up.

mod backend;
mod config;
mod grabs;
mod handlers;
mod icons;
mod input;
mod layers;
mod overview;
mod render;
mod state;
mod toplevels;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use state::Desk;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|a| a == name);
    let value = |name: &str| flag(name).and_then(|i| args.get(i + 1)).cloned();

    let mut event_loop: EventLoop<'static, Desk> = EventLoop::try_new()?;
    let display: Display<Desk> = Display::new()?;
    let mut desk = Desk::new(display, event_loop.handle(), event_loop.get_signal());

    let mut after: Option<Box<dyn FnMut(&mut Desk)>> = None;
    if let Some(i) = flag("--headless") {
        let size = args
            .get(i + 1)
            .and_then(|s| s.split_once('x'))
            .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
            .unwrap_or((1440, 900));
        let scale = value("--scale").and_then(|s| s.parse().ok()).unwrap_or(1.0);
        backend::headless::init(&mut event_loop, &mut desk, size, scale)?;
    } else if flag("--session").is_some() {
        after = Some(backend::session::init(&mut event_loop, &mut desk)?);
    } else {
        desk.nested = true;
        after = Some(backend::winit::init(&mut event_loop, &mut desk)?);
    }
    println!("WAYLAND_DISPLAY={}", desk.socket_name.to_string_lossy());
    config::init(&event_loop.handle(), &mut desk);
    desk.start_xwayland();
    if flag("--session").is_some() {
        start_session_services(&desk);
    } else if flag("--with-shell").is_some() {
        // A try-out inside another desktop: the shell and sync, but leave the
        // host session's environment alone.
        start_desktop_services(&desk);
    }
    if let Some(cmd) = value("-c") {
        desk.spawn(&cmd);
    }

    event_loop.run(None, &mut desk, move |desk| {
        if let Some(after) = after.as_mut() {
            after(desk);
        }
        desk.refresh_toplevels();
        // Send replies to clients after every round of requests.
        let _ = desk.display_handle.flush_clients();
    })?;
    Ok(())
}

/// Makes the session visible to the rest of the system and starts the
/// desktop's own background pieces.
fn start_session_services(desk: &Desk) {
    // Safety: still single-threaded here; nothing else reads the environment yet.
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &desk.socket_name);
        std::env::set_var("XDG_CURRENT_DESKTOP", "y7ya-os");
        std::env::set_var("XDG_SESSION_DESKTOP", "y7ya-os");
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        if let Some(display) = &desk.x_display {
            std::env::set_var("DISPLAY", display);
        }
    }
    let vars = ["WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "XDG_SESSION_TYPE", "DISPLAY"];
    // So portals, D-Bus-activated apps and user services find this display.
    let _ = std::process::Command::new("dbus-update-activation-environment").arg("--systemd").args(vars).status();
    let _ = std::process::Command::new("systemctl").args(["--user", "import-environment"]).args(vars).status();
    start_desktop_services(desk);
}

fn start_desktop_services(desk: &Desk) {
    for cmd in ["y7-syncd", "y7-shell", "clipboard --daemon"] {
        let program = cmd.split_whitespace().next().unwrap_or_default();
        let installed = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).exists()));
        if installed {
            desk.spawn(cmd);
        }
    }
}
