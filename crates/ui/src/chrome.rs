//! Window controls drawn inside an app's own title bar or toolbar.
//!
//! Under the y7ya-os desktop they're macOS traffic lights on the left. On
//! other desktops they follow KDE's button layout and Breeze look.

use crate::cx::Cx;
use crate::element::{Div, Styled, canvas, div};
use crate::geom::{Bounds, Color, pt, rgb};
use crate::platform::{WindowButton, is_y7ya, window_button_layout};
use crate::scene::Scene;

/// The window buttons for one end of a header, or `None` if that end has none.
/// `ink` is the color for Breeze glyphs (usually the text color).
pub fn window_buttons<S: 'static>(left: bool, cx: &Cx, ink: Color) -> Option<Div<S>> {
    if is_y7ya() {
        return left.then(|| traffic_lights(cx));
    }
    let (l, r) = window_button_layout();
    let buttons = if left { l } else { r };
    if buttons.is_empty() {
        return None;
    }
    let mut row = div().flex_row().items_center().gap(6.0);
    for kind in buttons {
        let hovered = cx.hovered(("window-button", kind));
        let (bg, fg) = match (kind, hovered) {
            (WindowButton::Close, true) => (Some(rgb(0xda4453)), Color::WHITE),
            (_, true) => (Some(ink.alpha(0.15)), ink),
            _ => (None, ink),
        };
        let glyph: fn(&mut Scene, Bounds, Color) = match kind {
            WindowButton::Close => breeze_close,
            WindowButton::Minimize => breeze_minimize,
            WindowButton::Maximize if cx.maximized => breeze_restore,
            WindowButton::Maximize => breeze_maximize,
        };
        row = row.child(
            div()
                .id(("window-button", kind))
                .size(20.0)
                .rounded(10.0)
                .bg_opt(bg)
                .on_click(move |_: &mut S, cx: &mut Cx| press(kind, cx))
                .child(canvas(move |s, b| glyph(s, b, fg)).size(20.0)),
        );
    }
    Some(row)
}

fn press(kind: WindowButton, cx: &mut Cx) {
    match kind {
        WindowButton::Close => cx.close_window(),
        WindowButton::Minimize => cx.minimize_window(),
        WindowButton::Maximize => cx.toggle_maximize(),
    }
}

/// macOS traffic lights: 12pt circles, 8pt apart, glyphs on hover.
fn traffic_lights<S: 'static>(cx: &Cx) -> Div<S> {
    let hover = cx.hovered("traffic-lights");
    let active = cx.window_focused || hover;
    let dark = cx.dark;
    let mut row = div().id("traffic-lights").flex_row().items_center().gap(8.0).hover_bg(Color::TRANSPARENT);
    for kind in [WindowButton::Close, WindowButton::Minimize, WindowButton::Maximize] {
        let pressed = cx.hovered(("traffic-light", kind)) && cx.mouse_down;
        row = row.child(
            div()
                .id(("traffic-light", kind))
                .size(12.0)
                .on_click(move |_: &mut S, cx: &mut Cx| press(kind, cx))
                .child(canvas(move |s, b| light(s, b, kind, active, hover, pressed, dark)).size(12.0)),
        );
    }
    row
}

fn light(scene: &mut Scene, b: Bounds, kind: WindowButton, active: bool, hover: bool, pressed: bool, dark: bool) {
    let (fill, rim, ink) = if active {
        match kind {
            WindowButton::Close => (rgb(0xff5f57), rgb(0xe0443e), rgb(0x4d0000)),
            WindowButton::Minimize => (rgb(0xfebc2e), rgb(0xde9a19), rgb(0x643c00)),
            WindowButton::Maximize => (rgb(0x28c840), rgb(0x1ba331), rgb(0x004d00)),
        }
    } else if dark {
        (rgb(0x4d4d4d), rgb(0x414141), Color::TRANSPARENT)
    } else {
        (rgb(0xcecece), rgb(0xbebebe), Color::TRANSPARENT)
    };
    let shade = |c: Color| if pressed { Color { r: c.r * 0.78, g: c.g * 0.78, b: c.b * 0.78, a: c.a } } else { c };
    scene.rect(b, shade(rim), b.w / 2.0);
    let inner = Bounds { x: b.x + 0.5, y: b.y + 0.5, w: b.w - 1.0, h: b.h - 1.0 };
    scene.rect(inner, shade(fill), inner.w / 2.0);
    if !hover {
        return;
    }
    let c = b.center();
    let ink = ink.alpha(0.85);
    match kind {
        WindowButton::Close => {
            let k = 2.6;
            scene.segment(pt(c.x - k, c.y - k), pt(c.x + k, c.y + k), 1.1, ink);
            scene.segment(pt(c.x - k, c.y + k), pt(c.x + k, c.y - k), 1.1, ink);
        }
        WindowButton::Minimize => scene.segment(pt(c.x - 3.3, c.y), pt(c.x + 3.3, c.y), 1.2, ink),
        WindowButton::Maximize => {
            // Two small triangles pointing apart (enter full screen).
            for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, 1.0)] {
                let tip = pt(c.x + sx * 3.2, c.y + sy * 3.2);
                for i in 0..=6 {
                    let t = i as f32 / 6.0 * 3.4;
                    scene.segment(pt(tip.x - sx * t, tip.y), pt(tip.x, tip.y - sy * t), 0.9, ink);
                }
            }
        }
    }
}

// Breeze glyphs, drawn on Breeze's 18-unit grid.

fn grid(b: Bounds) -> (f32, f32, f32) {
    let c = b.center();
    (c.x - 9.0, c.y - 9.0, 1.0)
}

fn breeze_close(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = grid(b);
    scene.segment(pt(x + 5.0 * s, y + 5.0 * s), pt(x + 13.0 * s, y + 13.0 * s), 1.3, color);
    scene.segment(pt(x + 13.0 * s, y + 5.0 * s), pt(x + 5.0 * s, y + 13.0 * s), 1.3, color);
}

fn breeze_minimize(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = grid(b);
    scene.segment(pt(x + 4.0 * s, y + 7.0 * s), pt(x + 9.0 * s, y + 12.0 * s), 1.3, color);
    scene.segment(pt(x + 9.0 * s, y + 12.0 * s), pt(x + 14.0 * s, y + 7.0 * s), 1.3, color);
}

fn breeze_maximize(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = grid(b);
    scene.segment(pt(x + 4.0 * s, y + 11.0 * s), pt(x + 9.0 * s, y + 6.0 * s), 1.3, color);
    scene.segment(pt(x + 9.0 * s, y + 6.0 * s), pt(x + 14.0 * s, y + 11.0 * s), 1.3, color);
}

fn breeze_restore(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = grid(b);
    let pts = [pt(x + 4.0 * s, y + 9.0 * s), pt(x + 9.0 * s, y + 4.0 * s), pt(x + 14.0 * s, y + 9.0 * s), pt(x + 9.0 * s, y + 14.0 * s)];
    for i in 0..4 {
        scene.segment(pts[i], pts[(i + 1) % 4], 1.3, color);
    }
}
