//! Toolbar line icons, SF Symbols style, on an 18-unit grid scaled to their box.

use ui::{Bounds, Color, Scene, pt};

const STROKE: f32 = 1.5;

fn frame(b: Bounds) -> (f32, f32, f32, f32) {
    let size = b.w.min(b.h);
    let c = b.center();
    let s = size / 18.0;
    (c.x - size / 2.0, c.y - size / 2.0, s, STROKE * s.max(1.0))
}

fn poly(scene: &mut Scene, pts: &[(f32, f32)], x: f32, y: f32, s: f32, w: f32, color: Color) {
    for p in pts.windows(2) {
        scene.segment(pt(x + p[0].0 * s, y + p[0].1 * s), pt(x + p[1].0 * s, y + p[1].1 * s), w, color);
    }
}

fn circle(scene: &mut Scene, cx: f32, cy: f32, r: f32, color: Color, w: f32) {
    scene.rect_bordered(Bounds::new(cx - r, cy - r, 2.0 * r, 2.0 * r), Color::TRANSPARENT, r, color, w);
}

fn arc(scene: &mut Scene, cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32, color: Color, stroke: f32) {
    let steps = ((to_deg - from_deg).abs() / 15.0).ceil().max(2.0) as usize;
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

pub fn sidebar(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    scene.rect_bordered(Bounds::new(x + 1.0 * s, y + 3.0 * s, 16.0 * s, 12.5 * s), Color::TRANSPARENT, 3.0 * s, color, w);
    poly(scene, &[(6.5, 3.5), (6.5, 15.0)], x, y, s, w, color);
    for i in 0..3 {
        let yy = 6.5 + i as f32 * 2.3;
        poly(scene, &[(3.3, yy), (4.6, yy)], x, y, s, w * 0.8, color);
    }
}

pub fn zoom_in(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    circle(scene, x + 7.6 * s, y + 7.6 * s, 5.8 * s, color, w);
    poly(scene, &[(11.8, 11.8), (16.2, 16.2)], x, y, s, w * 1.2, color);
    poly(scene, &[(7.6, 5.0), (7.6, 10.2)], x, y, s, w, color);
    poly(scene, &[(5.0, 7.6), (10.2, 7.6)], x, y, s, w, color);
}

pub fn zoom_out(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    circle(scene, x + 7.6 * s, y + 7.6 * s, 5.8 * s, color, w);
    poly(scene, &[(11.8, 11.8), (16.2, 16.2)], x, y, s, w * 1.2, color);
    poly(scene, &[(5.0, 7.6), (10.2, 7.6)], x, y, s, w, color);
}

/// A rounded arrow turning counter-clockwise (rotate left).
pub fn rotate_left(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    scene.rect_bordered(Bounds::new(x + 6.0 * s, y + 7.0 * s, 9.5 * s, 9.0 * s), Color::TRANSPARENT, 1.8 * s, color, w);
    arc(scene, x + 9.0 * s, y + 7.0 * s, 4.6 * s, 180.0, 270.0, color, w);
    poly(scene, &[(9.0, 2.4), (11.0, 2.4)], x, y, s, w, color);
    poly(scene, &[(4.4, 4.8), (4.4, 7.4), (2.0, 5.4)], x, y, s, w, color);
}

pub fn rotate_right(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    scene.rect_bordered(Bounds::new(x + 2.5 * s, y + 7.0 * s, 9.5 * s, 9.0 * s), Color::TRANSPARENT, 1.8 * s, color, w);
    arc(scene, x + 9.0 * s, y + 7.0 * s, 4.6 * s, 270.0, 360.0, color, w);
    poly(scene, &[(7.0, 2.4), (9.0, 2.4)], x, y, s, w, color);
    poly(scene, &[(13.6, 4.8), (13.6, 7.4), (16.0, 5.4)], x, y, s, w, color);
}

/// Arrows pointing out to the corners: zoom to fit.
pub fn fit(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    for (cx, cy, dx, dy) in [(2.5, 2.5, 1.0, 1.0), (15.5, 2.5, -1.0, 1.0), (2.5, 15.5, 1.0, -1.0), (15.5, 15.5, -1.0, -1.0)] {
        poly(scene, &[(cx + 4.2 * dx, cy), (cx, cy), (cx, cy + 4.2 * dy)], x, y, s, w, color);
    }
}

pub fn chevron_left(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(11.2, 3.6), (5.8, 9.0), (11.2, 14.4)], x, y, s, w * 1.1, color);
}

pub fn chevron_right(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(6.8, 3.6), (12.2, 9.0), (6.8, 14.4)], x, y, s, w * 1.1, color);
}

/// A page with a warning mark, for files that can't be shown.
pub fn broken_page(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(10.6, 1.8), (4.2, 1.8), (4.2, 16.2), (13.8, 16.2), (13.8, 5.0), (10.6, 1.8), (10.6, 5.0), (13.8, 5.0)], x, y, s, w * 0.7, color);
    poly(scene, &[(9.0, 7.0), (9.0, 11.0)], x, y, s, w, color);
    scene.rect(Bounds::new(x + 8.3 * s, y + 12.6 * s, 1.4 * s, 1.4 * s), color, 0.7 * s);
}
