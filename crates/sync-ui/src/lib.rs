//! "Sync with Phone": the sidebar row and pairing sheet every app shows.
//!
//! Pairing belongs to the whole computer (the sync daemon), so whichever app
//! the user pairs from, all of them sync with that device. Apps embed a
//! [`Pairing`], implement [`PairingHost`], and draw [`Pairing::sidebar_row`]
//! and [`Pairing::sheet`].

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ui::*;
use y7_sync::{Hub, Listener, Status};

/// How long a pairing code stays valid (matches y7-sync).
const PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
const GREEN: u32 = 0x34c759;
const RED: u32 = 0xff3b30;

/// Apps that show the pairing sheet.
pub trait PairingHost: 'static {
    fn pairing(&mut self) -> &mut Pairing;
}

/// Colors from the app's theme.
#[derive(Clone, Copy)]
pub struct Colors {
    /// The app's accent (Notes yellow, Reminders blue, ...).
    pub accent: Color,
    /// Text on the accent-filled Done button.
    pub on_accent: Color,
    pub content: Color,
    pub field: Color,
    pub text_secondary: Color,
    pub hover: Color,
    pub border: Color,
    pub shadow: Color,
}

/// The sheet while it's open.
struct Sheet {
    /// Devices paired before the sheet opened, to spot the new one.
    known: Vec<String>,
    opened: Instant,
}

pub struct Pairing {
    hub: Option<Arc<dyn Hub>>,
    sheet: Option<Sheet>,
}

impl Pairing {
    /// `None` in previews without sync.
    pub fn new(hub: Option<Arc<dyn Hub>>) -> Self {
        Pairing { hub, sheet: None }
    }

    pub fn status(&self) -> Status {
        self.hub.as_ref().map(|h| h.status()).unwrap_or_default()
    }

    pub fn is_open(&self) -> bool {
        self.sheet.is_some()
    }

    pub fn open(&mut self) {
        let known = self.status().peers.into_iter().map(|p| p.id).collect();
        if let Some(hub) = &self.hub {
            hub.start_pairing();
        }
        self.sheet = Some(Sheet { known, opened: Instant::now() });
    }

    pub fn close(&mut self) {
        if let Some(hub) = &self.hub {
            hub.cancel_pairing();
        }
        self.sheet = None;
    }

    /// For previews: blocks until the pairing code is ready.
    pub fn wait_for_code(&self) {
        let start = Instant::now();
        while self.status().pairing_code.is_none() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Redraw when a shown pairing code expires.
    pub fn next_wake(&self) -> Option<Instant> {
        let expiry = self.sheet.as_ref().map(|s| s.opened + PAIRING_TTL + Duration::from_secs(1))?;
        (expiry > Instant::now()).then_some(expiry)
    }

    /// While the sheet is open it takes every key; Escape and Enter close it.
    pub fn on_key(&mut self, key: &KeyPress) -> bool {
        if self.sheet.is_none() {
            return false;
        }
        if key.is_named(NamedKey::Escape) || key.is_named(NamedKey::Enter) {
            self.close();
        }
        true
    }

    /// The phone this computer syncs with, or an invitation to set it up.
    pub fn sidebar_row<S: PairingHost>(&self, c: &Colors) -> Div<S> {
        let status = self.status();
        let accent = c.accent;
        let (label, detail, connected) = match status.peers.first() {
            None => ("Sync with Phone".to_string(), None, false),
            Some(p) => {
                let detail = if p.connected {
                    "Connected".to_string()
                } else if let Some(ts) = p.last_sync {
                    format!("Synced {}", ago(ts))
                } else {
                    "Not connected".to_string()
                };
                (p.name.clone(), Some(detail), p.connected)
            }
        };
        div()
            .id("sync-row")
            .mx(ui::native::SIDEBAR_INSET)
            .h(40.0)
            .shrink_0()
            .px(ui::native::SIDEBAR_ROW_PAD)
            .rounded(ui::native::SIDEBAR_RADIUS)
            .flex_row()
            .items_center()
            .gap(8.0)
            .cursor(CursorIcon::Pointer)
            .hover_bg(c.hover)
            .on_click(|s: &mut S, _: &mut Cx| s.pairing().open())
            .child(canvas(move |s, b| phone(s, b, accent)).size(ui::native::SIDEBAR_ICON).shrink_0())
            .child(
                div()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(text(label).truncate())
                    .child_if(detail.is_some(), || {
                        text(detail.clone().unwrap_or_default()).size(11.0).color(c.text_secondary).truncate().into()
                    }),
            )
            .child_if(connected, || div().size(7.0).rounded(3.5).bg(rgb(GREEN)).shrink_0().into())
    }

    /// The pairing sheet, while open: a QR code for the phone to scan, and the
    /// paired devices.
    pub fn sheet<S: PairingHost>(&self, cx: &Cx, c: &Colors) -> Option<Div<S>> {
        let sheet = self.sheet.as_ref()?;
        let status = self.status();
        let new_peer = status.peers.iter().find(|p| !sheet.known.contains(&p.id));
        let secondary = c.text_secondary;

        let code_box = div().size(232.0).rounded(14.0).items_center().justify_center().flex_col().gap(10.0);
        let code_box = match (new_peer, &status.pairing_code) {
            (Some(p), _) => code_box
                .bg(c.field)
                .child(
                    div()
                        .size(56.0)
                        .rounded(28.0)
                        .bg(rgb(GREEN))
                        .items_center()
                        .justify_center()
                        .child(canvas(|s, b| check(s, b, Color::WHITE, 3.0)).size(32.0)),
                )
                .child(text(format!("Paired with {}", p.name)).size(14.0).weight(Weight::Semibold)),
            (None, Some(code)) => {
                let code = code.clone();
                code_box.bg(Color::WHITE).child(canvas(move |s, b| draw_qr(s, b, &code, rgb(0x000000))).size(204.0))
            }
            (None, None) if sheet.opened.elapsed() >= PAIRING_TTL => code_box
                .bg(c.field)
                .child(text("This code expired").color(secondary))
                .child(
                    div()
                        .id("sync-new-code")
                        .h(28.0)
                        .px(14.0)
                        .rounded(7.0)
                        .items_center()
                        .border(c.border, 1.0)
                        .hover_bg(c.hover)
                        .on_click(|s: &mut S, _: &mut Cx| {
                            let p = s.pairing();
                            p.close();
                            p.open();
                        })
                        .child(text("Show New Code")),
                ),
            (None, None) => code_box.bg(c.field).child(text("Preparing code\u{2026}").color(secondary)),
        };

        let mut card = div()
            .w(420.0)
            .flex_col()
            .items_center()
            .px(28.0)
            .pt(24.0)
            .pb(20.0)
            .gap(14.0)
            .rounded(12.0)
            .bg(c.content)
            .border(c.border, 0.5)
            .shadow(c.shadow, 30.0, 10.0)
            .occlude()
            .child(text("Sync with Your Phone").size(17.0).weight(Weight::Bold))
            .child(
                text(
                    "Notes, Calendar and Reminders sync directly between your devices. No account and no cloud: \
                     everything is end-to-end encrypted and only goes to devices you pair.",
                )
                .size(12.5)
                .line_height(1.4)
                .color(secondary)
                .w_full(),
            )
            .child(code_box)
            .child_if(new_peer.is_none(), || {
                div()
                    .flex_col()
                    .gap(4.0)
                    .w_full()
                    .child(text("On your Android phone, open Notes, tap Sync with Computer, then Scan Code.").size(12.5).line_height(1.4).w_full())
                    .child(text("The code works once and expires after 10 minutes.").size(11.5).color(secondary).w_full())
                    .into()
            });

        if !status.peers.is_empty() {
            let mut list = div().flex_col().w_full().rounded(8.0).bg(c.field).py(2.0);
            for p in &status.peers {
                let id = p.id.clone();
                let detail = if p.connected {
                    "Connected".to_string()
                } else {
                    p.last_sync.map_or("Not connected yet".to_string(), |ts| format!("Last synced {}", ago(ts)))
                };
                let accent = c.accent;
                list = list.child(
                    div()
                        .h(44.0)
                        .px(12.0)
                        .flex_row()
                        .items_center()
                        .gap(10.0)
                        .child(canvas(move |s, b| phone(s, b, accent)).size(18.0).shrink_0())
                        .child(
                            div()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(text(p.name.clone()).weight(Weight::Semibold).truncate())
                                .child(text(detail).size(11.0).color(if p.connected { rgb(GREEN) } else { secondary }).truncate()),
                        )
                        .child(
                            div()
                                .id(("unpair", p.id.clone()))
                                .h(24.0)
                                .px(10.0)
                                .rounded(6.0)
                                .items_center()
                                .hover_bg(c.hover)
                                .on_click(move |s: &mut S, _: &mut Cx| {
                                    if let Some(hub) = &s.pairing().hub {
                                        hub.unpair(&id);
                                    }
                                })
                                .child(text("Unpair").color(rgb(RED))),
                        ),
                );
            }
            card = card.child(
                div()
                    .flex_col()
                    .w_full()
                    .gap(6.0)
                    .child(text("PAIRED DEVICES").size(10.5).weight(Weight::Semibold).color(secondary))
                    .child(list),
            );
        }

        card = card.child(
            div()
                .id("sync-done")
                .mt(4.0)
                .h(30.0)
                .w(120.0)
                .rounded(7.0)
                .bg(c.accent)
                .items_center()
                .justify_center()
                .on_click(|s: &mut S, _: &mut Cx| s.pairing().close())
                .child(text("Done").color(c.on_accent).weight(Weight::Semibold)),
        );

        Some(
            div()
                .inset_0()
                .deferred()
                .bg(rgba(0x000000, if cx.dark { 0.5 } else { 0.28 }))
                .items_center()
                .justify_center()
                .on_mouse_down(|s: &mut S, _| s.pairing().close())
                .child(card),
        )
    }
}

/// Records what changed and wakes the UI thread, which picks it up in
/// `App::on_external`. Give it to [`y7_sync::Client::connect_default`].
#[derive(Default)]
pub struct SyncEvents {
    waker: Mutex<Option<Waker>>,
    changed: Mutex<Vec<String>>,
}

impl SyncEvents {
    pub fn set_waker(&self, waker: Waker) {
        *self.waker.lock().unwrap() = Some(waker);
    }

    /// True once after document `doc` changed in another app or device.
    pub fn take_changed(&self, doc: &str) -> bool {
        let mut changed = self.changed.lock().unwrap();
        let had = changed.iter().any(|d| d == doc);
        changed.retain(|d| d != doc);
        had
    }

    fn wake(&self) {
        if let Some(w) = &*self.waker.lock().unwrap() {
            w.wake();
        }
    }
}

impl Listener for SyncEvents {
    fn doc_changed(&self, doc: &str) {
        let mut changed = self.changed.lock().unwrap();
        if !changed.iter().any(|d| d == doc) {
            changed.push(doc.to_string());
        }
        drop(changed);
        self.wake();
    }

    fn status_changed(&self) {
        self.wake();
    }
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(ts: i64) -> String {
    match y7_sync::now() - ts {
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

fn frame(b: Bounds, size: f32) -> (f32, f32, f32) {
    let c = b.center();
    (c.x - size / 2.0, c.y - size / 2.0, size / 18.0)
}

/// SF Symbols–style iPhone outline.
pub fn phone(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    scene.rect_bordered(Bounds::new(x + 4.5 * s, y + 1.5 * s, 9.0 * s, 15.0 * s), Color::TRANSPARENT, 2.2 * s, color, 1.4);
    scene.segment(pt(x + 7.8 * s, y + 13.8 * s), pt(x + 10.2 * s, y + 13.8 * s), 1.2, color);
}

fn check(scene: &mut Scene, b: Bounds, color: Color, stroke: f32) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 3.5 * s, y + 9.5 * s), pt(x + 7.5 * s, y + 13.5 * s), stroke, color);
    scene.segment(pt(x + 7.5 * s, y + 13.5 * s), pt(x + 14.5 * s, y + 4.5 * s), stroke, color);
}
