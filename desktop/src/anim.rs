//! Spring animation, the way macOS moves things: a damped spring towards a
//! target, so motion keeps its speed when retargeted mid-flight and swipes
//! hand their finger velocity over.

use std::sync::OnceLock;
use std::time::Instant;

/// `Y7_SLOW=10` plays every animation ten times slower, to inspect them.
fn slow_motion() -> f64 {
    static SLOW: OnceLock<f64> = OnceLock::new();
    *SLOW.get_or_init(|| std::env::var("Y7_SLOW").ok().and_then(|s| s.parse().ok()).filter(|s| *s > 0.0).unwrap_or(1.0))
}

#[derive(Clone, Copy, Debug)]
pub struct Anim {
    from: f64,
    to: f64,
    /// Starting velocity, in units per second.
    v0: f64,
    start: Instant,
    /// Roughly how long the motion takes (the spring's period).
    response: f64,
    /// 1 = no overshoot; below 1 settles with a little bounce.
    damping: f64,
}

impl Anim {
    /// A value sitting still at `v`.
    pub fn fixed(v: f64) -> Self {
        Anim { from: v, to: v, v0: 0.0, start: Instant::now(), response: 0.4, damping: 1.0 }
    }

    pub fn spring(from: f64, to: f64, response: f64, damping: f64) -> Self {
        Anim { from, to, v0: 0.0, start: Instant::now(), response, damping }
    }

    fn at(&self, t: f64) -> f64 {
        let w = std::f64::consts::TAU / self.response;
        let c1 = self.from - self.to;
        let z = self.damping;
        if z >= 1.0 {
            // Critically damped.
            let c2 = self.v0 + w * c1;
            self.to + (c1 + c2 * t) * (-w * t).exp()
        } else {
            let wd = w * (1.0 - z * z).sqrt();
            let c2 = (self.v0 + z * w * c1) / wd;
            self.to + (-z * w * t).exp() * (c1 * (wd * t).cos() + c2 * (wd * t).sin())
        }
    }

    fn t(&self) -> f64 {
        self.start.elapsed().as_secs_f64() / slow_motion()
    }

    pub fn value(&self) -> f64 {
        if self.done() { self.to } else { self.at(self.t()) }
    }

    pub fn velocity(&self) -> f64 {
        let t = self.t();
        (self.at(t + 0.001) - self.at(t)) / 0.001
    }

    pub fn done(&self) -> bool {
        if self.from == self.to && self.v0 == 0.0 {
            return true;
        }
        // Settled once the motion left is under a thousandth of the distance.
        let t = self.t();
        t > self.response * 0.25 && (self.at(t) - self.to).abs() < 1e-3 && self.velocity().abs() < 1e-2
    }

    /// Seconds since this motion started.
    pub fn start_elapsed(&self) -> f64 {
        self.t()
    }

    pub fn target(&self) -> f64 {
        self.to
    }

    /// Heads for `to` from wherever the value is now, keeping its velocity.
    pub fn retarget(&mut self, to: f64, response: f64, damping: f64) {
        let (from, v0) = if self.done() { (self.to, 0.0) } else { (self.value(), self.velocity()) };
        *self = Anim { from, to, v0, start: Instant::now(), response, damping };
    }

    /// Like [`retarget`](Self::retarget) but starting with velocity `v0`
    /// (e.g. how fast the fingers were moving when they let go).
    pub fn fling(&mut self, to: f64, v0: f64, response: f64, damping: f64) {
        *self = Anim { from: self.value(), to, v0, start: Instant::now(), response, damping };
    }
}
