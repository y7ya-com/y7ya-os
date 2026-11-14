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

pub fn plus(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 9.0 * s, y + 3.0 * s), pt(x + 9.0 * s, y + 15.0 * s), 1.6, color);
    scene.segment(pt(x + 3.0 * s, y + 9.0 * s), pt(x + 15.0 * s, y + 9.0 * s), 1.6, color);
}

pub fn chevron_left(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 11.0 * s, y + 3.5 * s), pt(x + 5.5 * s, y + 9.0 * s), 1.6, color);
    scene.segment(pt(x + 5.5 * s, y + 9.0 * s), pt(x + 11.0 * s, y + 14.5 * s), 1.6, color);
}

pub fn chevron_right(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 7.0 * s, y + 3.5 * s), pt(x + 12.5 * s, y + 9.0 * s), 1.6, color);
    scene.segment(pt(x + 12.5 * s, y + 9.0 * s), pt(x + 7.0 * s, y + 14.5 * s), 1.6, color);
}

/// The small up/down chevrons on pop-up buttons.
pub fn updown(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    scene.segment(pt(c.x - 2.5, c.y - 1.5), pt(c.x, c.y - 4.0), 1.2, color);
    scene.segment(pt(c.x, c.y - 4.0), pt(c.x + 2.5, c.y - 1.5), 1.2, color);
    scene.segment(pt(c.x - 2.5, c.y + 1.5), pt(c.x, c.y + 4.0), 1.2, color);
    scene.segment(pt(c.x, c.y + 4.0), pt(c.x + 2.5, c.y + 1.5), 1.2, color);
}

pub fn check(scene: &mut Scene, b: Bounds, color: Color, stroke: f32) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 3.5 * s, y + 9.5 * s), pt(x + 7.5 * s, y + 13.5 * s), stroke, color);
    scene.segment(pt(x + 7.5 * s, y + 13.5 * s), pt(x + 14.5 * s, y + 4.5 * s), stroke, color);
}

/// Repeat arrows (circular), shown on repeating events.
pub fn repeat(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let r = b.w.min(b.h) * 0.36;
    let steps = 10;
    let mut prev = None;
    for i in 0..=steps {
        let a = (30.0 + 300.0 * i as f32 / steps as f32).to_radians();
        let p = pt(c.x + r * a.cos(), c.y + r * a.sin());
        if let Some(q) = prev {
            scene.segment(q, p, 1.1, color);
        }
        prev = Some(p);
    }
    let tip = pt(c.x + r * 30f32.to_radians().cos(), c.y + r * 30f32.to_radians().sin());
    scene.segment(tip, pt(tip.x + 2.4, tip.y - 0.6), 1.1, color);
    scene.segment(tip, pt(tip.x - 0.4, tip.y - 2.4), 1.1, color);
}
