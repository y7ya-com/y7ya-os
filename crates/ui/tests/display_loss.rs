//! `ui::run` must leave quietly, without a panic, when the compositor hangs
//! up on it. A fake Wayland socket accepts the app and then closes.
//!
//! winit wants its event loop on the main thread, so this test has its own
//! `main` (`harness = false`) and re-runs itself as the app.

use std::io::Read;
use std::os::unix::net::UnixListener;
use std::process::Command;

struct Idle;

impl ui::App for Idle {
    fn render(&mut self, _cx: &mut ui::Cx) -> ui::Div<Self> {
        ui::div()
    }
    fn base_text_style(&self, _cx: &ui::Cx) -> ui::TextStyle {
        ui::TextStyle::default()
    }
}

fn main() {
    if std::env::var_os("UI_DISPLAY_LOSS_CHILD").is_some() {
        let opts = ui::WindowOptions {
            title: "test".into(),
            app_id: "display-loss-test".into(),
            size: (200.0, 100.0),
            min_size: (100.0, 50.0),
            decorations: false,
        };
        ui::run(opts, |_| Idle);
        return;
    }

    let dir = std::env::temp_dir().join(format!("ui-display-loss-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("wl");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).unwrap();
    let server = std::thread::spawn(move || {
        // Take the client's first request, then go away like a crashed compositor.
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 64];
        let _ = conn.read(&mut buf);
    });

    let out = Command::new(std::env::current_exe().unwrap())
        .env("UI_DISPLAY_LOSS_CHILD", "1")
        .env("XDG_RUNTIME_DIR", &dir)
        .env("WAYLAND_DISPLAY", "wl")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "the app panicked:\n{stderr}");
    assert!(out.status.success(), "exit status {:?}:\n{stderr}", out.status);
    println!("display_loss: ok");
}
