//! Glyphs drawn for keys that have no good text form.

use ui::{Bounds, Color, Scene, pt, rgba};

/// "+/−" with a slash, as on the ± key.
pub fn plus_minus(scene: &mut Scene, b: Bounds, color: Color) {
    let s = b.w / 24.0;
    let (x, y) = (b.x, b.y);
    let stroke = 1.9 * s;
    scene.segment(pt(x + 3.0 * s, y + 7.0 * s), pt(x + 11.0 * s, y + 7.0 * s), stroke, color);
    scene.segment(pt(x + 7.0 * s, y + 3.0 * s), pt(x + 7.0 * s, y + 11.0 * s), stroke, color);
    scene.segment(pt(x + 16.5 * s, y + 3.0 * s), pt(x + 7.5 * s, y + 21.0 * s), 1.5 * s, color);
    scene.segment(pt(x + 13.0 * s, y + 17.0 * s), pt(x + 21.0 * s, y + 17.0 * s), stroke, color);
}

/// A small calculator: the mode button.
pub fn calculator(scene: &mut Scene, b: Bounds) {
    let s = b.w / 24.0;
    let color = rgba(0xffffff, 0.95);
    let body = Bounds::new(b.x + 5.0 * s, b.y + 2.0 * s, 14.0 * s, 20.0 * s);
    scene.rect_bordered(body, Color::TRANSPARENT, 3.0 * s, color, 1.6 * s);
    scene.rect(Bounds::new(body.x + 3.0 * s, body.y + 3.0 * s, 8.0 * s, 3.5 * s), color, 1.0 * s);
    for row in 0..3 {
        for col in 0..3 {
            let c = pt(body.x + (3.8 + col as f32 * 3.2) * s, body.y + (10.0 + row as f32 * 3.2) * s);
            scene.rect(Bounds::new(c.x - 1.0 * s, c.y - 1.0 * s, 2.0 * s, 2.0 * s), color, 1.0 * s);
        }
    }
}
