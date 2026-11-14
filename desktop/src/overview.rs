//! Spaces (multiple desktops), Mission Control and the app switcher.
//!
//! Mission Control spreads the current space's windows out so none overlap,
//! with a bar of spaces along the top. It follows a three-finger swipe up
//! (or Super+Up); swiping left/right (or Super+Left/Right) changes space.

pub use crate::anim::Anim;

use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::{Desk, frame_of};

/// Height of the strip of spaces at the top of Mission Control.
pub const SPACES_BAR_H: f64 = 124.0;
/// Spring settings (response in seconds, damping ratio).
const MC_SPRING: (f64, f64) = (0.42, 0.92);
const SLIDE_SPRING: (f64, f64) = (0.48, 0.94);
/// How far a three-finger swipe travels for a full transition.
const SWIPE_FULL: f64 = 320.0;

/// A space change in progress: `from` slides out while the active space slides in.
pub struct Slide {
    pub from: usize,
    /// 0 = still showing `from`, 1 = arrived.
    pub anim: Anim,
}

pub struct Switcher {
    pub windows: Vec<Window>,
    pub index: usize,
}

#[derive(Default)]
pub struct Gesture {
    fingers: u32,
    dx: f64,
    dy: f64,
    axis: Option<Axis>,
    /// Mission Control's openness when the swipe began.
    base: f64,
    /// The space the swipe started on.
    origin: usize,
    /// How fast the gesture's value is moving (per second), smoothed.
    speed: f64,
    last: Option<std::time::Instant>,
}

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    Vertical,
    Horizontal,
}

pub struct Spaces {
    /// Windows parked on inactive spaces, with their positions. The active
    /// space's entry is empty; its windows live in `Desk::space`.
    pub parked: Vec<Vec<(Window, Point<i32, Logical>)>>,
    pub active: usize,
    pub slide: Option<Slide>,
    /// Mission Control: 0 = closed, 1 = fully open.
    pub mc: Anim,
    pub mc_hover: Option<Window>,
    pub switcher: Option<Switcher>,
    /// Most recently used windows first.
    pub mru: Vec<Window>,
    pub gesture: Option<Gesture>,
}

impl Default for Spaces {
    fn default() -> Self {
        Spaces {
            parked: vec![Vec::new(), Vec::new()],
            active: 0,
            slide: None,
            mc: Anim::fixed(0.0),
            mc_hover: None,
            switcher: None,
            mru: Vec::new(),
            gesture: None,
        }
    }
}

/// What's under the pointer in Mission Control.
pub enum McHit {
    Window(Window),
    Space(usize),
    AddSpace,
    Nothing,
}

impl Desk {
    pub fn mc_open(&self) -> bool {
        self.spaces.mc.value() > 0.0 || self.spaces.mc.target() > 0.0
    }

    pub fn toggle_mission_control(&mut self) {
        if self.spaces.mc.target() > 0.5 {
            self.close_mission_control(None);
        } else {
            self.spaces.mc.retarget(1.0, MC_SPRING.0, MC_SPRING.1);
            self.spaces.mc_hover = None;
            self.redraw();
        }
    }

    pub fn close_mission_control(&mut self, focus: Option<Window>) {
        self.spaces.mc.retarget(0.0, MC_SPRING.0, MC_SPRING.1);
        self.spaces.mc_hover = None;
        if focus.is_some() {
            self.focus(focus);
        }
        self.redraw();
    }

    /// Where each window of the current space sits in Mission Control.
    pub fn mc_layout(&self) -> Vec<(Window, Rectangle<f64, Logical>)> {
        let area = self.usable_area().to_f64();
        let area = Rectangle::<f64, Logical>::new(
            (area.loc.x + 56.0, area.loc.y + SPACES_BAR_H + 28.0).into(),
            (area.size.w - 112.0, area.size.h - SPACES_BAR_H - 72.0).into(),
        );
        let mut windows: Vec<(Window, Rectangle<i32, Logical>)> = self
            .space
            .elements()
            .filter_map(|w| self.space.element_location(w).map(|loc| (w.clone(), frame_of(w, loc))))
            .collect();
        if windows.is_empty() {
            return Vec::new();
        }
        // Keep the user's rough arrangement: read rows top to bottom, then left to right.
        windows.sort_by_key(|(_, f)| (f.loc.y / 200, f.loc.x));
        let n = windows.len();
        let gap = 36.0;
        // Pick the column count that lets the windows be largest.
        let mut best = (1, 0.0);
        for cols in 1..=n {
            let rows = n.div_ceil(cols);
            let cw = (area.size.w - gap * (cols - 1) as f64) / cols as f64;
            let ch = (area.size.h - gap * (rows - 1) as f64) / rows as f64;
            let s = windows
                .iter()
                .map(|(_, f)| (cw / f.size.w as f64).min(ch / f.size.h as f64))
                .fold(f64::MAX, f64::min)
                .min(1.0);
            if s > best.1 {
                best = (cols, s);
            }
        }
        let (cols, _) = best;
        let rows = n.div_ceil(cols);
        let cw = (area.size.w - gap * (cols - 1) as f64) / cols as f64;
        let ch = (area.size.h - gap * (rows - 1) as f64) / rows as f64;
        let mut out = Vec::with_capacity(n);
        for (i, (w, f)) in windows.into_iter().enumerate() {
            let (row, col) = (i / cols, i % cols);
            // Center short last rows.
            let in_row = if row == rows - 1 { n - row * cols } else { cols };
            let row_w = in_row as f64 * cw + (in_row - 1) as f64 * gap;
            let x0 = area.loc.x + (area.size.w - row_w) / 2.0;
            let s = (cw / f.size.w as f64).min(ch / f.size.h as f64).min(1.0) * 0.94;
            let size = Size::<f64, Logical>::from((f.size.w as f64 * s, f.size.h as f64 * s));
            let cx = x0 + col as f64 * (cw + gap) + cw / 2.0;
            let cy = area.loc.y + row as f64 * (ch + gap) + ch / 2.0;
            out.push((w, Rectangle::new((cx - size.w / 2.0, cy - size.h / 2.0).into(), size)));
        }
        out
    }

    /// Tiles of the spaces bar, plus the "add space" button at the end.
    pub fn space_tiles(&self) -> (Vec<Rectangle<f64, Logical>>, Rectangle<f64, Logical>) {
        let out = self.output_geometry().to_f64();
        let h = 78.0;
        let w = h * out.size.w / out.size.h;
        let gap = 18.0;
        let n = self.spaces.parked.len() as f64;
        let total = n * w + (n - 1.0) * gap;
        let x0 = out.loc.x + (out.size.w - total) / 2.0;
        let y = self.usable_area().loc.y as f64 + 22.0;
        let tiles = (0..self.spaces.parked.len())
            .map(|i| Rectangle::new((x0 + i as f64 * (w + gap), y).into(), (w, h).into()))
            .collect();
        let add = Rectangle::new((out.loc.x + out.size.w - 72.0, y + h / 2.0 - 22.0).into(), (44.0, 44.0).into());
        (tiles, add)
    }

    pub fn mc_hit(&self, pos: Point<f64, Logical>) -> McHit {
        let (tiles, add) = self.space_tiles();
        if let Some(i) = tiles.iter().position(|t| t.contains(pos)) {
            return McHit::Space(i);
        }
        if add.contains(pos) {
            return McHit::AddSpace;
        }
        self.mc_layout()
            .into_iter()
            .rev()
            .find(|(_, r)| r.contains(pos))
            .map_or(McHit::Nothing, |(w, _)| McHit::Window(w))
    }

    pub fn mc_click(&mut self, pos: Point<f64, Logical>) {
        match self.mc_hit(pos) {
            McHit::Window(w) => self.close_mission_control(Some(w)),
            McHit::Space(i) => {
                self.switch_space(i);
                self.close_mission_control(None);
            }
            McHit::AddSpace => {
                self.spaces.parked.push(Vec::new());
                self.redraw();
            }
            McHit::Nothing => self.close_mission_control(None),
        }
    }

    pub fn mc_hover(&mut self, pos: Point<f64, Logical>) {
        let hovered = match self.mc_hit(pos) {
            McHit::Window(w) => Some(w),
            _ => None,
        };
        if hovered != self.spaces.mc_hover {
            self.spaces.mc_hover = hovered;
            self.redraw();
        }
    }

    /// Moves to space `n`, sliding the screen sideways.
    pub fn switch_space(&mut self, n: usize) {
        let from = self.spaces.active;
        if n == from || n >= self.spaces.parked.len() {
            return;
        }
        self.park_active();
        self.spaces.active = n;
        for (w, loc) in std::mem::take(&mut self.spaces.parked[n]) {
            self.space.map_element(w, loc, false);
        }
        let start = self.spaces.slide.take().filter(|s| s.from == n).map_or(0.0, |s| 1.0 - s.anim.value());
        let mut anim = Anim::fixed(start);
        anim.retarget(1.0, SLIDE_SPRING.0, SLIDE_SPRING.1);
        self.spaces.slide = Some(Slide { from, anim });
        let top = self.spaces.mru.iter().find(|w| self.space.elements().any(|x| x == *w)).cloned();
        self.focus(top.or_else(|| self.space.elements().last().cloned()));
    }

    fn park_active(&mut self) {
        let windows: Vec<(Window, Point<i32, Logical>)> = self
            .space
            .elements()
            .filter_map(|w| self.space.element_location(w).map(|l| (w.clone(), l)))
            .collect();
        for (w, _) in &windows {
            self.space.unmap_elem(w);
        }
        self.spaces.parked[self.spaces.active] = windows;
    }

    /// Sends the focused window to the neighboring space and follows it.
    pub fn move_to_space(&mut self, n: usize) {
        let Some(window) = self.focused_window() else { return };
        if n >= self.spaces.parked.len() || n == self.spaces.active {
            return;
        }
        let Some(loc) = self.space.element_location(&window) else { return };
        self.space.unmap_elem(&window);
        self.spaces.parked[n].push((window.clone(), loc));
        self.switch_space(n);
        self.focus(Some(window));
    }

    pub fn note_focus(&mut self, window: &Window) {
        self.spaces.mru.retain(|w| w != window);
        self.spaces.mru.insert(0, window.clone());
    }

    /// Windows to draw for the current frame, each with a sideways offset
    /// (non-zero only while sliding between spaces). Bottom to top.
    pub fn visible_windows(&self) -> Vec<(Window, Point<i32, Logical>, f64)> {
        let width = self.output_geometry().size.w as f64;
        let mut out = Vec::new();
        if let Some(slide) = &self.spaces.slide {
            let t = slide.anim.value();
            let dir = if slide.from < self.spaces.active { 1.0 } else { -1.0 };
            for (w, loc) in &self.spaces.parked[slide.from] {
                out.push((w.clone(), *loc, -dir * width * t));
            }
            for w in self.space.elements() {
                if let Some(loc) = self.space.element_location(w) {
                    out.push((w.clone(), loc, dir * width * (1.0 - t)));
                }
            }
            return out;
        }
        for w in self.space.elements() {
            if let Some(loc) = self.space.element_location(w) {
                out.push((w.clone(), loc, 0.0));
            }
        }
        out
    }

    /// Keeps animations going; returns true while any are running.
    pub fn tick_spaces(&mut self) -> bool {
        if self.spaces.slide.as_ref().is_some_and(|s| s.anim.done() && self.spaces.gesture.is_none()) {
            self.spaces.slide = None;
        }
        !self.spaces.mc.done() || self.spaces.slide.is_some()
    }

    // -- the app switcher ------------------------------------------------

    pub fn switcher_next(&mut self, back: bool) {
        match &mut self.spaces.switcher {
            Some(s) => {
                let n = s.windows.len();
                s.index = if back { (s.index + n - 1) % n } else { (s.index + 1) % n };
            }
            None => {
                let mut windows: Vec<Window> = self.spaces.mru.clone();
                let all: Vec<Window> = self.space.elements().rev().chain(self.minimized.iter()).cloned().collect();
                windows.retain(|w| all.contains(w));
                for w in all {
                    if !windows.contains(&w) {
                        windows.push(w);
                    }
                }
                if windows.is_empty() {
                    return;
                }
                let index = if windows.len() > 1 { 1 } else { 0 };
                self.spaces.switcher = Some(Switcher { windows, index });
            }
        }
        self.redraw();
    }

    pub fn switcher_commit(&mut self) {
        let Some(s) = self.spaces.switcher.take() else { return };
        if let Some(w) = s.windows.get(s.index).cloned() {
            if self.minimized.contains(&w) {
                self.restore(w);
            } else {
                self.focus(Some(w));
            }
        }
        self.redraw();
    }

    // -- three-finger gestures -------------------------------------------

    pub fn swipe_begin(&mut self, fingers: u32) {
        let base = self.spaces.mc.value();
        let origin = self.spaces.active;
        self.spaces.gesture = Some(Gesture { fingers, base, origin, ..Default::default() });
    }

    pub fn swipe_update(&mut self, dx: f64, dy: f64) {
        let width = self.output_geometry().size.w as f64;
        let Some(g) = self.spaces.gesture.as_mut() else { return };
        if g.fingers != 3 {
            return;
        }
        g.dx += dx;
        g.dy += dy;
        if g.axis.is_none() && (g.dx.abs() > 12.0 || g.dy.abs() > 12.0) {
            g.axis = Some(if g.dy.abs() > g.dx.abs() { Axis::Vertical } else { Axis::Horizontal });
        }
        let now = std::time::Instant::now();
        let dt = g.last.map_or(0.016, |t| now.duration_since(t).as_secs_f64().max(0.001));
        g.last = Some(now);
        match g.axis {
            Some(Axis::Vertical) => {
                let before = self.spaces.mc.value();
                let v = (g.base - g.dy / SWIPE_FULL).clamp(0.0, 1.0);
                g.speed = g.speed * 0.6 + (v - before) / dt * 0.4;
                self.spaces.mc = Anim::fixed(v);
            }
            Some(Axis::Horizontal) => {
                // Follow the fingers: the neighbor slides in alongside.
                let (origin, gdx) = (g.origin, g.dx);
                let target = if gdx < 0.0 { origin + 1 } else { origin.wrapping_sub(1) };
                let progress = (gdx.abs() / width * 1.6).min(1.0);
                let before = self.spaces.slide.as_ref().map_or(0.0, |s| s.anim.value());
                g.speed = g.speed * 0.6 + (progress - before) / dt * 0.4;
                if target < self.spaces.parked.len() {
                    self.preview_slide(origin, target, progress);
                }
            }
            None => {}
        }
        self.redraw();
    }

    /// Shows `target` sliding in over `origin`, `progress` of the way.
    fn preview_slide(&mut self, origin: usize, target: usize, progress: f64) {
        if self.spaces.active != target {
            // Swap which space is live (the fingers may have changed direction).
            let back = self.spaces.active;
            self.park_active();
            if back != origin {
                // Bring the origin back first.
                for (w, loc) in std::mem::take(&mut self.spaces.parked[origin]) {
                    self.space.map_element(w, loc, false);
                }
                self.spaces.active = origin;
                self.park_active();
            }
            self.spaces.active = target;
            for (w, loc) in std::mem::take(&mut self.spaces.parked[target]) {
                self.space.map_element(w, loc, false);
            }
        }
        self.spaces.slide = Some(Slide { from: origin, anim: Anim::fixed(progress) });
    }

    pub fn swipe_end(&mut self, cancelled: bool) {
        let Some(g) = self.spaces.gesture.take() else { return };
        match g.axis {
            Some(Axis::Vertical) => {
                let v = self.spaces.mc.value();
                // A quick flick counts even if it didn't travel far.
                let open = !cancelled && (if g.base > 0.5 { v > 0.7 } else { v > 0.3 } || g.speed > 2.5) && g.speed > -2.5;
                let (r, d) = MC_SPRING;
                self.spaces.mc.fling(if open { 1.0 } else { 0.0 }, g.speed, r, d);
            }
            Some(Axis::Horizontal) => {
                if let Some(slide) = self.spaces.slide.take() {
                    let progress = slide.anim.value();
                    let (r, d) = SLIDE_SPRING;
                    if !cancelled && (progress > 0.3 || g.speed > 2.0) {
                        let mut anim = slide.anim;
                        anim.fling(1.0, g.speed.max(0.0), r, d);
                        self.spaces.slide = Some(Slide { from: slide.from, anim });
                        self.focus_top();
                    } else {
                        // Go back where we were.
                        let back = slide.from;
                        let arriving = self.spaces.active;
                        self.park_active();
                        self.spaces.active = back;
                        for (w, loc) in std::mem::take(&mut self.spaces.parked[back]) {
                            self.space.map_element(w, loc, false);
                        }
                        let mut anim = Anim::fixed(1.0 - progress);
                        anim.fling(1.0, (-g.speed).max(0.0), r, d);
                        self.spaces.slide = Some(Slide { from: arriving, anim });
                    }
                }
            }
            None => {}
        }
        self.redraw();
    }
}
