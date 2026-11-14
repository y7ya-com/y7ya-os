//! Scrolling that feels like a Mac: touchpad pixel deltas follow the fingers,
//! a flick keeps gliding after they lift (momentum), pulling past either end
//! stretches with resistance and springs back (rubber band), and `scroll_y`
//! containers get thin overlay scrollbars that show while scrolling (or when
//! the pointer is near the edge) and fade out afterwards.
//!
//! Everything here runs frame by frame only while something moves or fades;
//! an idle window schedules no frames at all.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::cx::Cx;
use super::element::ElementId;
use super::geom::{Bounds, Color, Point};
use super::scene::Scene;

/// How long a scrollbar stays after the last scroll, then how long it fades.
const BAR_HOLD: Duration = Duration::from_millis(800);
const BAR_FADE: Duration = Duration::from_millis(250);
/// Width of the strip along the right edge where the pointer reveals the bar.
pub(crate) const BAR_ZONE: f32 = 11.0;
const KNOB_W: f32 = 7.0;
const KNOB_W_HOVER: f32 = 10.0;
const KNOB_MARGIN: f32 = 3.0;
const KNOB_MIN: f32 = 28.0;
/// Momentum decay per millisecond (the rate macOS and iOS use for lists).
const DECAY_PER_MS: f32 = 0.998;
/// Below this speed (points per second) a fling stops.
const MIN_SPEED: f32 = 12.0;
/// Rubber band: stiffness of the pull and of the spring back.
const RUBBER: f32 = 0.55;
const SPRING_OMEGA: f32 = 17.0;
/// A touchpad gesture with no events for this long counts as released
/// (some setups never send an end phase).
const GESTURE_IDLE: Duration = Duration::from_millis(160);

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Phase {
    /// A mouse wheel notch: no momentum, no rubber band.
    Wheel,
    /// Fingers on the touchpad.
    Touch,
}

#[derive(Default, Debug)]
pub(crate) struct Region {
    /// From the last paint: how far it can scroll, and the visible viewport.
    pub max: f32,
    pub viewport: Bounds,
    pub content: f32,
    /// Displacement past an end as drawn: negative is pulled down past the
    /// top, positive pulled up past the bottom.
    pub over: f32,
    /// Finger travel past the end before resistance (while the fingers are down).
    stretch: f32,
    /// Momentum or spring velocity, in offset points per second.
    velocity: f32,
    moving: bool,
    /// Last scroll; the scrollbar shows until `BAR_HOLD` after it.
    active_at: Option<Instant>,
}

/// What a touchpad gesture or fling drives.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Target {
    Region(ElementId),
    /// A widget or the app's own `on_scroll`; they get plain deltas.
    Other,
}

#[derive(Default)]
pub(crate) struct Scrolls {
    pub regions: HashMap<ElementId, Region>,
    /// The target a touchpad gesture is latched onto and its last event.
    latched: Option<(Target, Instant)>,
    samples: VecDeque<(Instant, Point)>,
    /// Momentum handed to a widget or the app, in points per second.
    pub other_fling: Option<Point>,
    last_step: Option<Instant>,
    /// Scrollbar being dragged: region and where in the knob it was grabbed.
    pub bar_drag: Option<(ElementId, f32)>,
    /// Region whose scrollbar zone the pointer is over.
    pub bar_hover: Option<ElementId>,
}

fn rubber(stretch: f32, dim: f32) -> f32 {
    let dim = dim.max(1.0);
    (1.0 - 1.0 / (stretch.abs() * RUBBER / dim + 1.0)) * dim * stretch.signum()
}

impl Scrolls {
    /// The target of the current touchpad gesture, if one is still going.
    pub fn latched(&self, now: Instant) -> Option<Target> {
        self.latched.filter(|(_, at)| now - *at < GESTURE_IDLE).map(|(t, _)| t)
    }

    /// Starts or continues a touchpad gesture on `target`, stopping any fling.
    pub fn touch(&mut self, target: Target, delta: Point, now: Instant) {
        if self.latched(now) != Some(target) {
            self.samples.clear();
        }
        self.latched = Some((target, now));
        self.other_fling = None;
        if let Target::Region(id) = target {
            if let Some(r) = self.regions.get_mut(&id) {
                r.moving = false;
                r.velocity = 0.0;
            }
        }
        self.samples.push_back((now, delta));
        while self.samples.len() > 12 {
            self.samples.pop_front();
        }
    }

    /// The fingers lifted: the speed of the last few events, in points per
    /// second (positive `y` toward the top, like the deltas), if it was a flick.
    pub fn release(&mut self, now: Instant) -> Option<(Target, Point)> {
        let (target, last) = self.latched.take()?;
        let samples: Vec<(Instant, Point)> = self.samples.drain(..).collect();
        let recent: Vec<&(Instant, Point)> = samples.iter().filter(|(t, _)| last - *t <= Duration::from_millis(100)).collect();
        // A pause before lifting means the fingers stopped: no momentum.
        if now - last > Duration::from_millis(80) || recent.len() < 2 {
            return Some((target, Point::default()));
        }
        let span = (recent[recent.len() - 1].0 - recent[0].0).as_secs_f32().max(1.0 / 120.0);
        let sum = recent[1..].iter().fold(Point::default(), |a, (_, d)| Point { x: a.x + d.x, y: a.y + d.y });
        Some((target, Point { x: sum.x / span, y: sum.y / span }))
    }

    pub fn animating(&self) -> bool {
        self.other_fling.is_some() || self.regions.values().any(|r| r.moving)
    }

    /// When the next frame without input is due: a fading scrollbar or a
    /// gesture that went quiet without an end event.
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        let mut wake: Option<Instant> = None;
        let mut at = |t: Instant| wake = Some(wake.map_or(t, |w: Instant| w.min(t)));
        if let Some((_, last)) = self.latched {
            if self.regions.values().any(|r| r.stretch != 0.0) {
                at(last + GESTURE_IDLE);
            }
        }
        for r in self.regions.values() {
            if let Some(a) = r.active_at {
                let (hold, gone) = (a + BAR_HOLD, a + BAR_HOLD + BAR_FADE);
                if now < hold {
                    at(hold);
                } else if now < gone {
                    // Fading: next frame.
                    at(now + Duration::from_millis(16));
                }
            }
        }
        wake
    }

    /// Opacity of region `id`'s scrollbar right now.
    fn bar_alpha(&self, id: ElementId, now: Instant) -> f32 {
        if self.bar_drag.is_some_and(|(d, _)| d == id) || self.bar_hover == Some(id) {
            return 1.0;
        }
        let Some(r) = self.regions.get(&id) else { return 0.0 };
        if r.moving {
            return 1.0;
        }
        let Some(a) = r.active_at else { return 0.0 };
        let t = now.saturating_duration_since(a);
        if t < BAR_HOLD {
            1.0
        } else {
            1.0 - ((t - BAR_HOLD).as_secs_f32() / BAR_FADE.as_secs_f32()).min(1.0)
        }
    }
}

impl Cx {
    /// Scrolls region `id` by `dy` (positive toward the top). Touchpad
    /// deltas may pull past the ends; wheel deltas stop at them.
    pub(crate) fn scroll_region(&mut self, id: ElementId, dy: f32, phase: Phase, now: Instant) {
        let off = self.scroll_offset(id);
        let r = self.scrolls.regions.entry(id).or_default();
        r.active_at = Some(now);
        let max = r.max;
        let dim = r.viewport.h;
        if phase == Phase::Wheel {
            r.moving = false;
            r.over = 0.0;
            r.stretch = 0.0;
            let v = (off - dy).clamp(0.0, max);
            self.set_scroll_offset(id, v);
            return;
        }
        // Undo any stretch first, then move the content, then stretch.
        let mut d = -dy;
        if r.stretch != 0.0 {
            let s = r.stretch + d;
            if s.signum() == r.stretch.signum() {
                r.stretch = s;
                d = 0.0;
            } else {
                r.stretch = 0.0;
                d = s;
            }
        }
        let mut v = off + d;
        if v < 0.0 {
            r.stretch += v;
            v = 0.0;
        } else if v > max {
            r.stretch += v - max;
            v = max;
        }
        r.over = rubber(r.stretch, dim);
        self.set_scroll_offset(id, v);
    }

    /// Fingers lifted from region `id`: glide with `velocity` (offset points
    /// per second) and/or spring back from a stretch.
    pub(crate) fn fling_region(&mut self, id: ElementId, velocity: f32, now: Instant) {
        let Some(r) = self.scrolls.regions.get_mut(&id) else { return };
        r.stretch = 0.0;
        r.velocity = velocity.clamp(-6000.0, 6000.0);
        r.moving = r.over != 0.0 || r.velocity.abs() > MIN_SPEED * 4.0;
        if !r.moving {
            r.velocity = 0.0;
        }
        r.active_at = Some(now);
        self.scrolls.last_step = Some(now);
    }

    /// Advances flings and springs to `now`. Returns deltas (positive toward
    /// the top) to hand to the widget or app a fling belongs to.
    pub(crate) fn step_scrolls(&mut self, now: Instant) -> Option<Point> {
        // A gesture that went quiet without an end event: let go of it.
        if let Some((Target::Region(id), last)) = self.scrolls.latched {
            if now - last >= GESTURE_IDLE {
                self.scrolls.latched = None;
                self.scrolls.samples.clear();
                if self.scrolls.regions.get(&id).is_some_and(|r| r.stretch != 0.0) {
                    self.fling_region(id, 0.0, now);
                }
            }
        }
        let dt = self.scrolls.last_step.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32()).min(0.05);
        self.scrolls.last_step = Some(now);
        let ids: Vec<ElementId> = self.scrolls.regions.iter().filter(|(_, r)| r.moving).map(|(id, _)| *id).collect();
        for id in ids {
            let off = self.scroll_offset(id);
            let r = self.scrolls.regions.get_mut(&id).unwrap();
            let (off, done) = advance(r, off, dt);
            r.active_at = Some(now);
            if done {
                r.moving = false;
                r.velocity = 0.0;
                r.over = 0.0;
            }
            self.set_scroll_offset(id, off);
        }
        let v = self.scrolls.other_fling?;
        let decay = DECAY_PER_MS.powf(dt * 1000.0);
        let next = Point { x: v.x * decay, y: v.y * decay };
        self.scrolls.other_fling = (next.x.hypot(next.y) > MIN_SPEED).then_some(next);
        Some(Point { x: v.x * dt, y: v.y * dt })
    }

    /// Records a painted `scroll_y` container; returns the overscroll to draw.
    pub(crate) fn note_region(&mut self, id: ElementId, viewport: Bounds, content: f32, max: f32) -> f32 {
        let r = self.scrolls.regions.entry(id).or_default();
        r.max = max;
        r.viewport = viewport;
        r.content = content;
        r.over
    }

    /// The knob of region `id`'s scrollbar, if it can scroll.
    pub(crate) fn scrollbar_knob(&self, id: ElementId) -> Option<(Bounds, Bounds)> {
        let r = self.scrolls.regions.get(&id)?;
        if r.max <= 0.5 || r.viewport.h < KNOB_MIN * 2.0 {
            return None;
        }
        let wide = self.scrolls.bar_hover == Some(id) || self.scrolls.bar_drag.is_some_and(|(d, _)| d == id);
        let w = if wide { KNOB_W_HOVER } else { KNOB_W };
        let v = r.viewport;
        let track = Bounds::new(v.right() - w - KNOB_MARGIN, v.y + KNOB_MARGIN, w, v.h - 2.0 * KNOB_MARGIN);
        // Squeezed while rubber-banding, like AppKit.
        let len = (track.h * v.h / r.content.max(v.h) - r.over.abs()).max(KNOB_MIN.min(track.h));
        let frac = (self.scroll_offset(id) / r.max).clamp(0.0, 1.0);
        let y = if r.over < 0.0 { track.y } else { track.y + (track.h - len) * frac };
        Some((track, Bounds::new(track.x, y, w, len)))
    }

    pub(crate) fn paint_scrollbar(&self, scene: &mut Scene, id: ElementId) {
        let a = self.scrolls.bar_alpha(id, Instant::now());
        if a <= 0.0 {
            return;
        }
        let Some((track, knob)) = self.scrollbar_knob(id) else { return };
        let ink = if self.dark { Color::WHITE } else { Color::BLACK };
        if self.scrolls.bar_hover == Some(id) || self.scrolls.bar_drag.is_some_and(|(d, _)| d == id) {
            let t = Bounds::new(track.x - 2.0, track.y - KNOB_MARGIN, track.w + 2.0 + KNOB_MARGIN, track.h + 2.0 * KNOB_MARGIN);
            scene.rect(t, ink.alpha(0.05 * a), 0.0);
            scene.rect(Bounds::new(t.x, t.y, 1.0 / self.scale.max(1.0), t.h), ink.alpha(0.08 * a), 0.0);
        }
        let knob_a = if self.dark { 0.45 } else { 0.42 };
        scene.rect(knob, ink.alpha(knob_a * a), knob.w / 2.0);
    }

    /// Which region's scrollbar zone `p` is in (topmost layer first).
    pub(crate) fn scrollbar_zone_at(&self, regions: &[ElementId], p: Point) -> Option<ElementId> {
        regions.iter().rev().copied().find(|id| {
            let Some(r) = self.scrolls.regions.get(id) else { return false };
            r.max > 0.5 && r.viewport.contains(p) && p.x >= r.viewport.right() - BAR_ZONE
        })
    }

    /// For widgets drawing their own scroller: show it now, fading out later.
    pub(crate) fn touch_scroller(&mut self, key: u64) {
        self.scrolls.regions.entry(key).or_default().active_at = Some(Instant::now());
    }

    /// The opacity for a widget's own scroller (see `touch_scroller`).
    pub(crate) fn scroller_alpha(&self, key: u64) -> f32 {
        self.scrolls.bar_alpha(key, Instant::now())
    }

    /// Restarts the hold-then-fade timer of region `id`'s scrollbar.
    pub(crate) fn touch_scrollbar(&mut self, id: Option<ElementId>) {
        if let Some(r) = id.and_then(|id| self.scrolls.regions.get_mut(&id)) {
            r.active_at = Some(Instant::now());
        }
    }

    /// Momentum for scrolling a widget or the app handles itself.
    pub(crate) fn fling_other(&mut self, v: Point, now: Instant) {
        self.scrolls.other_fling = (v.x.hypot(v.y) > MIN_SPEED * 4.0).then_some(v);
        self.scrolls.last_step = Some(now);
    }

    /// Moves region `id` so the knob's grab point follows `y`.
    pub(crate) fn drag_scrollbar(&mut self, id: ElementId, grab: f32, y: f32) {
        let Some((track, knob)) = self.scrollbar_knob(id) else { return };
        let max = self.scrolls.regions[&id].max;
        let room = (track.h - knob.h).max(1.0);
        let frac = ((y - grab - track.y) / room).clamp(0.0, 1.0);
        self.set_scroll_offset(id, frac * max);
        if let Some(r) = self.scrolls.regions.get_mut(&id) {
            r.active_at = Some(Instant::now());
        }
    }
}

/// One step of a fling or spring for a region at offset `off`; returns the
/// new offset and whether it came to rest.
fn advance(r: &mut Region, mut off: f32, dt: f32) -> (f32, bool) {
    // Small sub-steps keep the spring stable on slow frames.
    let steps = (dt / 0.004).ceil().max(1.0) as usize;
    let h = dt / steps as f32;
    for _ in 0..steps {
        if r.over != 0.0 || (off <= 0.0 && r.velocity < 0.0) || (off >= r.max && r.velocity > 0.0) {
            // Past an end: a critically damped spring pulls back to it.
            let a = -SPRING_OMEGA * SPRING_OMEGA * r.over - 2.0 * SPRING_OMEGA * r.velocity;
            r.velocity += a * h;
            r.over += r.velocity * h;
            off = off.clamp(0.0, r.max);
            if r.over.abs() < 0.3 && r.velocity.abs() < 8.0 {
                r.over = 0.0;
                return (off, true);
            }
        } else {
            off += r.velocity * h;
            r.velocity *= DECAY_PER_MS.powf(h * 1000.0);
            if off < 0.0 {
                r.over = off;
                off = 0.0;
            } else if off > r.max {
                r.over = off - r.max;
                off = r.max;
            }
            if r.over == 0.0 && r.velocity.abs() < MIN_SPEED {
                return (off, true);
            }
        }
    }
    (off, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx_with_region(max: f32) -> (Cx, ElementId) {
        let mut cx = Cx::new();
        let id = crate::eid("list");
        cx.note_region(id, Bounds::new(0.0, 0.0, 300.0, 400.0), 400.0 + max, max);
        (cx, id)
    }

    fn run_until_rest(cx: &mut Cx, start: Instant) -> Instant {
        let mut now = start;
        for _ in 0..2000 {
            now += Duration::from_millis(8);
            cx.step_scrolls(now);
            if !cx.scrolls.animating() {
                return now;
            }
        }
        panic!("never came to rest");
    }

    #[test]
    fn fling_glides_then_stops() {
        let (mut cx, id) = cx_with_region(10_000.0);
        let t0 = Instant::now();
        cx.fling_region(id, 2000.0, t0);
        assert!(cx.scrolls.animating());
        let end = run_until_rest(&mut cx, t0);
        let off = cx.scroll_offset(id);
        // Roughly v0 * tau, tau = -1 / ln(0.998) ms.
        assert!((800.0..1200.0).contains(&off), "glided {off}");
        assert!(end - t0 < Duration::from_secs(4));
        // Idle afterwards: no more frames, and the bar fades on a timer.
        assert!(!cx.scrolls.animating());
        let wake = cx.scrolls.next_wake(end).unwrap();
        assert!(wake > end);
        assert!(cx.scrolls.next_wake(end + BAR_HOLD + BAR_FADE + Duration::from_millis(1)).is_none());
    }

    #[test]
    fn pulling_past_the_top_resists_and_springs_back() {
        let (mut cx, id) = cx_with_region(500.0);
        let t0 = Instant::now();
        cx.scrolls.touch(Target::Region(id), Point { x: 0.0, y: 300.0 }, t0);
        cx.scroll_region(id, 300.0, Phase::Touch, t0);
        let over = cx.scrolls.regions[&id].over;
        assert!(over < -20.0 && over > -300.0, "stretched to {over}");
        assert_eq!(cx.scroll_offset(id), 0.0);
        cx.fling_region(id, 0.0, t0);
        run_until_rest(&mut cx, t0);
        assert_eq!(cx.scrolls.regions[&id].over, 0.0);
        assert_eq!(cx.scroll_offset(id), 0.0);
    }

    #[test]
    fn a_fling_into_the_end_bounces() {
        let (mut cx, id) = cx_with_region(300.0);
        let t0 = Instant::now();
        cx.set_scroll_offset(id, 250.0);
        cx.fling_region(id, 3000.0, t0);
        let mut now = t0;
        let mut peak: f32 = 0.0;
        while cx.scrolls.animating() {
            now += Duration::from_millis(8);
            cx.step_scrolls(now);
            peak = peak.max(cx.scrolls.regions[&id].over);
        }
        assert!(peak > 5.0 && peak < 120.0, "overshoot {peak}");
        assert_eq!(cx.scroll_offset(id), 300.0);
    }

    #[test]
    fn release_measures_the_flick() {
        let mut s = Scrolls::default();
        let t0 = Instant::now();
        let target = Target::Region(crate::eid("x"));
        for i in 0..6 {
            s.touch(target, Point { x: 0.0, y: -20.0 }, t0 + Duration::from_millis(10 * i));
        }
        let (t, v) = s.release(t0 + Duration::from_millis(55)).unwrap();
        assert_eq!(t, target);
        assert!((v.y + 2000.0).abs() < 1.0, "velocity {v:?}");

        // Fingers that rest before lifting don't fling.
        s.touch(target, Point { x: 0.0, y: -20.0 }, t0);
        s.touch(target, Point { x: 0.0, y: -20.0 }, t0 + Duration::from_millis(10));
        let (_, v) = s.release(t0 + Duration::from_millis(300)).unwrap();
        assert_eq!(v, Point::default());
    }

    #[test]
    fn wheel_stops_at_the_ends() {
        let (mut cx, id) = cx_with_region(100.0);
        cx.scroll_region(id, -500.0, Phase::Wheel, Instant::now());
        assert_eq!(cx.scroll_offset(id), 100.0);
        assert_eq!(cx.scrolls.regions[&id].over, 0.0);
    }
}
