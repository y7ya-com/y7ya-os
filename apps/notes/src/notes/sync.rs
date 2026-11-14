//! Glue between the sync core (background threads) and the UI thread.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ui::{Bounds, Color, Scene, Waker};

/// Records what changed and wakes the UI, which picks it up in `on_external`.
#[derive(Default)]
pub struct SyncEvents {
    waker: Mutex<Option<Waker>>,
    notes: AtomicBool,
}

impl SyncEvents {
    pub fn set_waker(&self, waker: Waker) {
        *self.waker.lock().unwrap() = Some(waker);
    }

    /// True once after notes arrived from another device.
    pub fn take_notes_changed(&self) -> bool {
        self.notes.swap(false, Ordering::SeqCst)
    }

    fn wake(&self) {
        if let Some(w) = &*self.waker.lock().unwrap() {
            w.wake();
        }
    }
}

impl notes_core::Listener for SyncEvents {
    fn notes_changed(&self) {
        self.notes.store(true, Ordering::SeqCst);
        self.wake();
    }

    fn status_changed(&self) {
        self.wake();
    }
}

/// This computer's name as shown on the phone.
pub fn device_name() -> String {
    let pretty = std::fs::read_to_string("/etc/machine-info").ok().and_then(|s| {
        s.lines().find_map(|l| l.strip_prefix("PRETTY_HOSTNAME=").map(|v| v.trim_matches('"').to_string()))
    });
    pretty
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Computer".into())
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(ts: i64) -> String {
    match notes_core::now() - ts {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86400 => format!("{} h ago", s / 3600),
        s if s < 2 * 86400 => "yesterday".into(),
        s => format!("{} days ago", s / 86400),
    }
}

/// Draws `data` as a QR code filling `b` (which should be square), dark on
/// whatever background is behind it.
pub fn draw_qr(scene: &mut Scene, b: Bounds, data: &str, color: Color) {
    let Ok(code) = qrcode::QrCode::with_error_correction_level(data, qrcode::EcLevel::M) else { return };
    let n = code.width();
    let colors = code.to_colors();
    let cell = (b.w.min(b.h) / n as f32).floor().max(1.0);
    let ox = b.x + ((b.w - cell * n as f32) / 2.0).floor();
    let oy = b.y + ((b.h - cell * n as f32) / 2.0).floor();
    for y in 0..n {
        // One rect per horizontal run of dark modules.
        let mut x = 0;
        while x < n {
            if colors[y * n + x] == qrcode::Color::Dark {
                let start = x;
                while x < n && colors[y * n + x] == qrcode::Color::Dark {
                    x += 1;
                }
                let r = Bounds::new(ox + start as f32 * cell, oy + y as f32 * cell, (x - start) as f32 * cell, cell);
                scene.rect(r, color, 0.0);
            } else {
                x += 1;
            }
        }
    }
}
