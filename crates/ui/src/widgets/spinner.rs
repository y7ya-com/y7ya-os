//! The macOS activity indicator: spokes around a circle, fading behind the
//! leading one. It only moves while the app keeps asking for frames (return
//! `now + SPINNER_FRAME` from `App::next_wake` while something is busy), so an
//! idle app stays idle.

use std::time::{Duration, Instant};

use crate::element::{Canvas, Styled, canvas};
use crate::geom::{Color, pt};

const SPOKES: usize = 8;

/// How often to redraw a visible spinner (one spoke step).
pub const SPINNER_FRAME: Duration = Duration::from_millis(100);

/// The spinner's phase (0–1) for an activity that started at `since`.
pub fn spinner_phase(since: Instant) -> f32 {
    let step = (since.elapsed().as_millis() / SPINNER_FRAME.as_millis()) as usize;
    (step % SPOKES) as f32 / SPOKES as f32
}

/// A `size`-point spinner in `color` at `phase` (see [`spinner_phase`]).
pub fn spinner(size: f32, phase: f32, color: Color) -> Canvas {
    canvas(move |scene, b| {
        let c = b.center();
        let r = b.w.min(b.h) / 2.0;
        let lead = (phase * SPOKES as f32).round() as usize % SPOKES;
        let thickness = (r * 0.24).max(1.2);
        for i in 0..SPOKES {
            let a = (i as f32 / SPOKES as f32) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let (s, cs) = a.sin_cos();
            // Brightest at the leading spoke, fading over the ones behind it.
            let behind = (lead + SPOKES - i) % SPOKES;
            let alpha = 1.0 - behind as f32 / SPOKES as f32 * 0.8;
            let inner = pt(c.x + cs * r * 0.42, c.y + s * r * 0.42);
            let outer = pt(c.x + cs * (r - thickness / 2.0), c.y + s * (r - thickness / 2.0));
            scene.segment(inner, outer, thickness, color.alpha(alpha));
        }
    })
    .size(size)
}
