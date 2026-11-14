//! SF Symbols–style line icons for the navigation rail and rows, drawn with
//! rounded strokes into an 18-unit box centered in the bounds.

use ui::{Bounds, Color, Scene, pt};

const STROKE: f32 = 1.5;

fn frame(b: Bounds, size: f32) -> (f32, f32, f32) {
    let c = b.center();
    (c.x - size / 2.0, c.y - size / 2.0, size / 18.0)
}

fn outline(scene: &mut Scene, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color, stroke: f32) {
    scene.rect_bordered(Bounds::new(x, y, w, h), Color::TRANSPARENT, r, color, stroke);
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

/// Four tiles: running processes.
pub fn processes(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    for (i, j) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        outline(scene, x + (2.0 + i * 7.6) * s, y + (2.0 + j * 7.6) * s, 6.4 * s, 6.4 * s, 1.8 * s, color, 1.4);
    }
}

/// A pulse line in a rounded frame: performance.
pub fn performance(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 1.5 * s, y + 2.5 * s, 15.0 * s, 13.0 * s, 3.0 * s, color, 1.4);
    let pts = [(3.5, 10.0), (6.5, 10.0), (8.0, 6.0), (10.0, 13.0), (11.5, 9.0), (14.5, 9.0)];
    for w in pts.windows(2) {
        scene.segment(pt(x + w[0].0 * s, y + w[0].1 * s), pt(x + w[1].0 * s, y + w[1].1 * s), 1.4, color);
    }
}

/// A gauge: startup apps.
pub fn startup(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    arc(scene, x + 9.0 * s, y + 11.0 * s, 7.5 * s, 160.0, 380.0, color, STROKE);
    scene.segment(pt(x + 9.0 * s, y + 11.0 * s), pt(x + 12.5 * s, y + 6.5 * s), STROKE, color);
    scene.rect(Bounds::new(x + 7.6 * s, y + 9.6 * s, 2.8 * s, 2.8 * s), color, 1.4 * s);
}

/// Two people: users.
pub fn users(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    outline(scene, x + 4.0 * s, y + 2.5 * s, 6.0 * s, 6.0 * s, 3.0 * s, color, STROKE);
    arc(scene, x + 7.0 * s, y + 16.0 * s, 5.5 * s, 180.0, 360.0, color, STROKE);
    arc(scene, x + 12.8 * s, y + 6.2 * s, 2.4 * s, 250.0, 470.0, color, 1.3);
    arc(scene, x + 13.0 * s, y + 15.0 * s, 4.0 * s, 250.0, 360.0, color, 1.3);
}

/// A bulleted list: details.
pub fn details(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    for i in 0..3 {
        let yy = y + (4.5 + i as f32 * 4.5) * s;
        scene.rect(Bounds::new(x + 2.0 * s, yy - 1.0 * s, 2.0 * s, 2.0 * s), color, 1.0 * s);
        scene.segment(pt(x + 6.5 * s, yy), pt(x + 16.0 * s, yy), STROKE, color);
    }
}

/// A gear: services.
pub fn services(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 18.0);
    let (cx, cy) = (x + 9.0 * s, y + 9.0 * s);
    outline(scene, cx - 5.0 * s, cy - 5.0 * s, 10.0 * s, 10.0 * s, 5.0 * s, color, STROKE);
    outline(scene, cx - 2.0 * s, cy - 2.0 * s, 4.0 * s, 4.0 * s, 2.0 * s, color, 1.3);
    for i in 0..8 {
        let a = (i as f32 * 45.0).to_radians();
        let (c, sn) = (a.cos(), a.sin());
        scene.segment(pt(cx + c * 5.6 * s, cy + sn * 5.6 * s), pt(cx + c * 7.6 * s, cy + sn * 7.6 * s), 2.2, color);
    }
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

pub fn magnifier(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 14.0);
    let r = 5.0 * s;
    outline(scene, x + 1.0 * s, y + 1.0 * s, 2.0 * r, 2.0 * r, r, color, STROKE);
    scene.segment(pt(x + 9.6 * s, y + 9.6 * s), pt(x + 13.5 * s, y + 13.5 * s), 1.8, color);
}

pub fn xmark(scene: &mut Scene, b: Bounds, color: Color, size: f32, stroke: f32) {
    let c = b.center();
    let h = size / 2.0;
    scene.segment(pt(c.x - h, c.y - h), pt(c.x + h, c.y + h), stroke, color);
    scene.segment(pt(c.x - h, c.y + h), pt(c.x + h, c.y - h), stroke, color);
}

/// Disclosure chevron: pointing right when collapsed, down when expanded.
pub fn disclosure(scene: &mut Scene, b: Bounds, color: Color, open: bool) {
    let c = b.center();
    let (a, l) = (2.0, 3.6);
    if open {
        scene.segment(pt(c.x - l, c.y - a), pt(c.x, c.y + a), 1.5, color);
        scene.segment(pt(c.x, c.y + a), pt(c.x + l, c.y - a), 1.5, color);
    } else {
        scene.segment(pt(c.x - a, c.y - l), pt(c.x + a, c.y), 1.5, color);
        scene.segment(pt(c.x + a, c.y), pt(c.x - a, c.y + l), 1.5, color);
    }
}

/// A generic program: a small window outline.
pub fn program(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 16.0);
    outline(scene, x + 1.5 * s, y + 2.5 * s, 15.0 * s, 13.0 * s, 2.5 * s, color, 1.2);
    scene.segment(pt(x + 2.0 * s, y + 6.0 * s), pt(x + 16.0 * s, y + 6.0 * s), 1.1, color);
}

/// A person, for user rows.
pub fn person(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s) = frame(b, 16.0);
    outline(scene, x + 6.0 * s, y + 2.0 * s, 6.0 * s, 6.0 * s, 3.0 * s, color, 1.3);
    arc(scene, x + 9.0 * s, y + 16.5 * s, 6.0 * s, 180.0, 360.0, color, 1.3);
}
