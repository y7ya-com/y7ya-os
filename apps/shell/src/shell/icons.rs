//! Taskbar, Start and Quick Settings glyphs, drawn with rounded strokes in
//! the SF Symbols spirit. Each centers itself in the given bounds.

use ui::{Bounds, Color, Scene, pt};

pub fn arc(scene: &mut Scene, cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32, color: Color, stroke: f32) {
    let steps = ((to_deg - from_deg).abs() / 12.0).ceil().max(3.0) as usize;
    let mut prev = None;
    for i in 0..=steps {
        let a = (from_deg + (to_deg - from_deg) * i as f32 / steps as f32).to_radians();
        let p = pt(cx + r * a.cos(), cy + r * a.sin());
        if let Some(q) = prev {
            scene.segment(q, p, stroke, color);
        }
        prev = Some(p);
    }
}

fn outline(scene: &mut Scene, b: Bounds, r: f32, color: Color, stroke: f32) {
    scene.rect_bordered(b, Color::TRANSPARENT, r, color, stroke);
}

/// The y7ya-os mark: a monoline "y" whose tail sweeps under it, in a soft
/// rounded square's footprint.
pub fn logo(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let s = b.w.min(b.h) / 16.0;
    let w = 1.9 * s;
    // Short arm, long arm, and the tail curling left.
    scene.segment(pt(c.x - 4.6 * s, c.y - 5.2 * s), pt(c.x - 0.4 * s, c.y + 1.4 * s), w, color);
    scene.segment(pt(c.x + 4.6 * s, c.y - 5.2 * s), pt(c.x - 1.0 * s, c.y + 4.6 * s), w, color);
    arc(scene, c.x - 3.0 * s, c.y + 3.4 * s, 2.3 * s, 30.0, 150.0, color, w);
    // The dot that makes it ours.
    scene.rect(Bounds::new(c.x + 3.0 * s, c.y + 3.0 * s, 2.6 * s, 2.6 * s), color, 1.3 * s);
}

/// Wi-Fi fan: lit arcs for `strength` (0–100); a slashed fan when off.
pub fn wifi(scene: &mut Scene, b: Bounds, color: Color, dim: Color, strength: u8, on: bool) {
    let c = b.center();
    let base = pt(c.x, c.y + 5.0);
    let bars = match strength {
        0..=25 => 1,
        26..=50 => 2,
        _ => 3,
    };
    scene.rect(Bounds::new(base.x - 1.4, base.y - 1.4, 2.8, 2.8), if on { color } else { dim }, 1.4);
    for i in 1..=3 {
        let lit = on && i <= bars;
        arc(scene, base.x, base.y, 3.4 * i as f32, -135.0, -45.0, if lit { color } else { dim }, 1.6);
    }
    if !on {
        scene.segment(pt(c.x - 6.5, c.y - 5.5), pt(c.x + 6.5, c.y + 6.0), 1.4, color);
    }
}

pub fn battery(scene: &mut Scene, b: Bounds, color: Color, fill: Color, percent: u8, charging: bool) {
    let c = b.center();
    let (w, h) = (23.0, 11.5);
    let body = Bounds::new(c.x - w / 2.0 - 1.0, c.y - h / 2.0, w, h);
    outline(scene, body, 3.2, color.alpha(0.45), 1.0);
    scene.rect(Bounds::new(body.right() + 0.8, c.y - 2.0, 1.6, 4.0), color.alpha(0.45), 0.8);
    let inner = body.inset(2.0);
    let level = (percent as f32 / 100.0).clamp(0.08, 1.0);
    scene.rect(Bounds::new(inner.x, inner.y, inner.w * level, inner.h), fill, 1.6);
    if charging {
        let (top, mid, tail, bottom) =
            (pt(c.x + 0.4, c.y - 4.6), pt(c.x - 2.6, c.y + 0.6), pt(c.x + 2.0, c.y - 0.6), pt(c.x - 1.0, c.y + 4.6));
        scene.segment(top, mid, 1.4, color);
        scene.segment(mid, tail, 1.4, color);
        scene.segment(tail, bottom, 1.4, color);
    }
}

/// Speaker with 0–3 waves by level, or a slash when muted.
pub fn speaker(scene: &mut Scene, b: Bounds, color: Color, level: f32, muted: bool) {
    let c = b.center();
    let x = c.x - 6.0;
    scene.rect(Bounds::new(x, c.y - 2.4, 3.2, 4.8), color, 0.8);
    // The cone, filled with strokes.
    for i in 0..=4 {
        let t = i as f32 / 4.0;
        let top = pt(x + 3.0 + 3.6 * t, c.y - 2.4 - 3.0 * t);
        scene.segment(top, pt(top.x, c.y + 2.4 + 3.0 * t), 1.3, color);
    }
    if muted {
        scene.segment(pt(c.x + 3.0, c.y - 2.5), pt(c.x + 8.0, c.y + 2.5), 1.4, color);
        scene.segment(pt(c.x + 3.0, c.y + 2.5), pt(c.x + 8.0, c.y - 2.5), 1.4, color);
        return;
    }
    let waves = if level <= 0.0 { 0 } else if level < 0.34 { 1 } else if level < 0.67 { 2 } else { 3 };
    for i in 1..=3 {
        let col = if i <= waves { color } else { color.alpha(0.3) };
        arc(scene, c.x + 0.5, c.y, 2.2 + 2.6 * i as f32, -45.0, 45.0, col, 1.4);
    }
}

/// Control Center: two switches.
pub fn magnifier(scene: &mut Scene, b: Bounds, color: Color, size: f32) {
    let c = b.center();
    let s = size / 16.0;
    let r = 5.2 * s;
    let o = pt(c.x - 1.4 * s, c.y - 1.4 * s);
    scene.rect_bordered(Bounds::new(o.x - r, o.y - r, 2.0 * r, 2.0 * r), Color::TRANSPARENT, r, color, 1.6 * s.max(1.0));
    scene.segment(pt(o.x + r * 0.72, o.y + r * 0.72), pt(c.x + 6.4 * s, c.y + 6.4 * s), 2.0 * s.max(0.9), color);
}

pub fn bluetooth(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let (top, bot) = (pt(c.x, c.y - 7.0), pt(c.x, c.y + 7.0));
    let w = 1.5;
    scene.segment(top, bot, w, color);
    scene.segment(top, pt(c.x + 4.0, c.y - 3.5), w, color);
    scene.segment(pt(c.x + 4.0, c.y - 3.5), pt(c.x - 4.0, c.y + 3.5), w, color);
    scene.segment(bot, pt(c.x + 4.0, c.y + 3.5), w, color);
    scene.segment(pt(c.x + 4.0, c.y + 3.5), pt(c.x - 4.0, c.y - 3.5), w, color);
}

/// Focus / Do Not Disturb: a crescent moon, thick on the left and tapering.
pub fn moon(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let steps = 14;
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let t1 = (i + 1) as f32 / steps as f32;
        let angle = |t: f32| (60.0 + 240.0 * t).to_radians();
        // Widest at the middle of the sweep (the moon's back).
        let width = 1.2 + 3.4 * (std::f32::consts::PI * (t0 + t1) / 2.0).sin();
        let r = 5.2;
        let p = |a: f32| pt(c.x + 0.8 + r * a.cos(), c.y + r * a.sin());
        scene.segment(p(angle(t0)), p(angle(t1)), width, color);
    }
}

/// Dark Mode: a half-filled circle.
pub fn appearance(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let r = 6.5;
    scene.rect_bordered(Bounds::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r), Color::TRANSPARENT, r, color, 1.5);
    // The right half, filled with vertical strokes.
    let steps = 8;
    for i in 0..steps {
        let x = (i as f32 + 0.5) / steps as f32 * (r - 0.5);
        let h = ((r - 0.5) * (r - 0.5) - x * x).max(0.0).sqrt();
        scene.segment(pt(c.x + x, c.y - h), pt(c.x + x, c.y + h), (r - 0.5) / steps as f32 + 0.3, color);
    }
}

pub fn sun(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    scene.rect(Bounds::new(c.x - 3.2, c.y - 3.2, 6.4, 6.4), color, 3.2);
    for i in 0..8 {
        let a = (i as f32 * 45.0).to_radians();
        scene.segment(pt(c.x + 5.4 * a.cos(), c.y + 5.4 * a.sin()), pt(c.x + 7.0 * a.cos(), c.y + 7.0 * a.sin()), 1.4, color);
    }
}

pub fn play(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    for i in 0..=6 {
        let t = i as f32 / 6.0;
        let x = c.x - 4.0 + 9.0 * t;
        let half = 5.5 * (1.0 - t);
        scene.segment(pt(x, c.y - half), pt(x, c.y + half), 1.8, color);
    }
}

pub fn pause(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    for dx in [-2.8, 2.8] {
        scene.segment(pt(c.x + dx, c.y - 5.5), pt(c.x + dx, c.y + 5.5), 2.6, color);
    }
}

/// Two triangles: forward when `forward`, else back.
pub fn skip(scene: &mut Scene, b: Bounds, color: Color, forward: bool) {
    let c = b.center();
    let dir = if forward { 1.0 } else { -1.0 };
    for off in [-4.5, 1.5] {
        for i in 0..=5 {
            let t = i as f32 / 5.0;
            let x = c.x + dir * (off + 6.0 * t) - if forward { 0.0 } else { 3.0 * dir };
            let half = 4.5 * (1.0 - t);
            scene.segment(pt(x, c.y - half), pt(x, c.y + half), 1.6, color);
        }
    }
}

pub fn xmark(scene: &mut Scene, b: Bounds, color: Color, size: f32, stroke: f32) {
    let c = b.center();
    let h = size / 2.0;
    scene.segment(pt(c.x - h, c.y - h), pt(c.x + h, c.y + h), stroke, color);
    scene.segment(pt(c.x - h, c.y + h), pt(c.x + h, c.y - h), stroke, color);
}

pub fn document(scene: &mut Scene, b: Bounds, color: Color, paper: Color) {
    let c = b.center();
    let body = Bounds::new(c.x - 7.5, c.y - 10.0, 15.0, 20.0);
    scene.rect_bordered(body, paper, 2.5, color, 1.0);
    for i in 0..4 {
        let y = body.y + 6.0 + i as f32 * 3.2;
        scene.segment(pt(body.x + 3.5, y), pt(body.right() - 3.5 - if i == 3 { 4.0 } else { 0.0 }, y), 1.0, color.alpha(0.6));
    }
}

pub fn folder(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    scene.rect(Bounds::new(c.x - 11.0, c.y - 8.0, 9.0, 5.0), color.alpha(0.85), 2.0);
    scene.rect(Bounds::new(c.x - 11.0, c.y - 5.5, 22.0, 14.5), color, 2.5);
}

/// "=" on a tile: the calculator result.
pub fn equals(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    for dy in [-2.6, 2.6] {
        scene.segment(pt(c.x - 5.5, c.y + dy), pt(c.x + 5.5, c.y + dy), 2.2, color);
    }
}

pub fn arrows(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    for (y, dir) in [(c.y - 3.0, 1.0), (c.y + 3.0, -1.0)] {
        scene.segment(pt(c.x - 6.0, y), pt(c.x + 6.0, y), 1.8, color);
        let tip = pt(c.x + 6.0 * dir, y);
        scene.segment(tip, pt(tip.x - 3.0 * dir, y - 3.0), 1.8, color);
        scene.segment(tip, pt(tip.x - 3.0 * dir, y + 3.0), 1.8, color);
    }
}

/// The Start button: the y7ya-os mark knocked out of a soft rounded tile.
/// `hole` is the color behind the tile (the mark shows through as it).
pub fn start_mark(scene: &mut Scene, b: Bounds, tile: Color, hole: Color) {
    let c = b.center();
    let side = b.w.min(b.h);
    let t = Bounds::new(c.x - side / 2.0, c.y - side / 2.0, side, side);
    scene.rect(t, tile, side * 0.3);
    logo(scene, t.inset(side * 0.16), hole);
}

/// Power: a ring open at the top with a bar through the gap.
pub fn power(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    arc(scene, c.x, c.y + 0.5, 6.0, -55.0, 235.0, color, 1.6);
    scene.segment(pt(c.x, c.y - 7.0), pt(c.x, c.y - 0.5), 1.6, color);
}

/// Restart: a ring turning clockwise, its arrowhead at the top.
pub fn restart(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let (r, end) = (6.0, 250.0f32);
    arc(scene, c.x, c.y, r, -50.0, end, color, 1.6);
    let a = end.to_radians();
    let tip = pt(c.x + r * a.cos(), c.y + r * a.sin());
    // Barbs swept back from the direction of travel.
    let (tx, ty) = (-a.sin(), a.cos());
    for turn in [0.6f32, -0.6] {
        let (s, k) = turn.sin_cos();
        let (bx, by) = (-(tx * k - ty * s), -(tx * s + ty * k));
        scene.segment(tip, pt(tip.x + 3.6 * bx, tip.y + 3.6 * by), 1.6, color);
    }
}

pub fn lock(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let body = Bounds::new(c.x - 5.5, c.y - 1.0, 11.0, 8.0);
    scene.rect(body, color, 2.0);
    arc(scene, c.x, c.y - 3.5, 3.4, 180.0, 360.0, color, 1.6);
    for x in [c.x - 3.4, c.x + 3.4] {
        scene.segment(pt(x, c.y - 3.5), pt(x, c.y - 0.5), 1.6, color);
    }
}

/// Log out: an open doorway with an arrow leaving it.
pub fn log_out(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let (l, r, top, bot) = (c.x - 6.5, c.x + 1.5, c.y - 6.5, c.y + 6.5);
    scene.segment(pt(r, top + 3.0), pt(r, top), 1.6, color);
    scene.segment(pt(r, top), pt(l, top), 1.6, color);
    scene.segment(pt(l, top), pt(l, bot), 1.6, color);
    scene.segment(pt(l, bot), pt(r, bot), 1.6, color);
    scene.segment(pt(r, bot), pt(r, bot - 3.0), 1.6, color);
    scene.segment(pt(c.x - 2.0, c.y), pt(c.x + 7.0, c.y), 1.6, color);
    scene.segment(pt(c.x + 7.0, c.y), pt(c.x + 4.0, c.y - 3.0), 1.6, color);
    scene.segment(pt(c.x + 7.0, c.y), pt(c.x + 4.0, c.y + 3.0), 1.6, color);
}

pub fn chevron(scene: &mut Scene, b: Bounds, color: Color, right: bool) {
    let c = b.center();
    let d = if right { 1.0 } else { -1.0 };
    scene.segment(pt(c.x - 2.0 * d, c.y - 4.5), pt(c.x + 2.5 * d, c.y), 1.6, color);
    scene.segment(pt(c.x + 2.5 * d, c.y), pt(c.x - 2.0 * d, c.y + 4.5), 1.6, color);
}

/// A chevron `size` points tall (`chevron` is the 9pt one).
pub fn chevron_sized(scene: &mut Scene, b: Bounds, color: Color, right: bool, size: f32, stroke: f32) {
    let c = b.center();
    let (h, w) = (size / 2.0, size * 0.27);
    let d = if right { 1.0 } else { -1.0 };
    scene.segment(pt(c.x - w * d, c.y - h), pt(c.x + w * d, c.y), stroke, color);
    scene.segment(pt(c.x + w * d, c.y), pt(c.x - w * d, c.y + h), stroke, color);
}

/// A checkmark, for the chosen row of a list.
pub fn check(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    scene.segment(pt(c.x - 5.0, c.y + 0.2), pt(c.x - 1.6, c.y + 3.8), 1.8, color);
    scene.segment(pt(c.x - 1.6, c.y + 3.8), pt(c.x + 5.2, c.y - 4.2), 1.8, color);
}

/// A small padlock outline, for secured networks.
pub fn padlock(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let body = Bounds::new(c.x - 4.5, c.y - 1.0, 9.0, 7.0);
    scene.rect(body, color, 1.8);
    arc(scene, c.x, c.y - 3.0, 2.7, 180.0, 360.0, color, 1.4);
    for x in [c.x - 2.7, c.x + 2.7] {
        scene.segment(pt(x, c.y - 3.0), pt(x, c.y - 0.5), 1.4, color);
    }
}

pub fn headphones(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    arc(scene, c.x, c.y + 1.0, 6.5, 180.0, 360.0, color, 1.6);
    for x in [c.x - 6.5, c.x + 6.5] {
        scene.segment(pt(x, c.y + 1.0), pt(x, c.y + 3.0), 1.6, color);
    }
    for x in [c.x - 7.6, c.x + 4.6] {
        scene.rect(Bounds::new(x, c.y + 0.5, 3.0, 6.5), color, 1.4);
    }
}

pub fn mouse(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let body = Bounds::new(c.x - 4.8, c.y - 7.5, 9.6, 15.0);
    scene.rect_bordered(body, Color::TRANSPARENT, 4.8, color, 1.5);
    scene.segment(pt(c.x, c.y - 5.0), pt(c.x, c.y - 2.2), 1.5, color);
}

pub fn keyboard(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let body = Bounds::new(c.x - 8.5, c.y - 5.0, 17.0, 10.0);
    scene.rect_bordered(body, Color::TRANSPARENT, 2.0, color, 1.4);
    for row in 0..2 {
        for col in 0..5 {
            let k = Bounds::new(body.x + 2.6 + col as f32 * 2.6, body.y + 2.4 + row as f32 * 2.4, 1.3, 1.3);
            scene.rect(k, color, 0.4);
        }
    }
    scene.segment(pt(c.x - 3.5, c.y + 2.6), pt(c.x + 3.5, c.y + 2.6), 1.2, color);
}

pub fn phone(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let body = Bounds::new(c.x - 4.5, c.y - 8.0, 9.0, 16.0);
    scene.rect_bordered(body, Color::TRANSPARENT, 2.2, color, 1.4);
    scene.segment(pt(c.x - 1.4, c.y + 5.6), pt(c.x + 1.4, c.y + 5.6), 1.2, color);
}

pub fn computer(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let screen = Bounds::new(c.x - 7.0, c.y - 6.0, 14.0, 9.5);
    scene.rect_bordered(screen, Color::TRANSPARENT, 1.6, color, 1.4);
    scene.segment(pt(c.x - 9.0, c.y + 5.5), pt(c.x + 9.0, c.y + 5.5), 1.6, color);
}

pub fn gamepad(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let body = Bounds::new(c.x - 8.5, c.y - 4.5, 17.0, 9.5);
    scene.rect_bordered(body, Color::TRANSPARENT, 4.7, color, 1.4);
    scene.segment(pt(c.x - 5.5, c.y), pt(c.x - 2.5, c.y), 1.3, color);
    scene.segment(pt(c.x - 4.0, c.y - 1.5), pt(c.x - 4.0, c.y + 1.5), 1.3, color);
    scene.rect(Bounds::new(c.x + 2.6, c.y - 1.8, 1.8, 1.8), color, 0.9);
    scene.rect(Bounds::new(c.x + 4.4, c.y + 0.2, 1.8, 1.8), color, 0.9);
}

/// A loudspeaker box, for sound outputs.
pub fn speaker_box(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let body = Bounds::new(c.x - 5.5, c.y - 8.0, 11.0, 16.0);
    scene.rect_bordered(body, Color::TRANSPARENT, 2.2, color, 1.4);
    scene.rect_bordered(Bounds::new(c.x - 3.2, c.y - 0.6, 6.4, 6.4), Color::TRANSPARENT, 3.2, color, 1.3);
    scene.rect(Bounds::new(c.x - 1.0, c.y - 5.6, 2.0, 2.0), color, 1.0);
}

/// Wi-Fi signal for a list row: the fan with `strength` of it lit.
pub fn signal(scene: &mut Scene, b: Bounds, color: Color, dim: Color, strength: u8) {
    wifi(scene, Bounds::new(b.x, b.y - 1.0, b.w, b.h), color, dim, strength, true);
}

/// A file icon: a page with a folded corner, tinted for its kind.
pub fn page(scene: &mut Scene, b: Bounds, paper: Color, edge: Color, tint: Color) {
    let c = b.center();
    let (w, h) = (b.w * 0.66, b.h * 0.86);
    let body = Bounds::new(c.x - w / 2.0, c.y - h / 2.0, w, h);
    scene.rect_bordered(body, paper, w * 0.1, edge, 0.75);
    // The folded corner.
    let f = w * 0.3;
    scene.rect(Bounds::new(body.right() - f, body.y, f, f), tint.alpha(0.22), w * 0.08);
    // The kind's band near the bottom.
    scene.rect(Bounds::new(body.x + w * 0.16, body.bottom() - h * 0.34, w * 0.68, h * 0.2), tint, h * 0.05);
}
