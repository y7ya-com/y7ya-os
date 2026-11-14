//! SF Symbols–style line icons drawn with rounded strokes. Each draws into a
//! square box centered in the given bounds.

use ui::{Bounds, Color, Scene, pt};

const STROKE: f32 = 1.5;

fn frame(b: Bounds, size: f32) -> (f32, f32, f32) {
    let c = b.center();
    (c.x - size / 2.0, c.y - size / 2.0, size / 18.0)
}

pub fn magnifier(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 14.0);
    let r = 5.0 * s;
    scene.rect_bordered(Bounds::new(x + 1.0 * s, y + 1.0 * s, 2.0 * r, 2.0 * r), Color::TRANSPARENT, r, color, STROKE);
    scene.segment(pt(x + 9.6 * s, y + 9.6 * s), pt(x + 13.5 * s, y + 13.5 * s), 1.8, color);
}

pub fn chevron_left(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 14.0);
    scene.segment(pt(x + 9.0 * s, y + 3.0 * s), pt(x + 4.0 * s, y + 9.0 * s), STROKE, color);
    scene.segment(pt(x + 4.0 * s, y + 9.0 * s), pt(x + 9.0 * s, y + 15.0 * s), STROKE, color);
}

pub fn chevron_right(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 14.0);
    scene.segment(pt(x + 5.0 * s, y + 3.0 * s), pt(x + 10.0 * s, y + 9.0 * s), STROKE, color);
    scene.segment(pt(x + 10.0 * s, y + 9.0 * s), pt(x + 5.0 * s, y + 15.0 * s), STROKE, color);
}

/// Lines with a return arrow: wrap text to the window.
pub fn wrap(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    scene.segment(pt(x + 2.0 * s, y + 4.0 * s), pt(x + 16.0 * s, y + 4.0 * s), STROKE, color);
    scene.segment(pt(x + 2.0 * s, y + 9.0 * s), pt(x + 13.0 * s, y + 9.0 * s), STROKE, color);
    // The arrow curls from the end of the middle line back to the left.
    let pts = [(13.0, 9.0), (15.0, 9.4), (16.2, 11.2), (15.8, 13.0), (14.0, 14.0), (9.0, 14.0)];
    for w in pts.windows(2) {
        scene.segment(pt(x + w[0].0 * s, y + w[0].1 * s), pt(x + w[1].0 * s, y + w[1].1 * s), STROKE, color);
    }
    scene.segment(pt(x + 9.0 * s, y + 14.0 * s), pt(x + 11.2 * s, y + 11.8 * s), STROKE, color);
    scene.segment(pt(x + 9.0 * s, y + 14.0 * s), pt(x + 11.2 * s, y + 16.2 * s), STROKE, color);
    scene.segment(pt(x + 2.0 * s, y + 14.0 * s), pt(x + 5.5 * s, y + 14.0 * s), STROKE, color);
}

pub fn folder(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 16.0);
    scene.rect_bordered(Bounds::new(x + 1.5 * s, y + 5.5 * s, 15.0 * s, 10.5 * s), Color::TRANSPARENT, 2.2 * s, color, 1.3);
    scene.segment(pt(x + 2.2 * s, y + 5.5 * s), pt(x + 2.2 * s, y + 4.0 * s), 1.3, color);
    scene.segment(pt(x + 2.6 * s, y + 3.4 * s), pt(x + 6.6 * s, y + 3.4 * s), 1.3, color);
    scene.segment(pt(x + 6.6 * s, y + 3.4 * s), pt(x + 8.0 * s, y + 5.2 * s), 1.3, color);
}

pub fn document(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 16.0);
    let (l, t, r, bo) = (x + 3.5 * s, y + 1.5 * s, x + 14.5 * s, y + 16.5 * s);
    scene.segment(pt(l, t), pt(r - 4.0 * s, t), 1.3, color);
    scene.segment(pt(r - 4.0 * s, t), pt(r, t + 4.0 * s), 1.3, color);
    scene.segment(pt(r, t + 4.0 * s), pt(r, bo), 1.3, color);
    scene.segment(pt(r, bo), pt(l, bo), 1.3, color);
    scene.segment(pt(l, bo), pt(l, t), 1.3, color);
}

pub fn check(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 12.0);
    scene.segment(pt(x + 2.5 * s, y + 9.5 * s), pt(x + 7.0 * s, y + 14.0 * s), 1.6, color);
    scene.segment(pt(x + 7.0 * s, y + 14.0 * s), pt(x + 15.5 * s, y + 4.0 * s), 1.6, color);
}
