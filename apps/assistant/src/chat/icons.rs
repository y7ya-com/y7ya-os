//! Line icons in the SF Symbols spirit (see apps/notes), plus the white
//! System Settings–style glyphs that sit on each setting's colored tile.

use ui::{Bounds, Color, Scene, pt, rgb};

use crate::tools::Category;

/// An 18-unit drawing frame centered in `b`, `size` points across.
fn frame(b: Bounds, size: f32) -> (f32, f32, f32) {
    let c = b.center();
    (c.x - size / 2.0, c.y - size / 2.0, size / 18.0)
}

fn circle(scene: &mut Scene, cx: f32, cy: f32, r: f32, color: Color) {
    scene.rect(Bounds::new(cx - r, cy - r, 2.0 * r, 2.0 * r), color, r);
}

fn ring(scene: &mut Scene, cx: f32, cy: f32, r: f32, color: Color, stroke: f32) {
    scene.rect_bordered(Bounds::new(cx - r, cy - r, 2.0 * r, 2.0 * r), Color::TRANSPARENT, r, color, stroke);
}

fn arc(scene: &mut Scene, cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32, color: Color, stroke: f32) {
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

fn polyline(scene: &mut Scene, x: f32, y: f32, s: f32, pts: &[(f32, f32)], stroke: f32, color: Color) {
    for w in pts.windows(2) {
        scene.segment(pt(x + w[0].0 * s, y + w[0].1 * s), pt(x + w[1].0 * s, y + w[1].1 * s), stroke, color);
    }
}

/// A four-pointed star filled by scanlines of the astroid |x|^½ + |y|^½ = r^½.
fn star(scene: &mut Scene, cx: f32, cy: f32, r: f32, color: Color) {
    let step = 0.4;
    let mut y = -r;
    while y <= r {
        let w = (r.sqrt() - y.abs().sqrt()).powi(2);
        if w > 0.05 {
            scene.segment(pt(cx - w, cy + y), pt(cx + w, cy + y), step + 0.25, color);
        }
        y += step;
    }
}

/// The Assistant's mark: a large and a small sparkle.
pub fn sparkle(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, b.w.min(b.h));
    star(scene, x + 8.0 * s, y + 10.0 * s, 7.5 * s, color);
    star(scene, x + 14.5 * s, y + 3.8 * s, 3.2 * s, color);
}

pub fn arrow_up(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, b.w.min(b.h));
    polyline(scene, x, y, s, &[(9.0, 14.0), (9.0, 4.5)], 2.0, color);
    polyline(scene, x, y, s, &[(4.8, 8.6), (9.0, 4.4), (13.2, 8.6)], 2.0, color);
}

pub fn stop(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let h = b.w.min(b.h) * 0.2;
    scene.rect(Bounds::new(c.x - h, c.y - h, 2.0 * h, 2.0 * h), color, h * 0.35);
}

pub fn chevron_down(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, b.w.min(b.h));
    polyline(scene, x, y, s, &[(5.0, 7.0), (9.0, 11.0), (13.0, 7.0)], 1.6, color);
}

pub fn check(scene: &mut Scene, b: Bounds, color: Color, stroke: f32) {
    let (x, y, s) = frame(b, b.w.min(b.h));
    polyline(scene, x, y, s, &[(4.0, 9.5), (7.6, 13.0), (14.0, 5.2)], stroke, color);
}

pub fn check_bold(scene: &mut Scene, b: Bounds, color: Color) {
    check(scene, b, color, 2.0);
}

pub fn exclamation(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, b.w.min(b.h));
    polyline(scene, x, y, s, &[(9.0, 4.2), (9.0, 10.8)], 2.2, color);
    circle(scene, x + 9.0 * s, y + 13.9 * s, 1.25 * s, color);
}

pub fn compose(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    let st = 1.5;
    let (l, t, r, bo) = (x + 1.5 * s, y + 3.0 * s, x + 14.5 * s, y + 16.0 * s);
    scene.segment(pt(l + 2.5 * s, t), pt(x + 9.0 * s, t), st, color);
    scene.segment(pt(l, t + 2.5 * s), pt(l, bo - 2.5 * s), st, color);
    scene.segment(pt(l + 2.5 * s, bo), pt(r - 2.5 * s, bo), st, color);
    scene.segment(pt(r, bo - 2.5 * s), pt(r, y + 9.5 * s), st, color);
    arc(scene, l + 2.5 * s, t + 2.5 * s, 2.5 * s, 180.0, 270.0, color, st);
    arc(scene, l + 2.5 * s, bo - 2.5 * s, 2.5 * s, 90.0, 180.0, color, st);
    arc(scene, r - 2.5 * s, bo - 2.5 * s, 2.5 * s, 0.0, 90.0, color, st);
    scene.segment(pt(x + 7.5 * s, y + 10.5 * s), pt(x + 15.8 * s, y + 2.2 * s), 2.6 * s, color);
    scene.segment(pt(x + 6.6 * s, y + 11.4 * s), pt(x + 7.4 * s, y + 10.6 * s), 1.2, color);
}

// -- setting tiles ------------------------------------------------------------

/// The tile color behind a category's glyph, like System Settings.
pub fn tile_color(c: Category) -> Color {
    rgb(match c {
        Category::Appearance => 0x3a3a3c,
        Category::Wallpaper => 0x32ade6,
        Category::Display | Category::Wifi | Category::Bluetooth => 0x0a84ff,
        Category::NightLight => 0xff9f0a,
        Category::Sound => 0xff375f,
        Category::Power => 0x30d158,
        Category::Notifications => 0x5e5ce6,
        Category::Keyboard | Category::Apps | Category::DateTime | Category::About => 0x8e8e93,
    })
}

/// A category's glyph in `fg`, drawn over a tile of color `bg`.
pub fn glyph(c: Category, scene: &mut Scene, b: Bounds, fg: Color, bg: Color) {
    let (x, y, s) = frame(b, b.w.min(b.h));
    let st = 1.5 * s.max(0.8);
    match c {
        Category::Appearance => {
            let (cx, cy, r) = (x + 9.0 * s, y + 9.0 * s, 6.6 * s);
            ring(scene, cx, cy, r, fg, st);
            // Right half filled.
            let mut dx = 0.0;
            while dx < r {
                let h = (r * r - dx * dx).sqrt();
                scene.segment(pt(cx + dx, cy - h), pt(cx + dx, cy + h), 0.8, fg);
                dx += 0.5;
            }
        }
        Category::Display => {
            let (cx, cy) = (x + 9.0 * s, y + 9.0 * s);
            circle(scene, cx, cy, 3.3 * s, fg);
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                let (sn, cs) = a.sin_cos();
                scene.segment(pt(cx + cs * 5.6 * s, cy + sn * 5.6 * s), pt(cx + cs * 7.6 * s, cy + sn * 7.6 * s), st, fg);
            }
        }
        Category::NightLight | Category::Notifications => {
            circle(scene, x + 8.5 * s, y + 9.5 * s, 6.5 * s, fg);
            circle(scene, x + 12.3 * s, y + 6.6 * s, 5.6 * s, bg);
        }
        Category::Sound => {
            scene.rect(Bounds::new(x + 2.2 * s, y + 6.6 * s, 3.2 * s, 4.8 * s), fg, 0.8 * s);
            let mut px = 5.0;
            while px <= 9.0 {
                let h = 2.4 + (px - 5.0) / 4.0 * 3.6;
                scene.segment(pt(x + px * s, y + (9.0 - h) * s), pt(x + px * s, y + (9.0 + h) * s), 0.9, fg);
                px += 0.4;
            }
            arc(scene, x + 9.5 * s, y + 9.0 * s, 3.2 * s, -50.0, 50.0, fg, st);
            arc(scene, x + 9.5 * s, y + 9.0 * s, 6.2 * s, -50.0, 50.0, fg, st);
        }
        Category::Wifi => {
            let (cx, cy) = (x + 9.0 * s, y + 14.6 * s);
            for r in [4.0, 7.6, 11.2] {
                arc(scene, cx, cy, r * s, 225.0, 315.0, fg, 1.8 * s.max(0.8));
            }
            circle(scene, cx, cy - 0.2 * s, 1.5 * s, fg);
        }
        Category::Bluetooth => {
            polyline(scene, x, y, s, &[(5.2, 5.8), (12.6, 12.2), (9.0, 15.6), (9.0, 2.4), (12.6, 5.8), (5.2, 12.2)], st, fg);
        }
        Category::Power => {
            scene.rect_bordered(Bounds::new(x + 1.8 * s, y + 5.4 * s, 12.6 * s, 7.2 * s), Color::TRANSPARENT, 2.0 * s, fg, 1.3 * s.max(0.8));
            scene.rect(Bounds::new(x + 3.4 * s, y + 7.0 * s, 7.2 * s, 4.0 * s), fg, 0.8 * s);
            scene.rect(Bounds::new(x + 15.0 * s, y + 7.6 * s, 1.4 * s, 2.8 * s), fg, 0.6 * s);
        }
        Category::Keyboard => {
            scene.rect_bordered(Bounds::new(x + 1.5 * s, y + 4.2 * s, 15.0 * s, 9.6 * s), Color::TRANSPARENT, 2.0 * s, fg, 1.3 * s.max(0.8));
            for row in 0..2 {
                for col in 0..5 {
                    let kx = x + (3.8 + col as f32 * 2.6) * s;
                    let ky = y + (6.2 + row as f32 * 2.4) * s;
                    scene.rect(Bounds::new(kx, ky, 1.4 * s, 1.3 * s), fg, 0.3 * s);
                }
            }
            scene.segment(pt(x + 6.0 * s, y + 11.4 * s), pt(x + 12.0 * s, y + 11.4 * s), 1.3 * s, fg);
        }
        Category::Wallpaper => {
            scene.rect_bordered(Bounds::new(x + 1.8 * s, y + 3.4 * s, 14.4 * s, 11.2 * s), Color::TRANSPARENT, 2.0 * s, fg, 1.3 * s.max(0.8));
            polyline(scene, x, y, s, &[(3.4, 13.0), (7.4, 8.6), (10.4, 11.6), (12.4, 9.6), (14.8, 12.4)], 1.3 * s.max(0.8), fg);
            circle(scene, x + 12.2 * s, y + 6.6 * s, 1.3 * s, fg);
        }
        Category::Apps => {
            for (px, py) in [(2.6, 2.6), (9.6, 2.6), (2.6, 9.6), (9.6, 9.6)] {
                scene.rect(Bounds::new(x + px * s, y + py * s, 5.8 * s, 5.8 * s), fg, 1.6 * s);
            }
        }
        Category::DateTime => {
            let (cx, cy) = (x + 9.0 * s, y + 9.0 * s);
            ring(scene, cx, cy, 7.0 * s, fg, st);
            scene.segment(pt(cx, cy), pt(cx, cy - 4.4 * s), st, fg);
            scene.segment(pt(cx, cy), pt(cx + 3.2 * s, cy + 1.8 * s), st, fg);
        }
        Category::About => {
            scene.rect_bordered(Bounds::new(x + 3.4 * s, y + 3.8 * s, 11.2 * s, 7.8 * s), Color::TRANSPARENT, 1.4 * s, fg, 1.3 * s.max(0.8));
            scene.segment(pt(x + 1.6 * s, y + 13.6 * s), pt(x + 16.4 * s, y + 13.6 * s), 1.8 * s, fg);
        }
    }
}
