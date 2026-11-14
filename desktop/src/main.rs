//! y7ya-os desktop: a Wayland compositor with Mac-style windows.
//!
//! `y7-desktop` runs nested in a window inside your current desktop.
//! `y7-desktop --headless [WxH] [--scale S]` renders offscreen and reads test
//! commands from stdin (see `backend/headless.rs`).
//! `-c <command>` starts a program once the desktop is up.

mod backend;
mod grabs;
mod handlers;
mod input;
mod render;
mod state;

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
    } else {
        after = Some(backend::winit::init(&mut event_loop, &mut desk)?);
    }
    println!("WAYLAND_DISPLAY={}", desk.socket_name.to_string_lossy());
    if let Some(cmd) = value("-c") {
        desk.spawn(&cmd);
    }

    event_loop.run(None, &mut desk, move |desk| {
        if let Some(after) = after.as_mut() {
            after(desk);
        }
        // Send replies to clients after every round of requests.
        let _ = desk.display_handle.flush_clients();
    })?;
    Ok(())
}
