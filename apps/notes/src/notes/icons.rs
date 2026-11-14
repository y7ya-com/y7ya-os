//! SF Symbols–style line icons drawn with rounded strokes. Each draws into a
//! square box centered in the given bounds.

use ui::{Bounds, Color, Scene, pt};

const STROKE: f32 = 1.5;

fn frame(b: Bounds, size: f32) -> (f32, f32, f32) {
    let c = b.center();
    (c.x - size / 2.0, c.y - size / 2.0, size / 18.0)
}

fn outline(scene: &mut Scene, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color, stroke: f32) {
    scene.rect_bordered(Bounds::new(x, y, w, h), Color::TRANSPARENT, r, color, stroke);
}

pub fn sidebar(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 1.0 * s, y + 3.0 * s, 16.0 * s, 12.5 * s, 3.0 * s, color, STROKE);
    scene.segment(pt(x + 6.5 * s, y + 3.5 * s), pt(x + 6.5 * s, y + 15.0 * s), STROKE, color);
    for i in 0..3 {
        let yy = y + (6.5 + i as f32 * 2.3) * s;
        scene.segment(pt(x + 3.3 * s, yy), pt(x + 4.6 * s, yy), 1.2, color);
    }
}

pub fn trash(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 2.5 * s, y + 4.5 * s), pt(x + 15.5 * s, y + 4.5 * s), STROKE, color);
    scene.segment(pt(x + 6.8 * s, y + 4.5 * s), pt(x + 7.2 * s, y + 2.2 * s), STROKE, color);
    scene.segment(pt(x + 7.2 * s, y + 2.2 * s), pt(x + 10.8 * s, y + 2.2 * s), STROKE, color);
    scene.segment(pt(x + 10.8 * s, y + 2.2 * s), pt(x + 11.2 * s, y + 4.5 * s), STROKE, color);
    // Tapered can.
    scene.segment(pt(x + 4.2 * s, y + 4.8 * s), pt(x + 5.0 * s, y + 15.6 * s), STROKE, color);
    scene.segment(pt(x + 13.8 * s, y + 4.8 * s), pt(x + 13.0 * s, y + 15.6 * s), STROKE, color);
    scene.segment(pt(x + 5.0 * s, y + 15.6 * s), pt(x + 13.0 * s, y + 15.6 * s), STROKE, color);
    scene.segment(pt(x + 7.4 * s, y + 7.2 * s), pt(x + 7.6 * s, y + 13.2 * s), 1.2, color);
    scene.segment(pt(x + 10.6 * s, y + 7.2 * s), pt(x + 10.4 * s, y + 13.2 * s), 1.2, color);
}

pub fn compose(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    // Open square: left, bottom, right (lower part) and top (left part).
    let (l, t, r, bo) = (x + 1.5 * s, y + 3.0 * s, x + 14.5 * s, y + 16.0 * s);
    scene.segment(pt(l + 2.5 * s, t), pt(x + 9.0 * s, t), STROKE, color);
    scene.segment(pt(l, t + 2.5 * s), pt(l, bo - 2.5 * s), STROKE, color);
    scene.segment(pt(l + 2.5 * s, bo), pt(r - 2.5 * s, bo), STROKE, color);
    scene.segment(pt(r, bo - 2.5 * s), pt(r, y + 9.5 * s), STROKE, color);
    for (cx, cy, from, to) in [(l + 2.5 * s, t + 2.5 * s, 180.0, 270.0), (l + 2.5 * s, bo - 2.5 * s, 90.0, 180.0), (r - 2.5 * s, bo - 2.5 * s, 0.0, 90.0)] {
        arc(scene, cx, cy, 2.5 * s, from, to, color, STROKE);
    }
    // Pencil.
    scene.segment(pt(x + 7.5 * s, y + 10.5 * s), pt(x + 15.8 * s, y + 2.2 * s), 2.6 * s, color);
    scene.segment(pt(x + 6.6 * s, y + 11.4 * s), pt(x + 7.4 * s, y + 10.6 * s), 1.2, color);
}

pub fn magnifier(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 14.0);
    let r = 5.0 * s;
    outline(scene, x + 1.0 * s, y + 1.0 * s, 2.0 * r, 2.0 * r, r, color, STROKE);
    scene.segment(pt(x + 9.6 * s, y + 9.6 * s), pt(x + 13.5 * s, y + 13.5 * s), 1.8, color);
}

/// Fills its box (18pt in a 16pt sidebar slot, smaller next to captions).
pub fn folder(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, b.w.min(b.h) * 18.0 / 16.0);
    outline(scene, x + 1.5 * s, y + 5.5 * s, 15.0 * s, 10.5 * s, 2.2 * s, color, 1.4);
    // Tab on top-left.
    scene.segment(pt(x + 2.2 * s, y + 5.5 * s), pt(x + 2.2 * s, y + 4.4 * s), 1.4, color);
    scene.segment(pt(x + 2.6 * s, y + 3.4 * s), pt(x + 6.6 * s, y + 3.4 * s), 1.4, color);
    scene.segment(pt(x + 6.6 * s, y + 3.4 * s), pt(x + 8.0 * s, y + 5.2 * s), 1.4, color);
    arc(scene, x + 3.4 * s, y + 4.4 * s, 1.0 * s, 180.0, 270.0, color, 1.4);
}

pub fn plus_circle(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 16.0);
    outline(scene, x + 1.0 * s, y + 1.0 * s, 14.0 * s, 14.0 * s, 7.0 * s, color, 1.3);
    scene.segment(pt(x + 8.0 * s, y + 4.8 * s), pt(x + 8.0 * s, y + 11.2 * s), 1.4, color);
    scene.segment(pt(x + 4.8 * s, y + 8.0 * s), pt(x + 11.2 * s, y + 8.0 * s), 1.4, color);
}

/// A filled push pin (SF Symbols' "pin.fill"), for the "Pinned" header.
pub fn pin(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 14.0);
    let r = |l: f32, t: f32, w: f32, h: f32, rad: f32| (Bounds::new(x + l * s, y + t * s, w * s, h * s), rad * s);
    for (rect, rad) in [r(4.5, 1.5, 9.0, 2.6, 1.3), r(6.0, 3.6, 6.0, 5.0, 0.8), r(3.4, 8.0, 11.2, 2.8, 1.4)] {
        scene.rect(rect, color, rad);
    }
    // The needle.
    scene.segment(pt(x + 9.0 * s, y + 10.4 * s), pt(x + 9.0 * s, y + 16.5 * s), 1.3, color);
}

pub fn xmark(scene: &mut Scene, b: Bounds, color: Color, size: f32, stroke: f32) {
    let c = b.center();
    let h = size / 2.0;
    scene.segment(pt(c.x - h, c.y - h), pt(c.x + h, c.y + h), stroke, color);
    scene.segment(pt(c.x - h, c.y + h), pt(c.x + h, c.y - h), stroke, color);
}

fn arc(scene: &mut Scene, cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32, color: Color, stroke: f32) {
    let steps = 6;
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
