//! Time-series graphs (live charts and sparklines) drawn with scene
//! primitives: a filled area under a polyline, on an optional grid.
//!
//! Values are oldest first. The newest sample sits at the right edge and the
//! graph scrolls left as samples arrive, so a series shorter than `capacity`
//! only covers the right part of the graph.

use crate::element::{Canvas, canvas};
use crate::geom::{Bounds, Color, pt};
use crate::scene::Scene;

#[derive(Clone)]
pub struct Series {
    pub values: Vec<f32>,
    pub line: Color,
    /// Area fill under the line.
    pub fill: Option<Color>,
    pub dashed: bool,
}

impl Series {
    pub fn new(values: Vec<f32>, line: Color) -> Self {
        Self { values, line, fill: None, dashed: false }
    }

    pub fn fill(mut self, c: Color) -> Self {
        self.fill = Some(c);
        self
    }

    pub fn dashed(mut self) -> Self {
        self.dashed = true;
        self
    }
}

#[derive(Clone)]
pub struct Graph {
    pub series: Vec<Series>,
    /// Number of samples the full width represents.
    pub capacity: usize,
    /// The value at the top edge; values above it are clipped.
    pub max: f32,
    /// Grid cells across and down, or (0, 0) for none.
    pub grid: (usize, usize),
    pub grid_color: Color,
    pub border: Option<Color>,
    pub background: Option<Color>,
    pub line_width: f32,
}

impl Graph {
    pub fn new(capacity: usize, max: f32) -> Self {
        Self {
            series: Vec::new(),
            capacity,
            max,
            grid: (0, 0),
            grid_color: Color::TRANSPARENT,
            border: None,
            background: None,
            line_width: 1.5,
        }
    }

    pub fn series(mut self, s: Series) -> Self {
        self.series.push(s);
        self
    }

    pub fn grid(mut self, cols: usize, rows: usize, color: Color) -> Self {
        self.grid = (cols, rows);
        self.grid_color = color;
        self
    }

    pub fn border(mut self, c: Color) -> Self {
        self.border = Some(c);
        self
    }

    pub fn background(mut self, c: Color) -> Self {
        self.background = Some(c);
        self
    }

    pub fn line_width(mut self, w: f32) -> Self {
        self.line_width = w;
        self
    }

    /// An element that paints this graph into its layout box.
    pub fn element(self) -> Canvas {
        canvas(move |scene, b| self.paint(scene, b))
    }

    pub fn paint(&self, scene: &mut Scene, b: Bounds) {
        if b.w <= 1.0 || b.h <= 1.0 {
            return;
        }
        if let Some(bg) = self.background {
            scene.rect(b, bg, 0.0);
        }
        let hair = 1.0;
        let (cols, rows) = self.grid;
        for i in 1..cols {
            let x = (b.x + b.w * i as f32 / cols as f32).round();
            scene.rect(Bounds::new(x, b.y, hair, b.h), self.grid_color, 0.0);
        }
        for i in 1..rows {
            let y = (b.y + b.h * i as f32 / rows as f32).round();
            scene.rect(Bounds::new(b.x, y, b.w, hair), self.grid_color, 0.0);
        }
        scene.push_clip(b);
        for s in &self.series {
            paint_series(scene, b, s, self.capacity, self.max, self.line_width);
        }
        scene.pop_clip();
        if let Some(c) = self.border {
            scene.rect_bordered(b, Color::TRANSPARENT, 0.0, c, hair);
        }
    }
}

/// Where sample `i` of `n` lands, with the newest at the right edge.
fn points(b: Bounds, values: &[f32], capacity: usize, max: f32) -> Vec<(f32, f32)> {
    let cap = capacity.max(2);
    let step = b.w / (cap - 1) as f32;
    let n = values.len().min(cap);
    let start = values.len() - n;
    let max = if max > 0.0 { max } else { 1.0 };
    values[start..]
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = b.right() - (n - 1 - i) as f32 * step;
            let t = (v / max).clamp(0.0, 1.0);
            (x, b.bottom() - t * b.h)
        })
        .collect()
}

fn paint_series(scene: &mut Scene, b: Bounds, s: &Series, capacity: usize, max: f32, width: f32) {
    let pts = points(b, &s.values, capacity, max);
    if pts.len() < 2 {
        return;
    }
    if let Some(fill) = s.fill {
        // One-point-wide strips on whole-point boundaries, so neighbors meet
        // exactly and no seams show through a translucent fill.
        let x0 = pts[0].0.floor();
        let x1 = pts[pts.len() - 1].0.ceil();
        let mut k = 0;
        let mut x = x0;
        while x < x1 {
            let mid = x + 0.5;
            while k + 2 < pts.len() && pts[k + 1].0 < mid {
                k += 1;
            }
            let (a, c) = (pts[k], pts[k + 1]);
            let t = if c.0 > a.0 { ((mid - a.0) / (c.0 - a.0)).clamp(0.0, 1.0) } else { 0.0 };
            let y = a.1 + (c.1 - a.1) * t;
            if y < b.bottom() {
                scene.rect(Bounds::new(x, y, 1.0, b.bottom() - y), fill, 0.0);
            }
            x += 1.0;
        }
    }
    if s.dashed {
        let (on, off) = (4.0, 3.0);
        let mut phase = 0.0f32;
        for w in pts.windows(2) {
            let (a, c) = (w[0], w[1]);
            let len = ((c.0 - a.0).powi(2) + (c.1 - a.1).powi(2)).sqrt();
            let mut d = 0.0;
            while d < len {
                let pos = (phase + d) % (on + off);
                let run = if pos < on { (on - pos).min(len - d) } else { (on + off - pos).min(len - d) };
                if pos < on && run > 0.0 {
                    let (t0, t1) = (d / len, (d + run) / len);
                    let p0 = pt(a.0 + (c.0 - a.0) * t0, a.1 + (c.1 - a.1) * t0);
                    let p1 = pt(a.0 + (c.0 - a.0) * t1, a.1 + (c.1 - a.1) * t1);
                    scene.segment(p0, p1, width, s.line);
                }
                d += run.max(0.01);
            }
            phase = (phase + len) % (on + off);
        }
    } else {
        for w in pts.windows(2) {
            scene.segment(pt(w[0].0, w[0].1), pt(w[1].0, w[1].1), width, s.line);
        }
    }
}

/// A rounded "nice" ceiling for an autoscaling axis: 1, 2 or 5 times a power of ten.
pub fn nice_max(v: f32) -> f32 {
    if v <= 0.0 || !v.is_finite() {
        return 1.0;
    }
    let p = 10f32.powf(v.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if v <= m * p {
            return m * p;
        }
    }
    10.0 * p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_sample_is_at_the_right_edge() {
        let b = Bounds::new(0.0, 0.0, 100.0, 50.0);
        let p = points(b, &[0.0, 50.0, 100.0], 11, 100.0);
        assert_eq!(p.len(), 3);
        assert_eq!(p[2], (100.0, 0.0));
        assert_eq!(p[1], (90.0, 25.0));
        assert_eq!(p[0], (80.0, 50.0));
        // Longer than capacity: only the newest `capacity` samples show.
        let many: Vec<f32> = (0..30).map(|i| i as f32).collect();
        assert_eq!(points(b, &many, 11, 100.0).len(), 11);
    }

    #[test]
    fn nice_max_rounds_up() {
        assert_eq!(nice_max(0.0), 1.0);
        assert_eq!(nice_max(3.0), 5.0);
        assert_eq!(nice_max(12.0), 20.0);
        assert_eq!(nice_max(100.0), 100.0);
        assert_eq!(nice_max(501.0), 1000.0);
    }
}
