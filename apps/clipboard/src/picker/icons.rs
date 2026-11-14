//! SF Symbols–style line icons for the picker.

use ui::{Bounds, Color, Scene, pt};

fn frame(b: Bounds, size: f32) -> (f32, f32, f32) {
    let c = b.center();
    (c.x - size / 2.0, c.y - size / 2.0, size / 18.0)
}

fn outline(scene: &mut Scene, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color, stroke: f32) {
    scene.rect_bordered(Bounds::new(x, y, w, h), Color::TRANSPARENT, r, color, stroke);
}

pub fn magnifier(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    let r = 6.2 * s;
    outline(scene, x + 1.5 * s, y + 1.5 * s, 2.0 * r, 2.0 * r, r, color, 1.8);
    scene.segment(pt(x + 11.6 * s, y + 11.6 * s), pt(x + 16.5 * s, y + 16.5 * s), 2.2, color);
}

/// A page with lines of text.
pub fn document(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 3.5 * s, y + 1.5 * s, 11.0 * s, 15.0 * s, 2.0 * s, color, 1.3);
    for (i, w) in [6.0, 6.0, 4.0].iter().enumerate() {
        let yy = y + (6.0 + i as f32 * 3.0) * s;
        scene.segment(pt(x + 6.0 * s, yy), pt(x + (6.0 + w) * s, yy), 1.2, color);
    }
}

/// A pushpin.
pub fn pin(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 12.0);
    scene.segment(pt(x + 4.0 * s, y + 2.0 * s), pt(x + 8.0 * s, y + 2.0 * s), 1.4, color);
    scene.segment(pt(x + 4.6 * s, y + 2.0 * s), pt(x + 4.0 * s, y + 6.5 * s), 1.4, color);
    scene.segment(pt(x + 7.4 * s, y + 2.0 * s), pt(x + 8.0 * s, y + 6.5 * s), 1.4, color);
    scene.segment(pt(x + 2.5 * s, y + 6.8 * s), pt(x + 9.5 * s, y + 6.8 * s), 1.4, color);
    scene.segment(pt(x + 6.0 * s, y + 6.8 * s), pt(x + 6.0 * s, y + 11.0 * s), 1.2, color);
}

/// Stacked pages: the empty-history illustration.
pub fn clipboard(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 3.0 * s, y + 3.0 * s, 12.0 * s, 14.0 * s, 2.0 * s, color, 1.4);
    outline(scene, x + 6.5 * s, y + 1.5 * s, 5.0 * s, 3.0 * s, 1.0 * s, color, 1.3);
    scene.segment(pt(x + 6.0 * s, y + 9.0 * s), pt(x + 12.0 * s, y + 9.0 * s), 1.2, color);
    scene.segment(pt(x + 6.0 * s, y + 12.0 * s), pt(x + 10.5 * s, y + 12.0 * s), 1.2, color);
}
