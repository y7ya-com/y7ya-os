//! SF Symbols–style icons drawn with rounded strokes. Each draws into a
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

fn circle(scene: &mut Scene, cx: f32, cy: f32, r: f32, color: Color) {
    scene.rect(Bounds::new(cx - r, cy - r, 2.0 * r, 2.0 * r), color, r);
}

pub fn magnifier(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 14.0);
    let r = 5.0 * s;
    outline(scene, x + 1.0 * s, y + 1.0 * s, 2.0 * r, 2.0 * r, r, color, STROKE);
    scene.segment(pt(x + 9.6 * s, y + 9.6 * s), pt(x + 13.5 * s, y + 13.5 * s), 1.8, color);
}

pub fn plus(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 9.0 * s, y + 3.0 * s), pt(x + 9.0 * s, y + 15.0 * s), 1.6, color);
    scene.segment(pt(x + 3.0 * s, y + 9.0 * s), pt(x + 15.0 * s, y + 9.0 * s), 1.6, color);
}

pub fn plus_circle(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 16.0);
    outline(scene, x + 1.0 * s, y + 1.0 * s, 14.0 * s, 14.0 * s, 7.0 * s, color, 1.3);
    scene.segment(pt(x + 8.0 * s, y + 4.8 * s), pt(x + 8.0 * s, y + 11.2 * s), 1.4, color);
    scene.segment(pt(x + 4.8 * s, y + 8.0 * s), pt(x + 11.2 * s, y + 8.0 * s), 1.4, color);
}

/// The circled "i" that opens a reminder's details.
pub fn info(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 1.5 * s, y + 1.5 * s, 15.0 * s, 15.0 * s, 7.5 * s, color, 1.3);
    circle(scene, x + 9.0 * s, y + 5.6 * s, 1.0 * s, color);
    scene.segment(pt(x + 9.0 * s, y + 8.2 * s), pt(x + 9.0 * s, y + 12.8 * s), 1.6, color);
}

/// A flag: pole and a filled pennant.
pub fn flag(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 4.0 * s, y + 2.5 * s), pt(x + 4.0 * s, y + 16.0 * s), 1.6 * s.max(1.0), color);
    // The pennant, a slightly wavy band.
    let (top, bottom) = (y + 3.0 * s, y + 10.5 * s);
    let steps = 10;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let xx = x + 4.6 * s + t * 10.0 * s;
        let wave = (t * std::f32::consts::PI * 2.0).sin() * 0.8 * s;
        scene.segment(pt(xx, top + wave), pt(xx, bottom + wave), 1.4 * s, color);
    }
}

/// Today: a calendar page.
pub fn calendar(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 2.5 * s, y + 3.5 * s, 13.0 * s, 12.0 * s, 2.2 * s, color, 1.4);
    scene.rect(Bounds::new(x + 2.5 * s, y + 3.5 * s, 13.0 * s, 3.2 * s), color, 2.0 * s);
    for (i, j) in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1)] {
        circle(scene, x + (6.0 + i as f32 * 3.0) * s, y + (9.6 + j as f32 * 3.0) * s, 0.8 * s, color);
    }
}

/// Scheduled: a calendar page with a clock face.
pub fn scheduled(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 2.5 * s, y + 3.5 * s, 13.0 * s, 12.0 * s, 2.2 * s, color, 1.4);
    scene.rect(Bounds::new(x + 2.5 * s, y + 3.5 * s, 13.0 * s, 3.2 * s), color, 2.0 * s);
    scene.segment(pt(x + 9.0 * s, y + 8.6 * s), pt(x + 9.0 * s, y + 11.4 * s), 1.4, color);
    scene.segment(pt(x + 9.0 * s, y + 11.4 * s), pt(x + 11.2 * s, y + 12.6 * s), 1.4, color);
}

/// All: an inbox tray.
pub fn tray(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 2.0 * s, y + 3.5 * s, 14.0 * s, 11.5 * s, 2.4 * s, color, 1.4);
    scene.segment(pt(x + 2.4 * s, y + 10.0 * s), pt(x + 6.2 * s, y + 10.0 * s), 1.4, color);
    scene.segment(pt(x + 6.2 * s, y + 10.0 * s), pt(x + 7.2 * s, y + 11.8 * s), 1.4, color);
    scene.segment(pt(x + 7.2 * s, y + 11.8 * s), pt(x + 10.8 * s, y + 11.8 * s), 1.4, color);
    scene.segment(pt(x + 10.8 * s, y + 11.8 * s), pt(x + 11.8 * s, y + 10.0 * s), 1.4, color);
    scene.segment(pt(x + 11.8 * s, y + 10.0 * s), pt(x + 15.6 * s, y + 10.0 * s), 1.4, color);
}

pub fn check(scene: &mut Scene, b: Bounds, color: Color, stroke: f32) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 3.5 * s, y + 9.5 * s), pt(x + 7.5 * s, y + 13.5 * s), stroke, color);
    scene.segment(pt(x + 7.5 * s, y + 13.5 * s), pt(x + 14.5 * s, y + 4.5 * s), stroke, color);
}

/// My Lists' glyph: three bulleted lines.
pub fn bullets(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, b.w.min(b.h) * 18.0 / 14.0);
    for i in 0..3 {
        let yy = y + (5.0 + i as f32 * 4.0) * s;
        circle(scene, x + 4.5 * s, yy, 1.2 * s, color);
        scene.segment(pt(x + 7.5 * s, yy), pt(x + 14.5 * s, yy), 1.5 * s, color);
    }
}

/// The circle in front of a reminder: an empty ring, or (done) the ring with
/// a filled dot inside, as Reminders draws it.
pub fn completion(scene: &mut Scene, b: Bounds, ring: Color, fill: Color, done: bool) {
    let c = b.center();
    let r = 9.0;
    if done {
        scene.rect_bordered(Bounds::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r), Color::TRANSPARENT, r, fill, 1.5);
        circle(scene, c.x, c.y, r - 3.4, fill);
    } else {
        scene.rect_bordered(Bounds::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r), Color::TRANSPARENT, r, ring, 1.3);
    }
}

pub fn chevron_left(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    scene.segment(pt(c.x + 2.0, c.y - 4.5), pt(c.x - 2.5, c.y), 1.6, color);
    scene.segment(pt(c.x - 2.5, c.y), pt(c.x + 2.0, c.y + 4.5), 1.6, color);
}

pub fn chevron_right(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    scene.segment(pt(c.x - 2.0, c.y - 4.5), pt(c.x + 2.5, c.y), 1.6, color);
    scene.segment(pt(c.x + 2.5, c.y), pt(c.x - 2.0, c.y + 4.5), 1.6, color);
}

/// "Show completed": a circle with a check.
pub fn check_circle(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 1.5 * s, y + 1.5 * s, 15.0 * s, 15.0 * s, 7.5 * s, color, 1.4);
    scene.segment(pt(x + 5.5 * s, y + 9.2 * s), pt(x + 8.0 * s, y + 11.8 * s), 1.5, color);
    scene.segment(pt(x + 8.0 * s, y + 11.8 * s), pt(x + 12.6 * s, y + 6.4 * s), 1.5, color);
}

pub fn xmark(scene: &mut Scene, b: Bounds, color: Color, size: f32, stroke: f32) {
    let c = b.center();
    let h = size / 2.0;
    scene.segment(pt(c.x - h, c.y - h), pt(c.x + h, c.y + h), stroke, color);
    scene.segment(pt(c.x - h, c.y + h), pt(c.x + h, c.y - h), stroke, color);
}
