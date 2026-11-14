//! Pointer grabs for moving and resizing windows (adapted from Smithay's
//! smallvil example, MIT licensed).

use std::cell::RefCell;

use smithay::desktop::Window;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::compositor;
use smithay::wayland::shell::xdg::SurfaceCachedState;

use crate::state::Desk;

const BTN_LEFT: u32 = 0x110;

/// Everything but motion and button just passes through.
macro_rules! forward_rest {
    () => {
        fn relative_motion(
            &mut self,
            data: &mut Desk,
            handle: &mut PointerInnerHandle<'_, Desk>,
            focus: Option<(WlSurface, Point<f64, Logical>)>,
            event: &RelativeMotionEvent,
        ) {
            handle.relative_motion(data, focus, event);
        }
        fn axis(&mut self, data: &mut Desk, handle: &mut PointerInnerHandle<'_, Desk>, details: AxisFrame) {
            handle.axis(data, details)
        }
        fn frame(&mut self, data: &mut Desk, handle: &mut PointerInnerHandle<'_, Desk>) {
            handle.frame(data);
        }
        fn gesture_swipe_begin(&mut self, d: &mut Desk, h: &mut PointerInnerHandle<'_, Desk>, e: &GestureSwipeBeginEvent) {
            h.gesture_swipe_begin(d, e)
        }
        fn gesture_swipe_update(&mut self, d: &mut Desk, h: &mut PointerInnerHandle<'_, Desk>, e: &GestureSwipeUpdateEvent) {
            h.gesture_swipe_update(d, e)
        }
        fn gesture_swipe_end(&mut self, d: &mut Desk, h: &mut PointerInnerHandle<'_, Desk>, e: &GestureSwipeEndEvent) {
            h.gesture_swipe_end(d, e)
        }
        fn gesture_pinch_begin(&mut self, d: &mut Desk, h: &mut PointerInnerHandle<'_, Desk>, e: &GesturePinchBeginEvent) {
            h.gesture_pinch_begin(d, e)
        }
        fn gesture_pinch_update(&mut self, d: &mut Desk, h: &mut PointerInnerHandle<'_, Desk>, e: &GesturePinchUpdateEvent) {
            h.gesture_pinch_update(d, e)
        }
        fn gesture_pinch_end(&mut self, d: &mut Desk, h: &mut PointerInnerHandle<'_, Desk>, e: &GesturePinchEndEvent) {
            h.gesture_pinch_end(d, e)
        }
        fn gesture_hold_begin(&mut self, d: &mut Desk, h: &mut PointerInnerHandle<'_, Desk>, e: &GestureHoldBeginEvent) {
            h.gesture_hold_begin(d, e)
        }
        fn gesture_hold_end(&mut self, d: &mut Desk, h: &mut PointerInnerHandle<'_, Desk>, e: &GestureHoldEndEvent) {
            h.gesture_hold_end(d, e)
        }
        fn start_data(&self) -> &GrabStartData<Desk> {
            &self.start_data
        }
        fn unset(&mut self, _data: &mut Desk) {}
    };
}

pub struct MoveGrab {
    pub start_data: GrabStartData<Desk>,
    pub window: Window,
    pub initial_loc: Point<i32, Logical>,
}

impl PointerGrab<Desk> for MoveGrab {
    fn motion(
        &mut self,
        data: &mut Desk,
        handle: &mut PointerInnerHandle<'_, Desk>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        let delta = event.location - self.start_data.location;
        // Dragging a tiled or zoomed window gives it back its own size, with
        // the title bar staying under the pointer.
        if delta.x.abs() + delta.y.abs() > 6.0 {
            if let Some(loc) = data.untile(&self.window, self.start_data.location) {
                self.initial_loc = loc;
            }
        }
        let mut loc = (self.initial_loc.to_f64() + delta).to_i32_round::<i32>();
        // Like macOS, a title bar can't be dragged above the top of the screen.
        loc.y = loc.y.max(data.min_content_y(&self.window));
        data.space.map_element(self.window.clone(), loc, true);
        // Near an edge, show where the window would tile.
        let preview = data.snap_zone(event.location);
        if preview != data.snap_preview {
            data.snap_preview = preview;
        }
        data.redraw();
    }

    fn button(&mut self, data: &mut Desk, handle: &mut PointerInnerHandle<'_, Desk>, event: &ButtonEvent) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
            if let Some(rect) = data.snap_preview.take() {
                data.tile(&self.window, rect);
            }
        }
    }

    forward_rest!();
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Edges: u32 {
        const TOP = 0b0001;
        const BOTTOM = 0b0010;
        const LEFT = 0b0100;
        const RIGHT = 0b1000;
    }
}

impl From<xdg_toplevel::ResizeEdge> for Edges {
    fn from(x: xdg_toplevel::ResizeEdge) -> Self {
        Self::from_bits(x as u32).unwrap_or(Edges::empty())
    }
}

pub struct ResizeGrab {
    start_data: GrabStartData<Desk>,
    window: Window,
    edges: Edges,
    initial: Rectangle<i32, Logical>,
    last_size: Size<i32, Logical>,
}

impl ResizeGrab {
    pub fn start(start_data: GrabStartData<Desk>, window: Window, edges: Edges, initial: Rectangle<i32, Logical>) -> Self {
        ResizeState::with(window.toplevel().unwrap().wl_surface(), |s| *s = ResizeState::Resizing { edges, initial });
        let toplevel = window.toplevel().unwrap();
        toplevel.with_pending_state(|s| {
            s.states.set(xdg_toplevel::State::Resizing);
        });
        toplevel.send_pending_configure();
        Self { start_data, window, edges, initial, last_size: initial.size }
    }
}

impl PointerGrab<Desk> for ResizeGrab {
    fn motion(
        &mut self,
        data: &mut Desk,
        handle: &mut PointerInnerHandle<'_, Desk>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        let mut delta = event.location - self.start_data.location;
        let mut w = self.initial.size.w;
        let mut h = self.initial.size.h;
        if self.edges.intersects(Edges::LEFT | Edges::RIGHT) {
            if self.edges.contains(Edges::LEFT) {
                delta.x = -delta.x;
            }
            w = (self.initial.size.w as f64 + delta.x) as i32;
        }
        if self.edges.intersects(Edges::TOP | Edges::BOTTOM) {
            if self.edges.contains(Edges::TOP) {
                delta.y = -delta.y;
            }
            h = (self.initial.size.h as f64 + delta.y) as i32;
        }
        let toplevel = self.window.toplevel().unwrap();
        let (min, max) = compositor::with_states(toplevel.wl_surface(), |states| {
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let data = guard.current();
            (data.min_size, data.max_size)
        });
        let max_w = if max.w == 0 { i32::MAX } else { max.w };
        let max_h = if max.h == 0 { i32::MAX } else { max.h };
        self.last_size = Size::from((w.clamp(min.w.max(120), max_w.max(120)), h.clamp(min.h.max(60), max_h.max(60))));
        toplevel.with_pending_state(|s| {
            s.states.set(xdg_toplevel::State::Resizing);
            s.size = Some(self.last_size);
        });
        toplevel.send_pending_configure();
    }

    fn button(&mut self, data: &mut Desk, handle: &mut PointerInnerHandle<'_, Desk>, event: &ButtonEvent) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
            let toplevel = self.window.toplevel().unwrap();
            toplevel.with_pending_state(|s| {
                s.states.unset(xdg_toplevel::State::Resizing);
                s.size = Some(self.last_size);
            });
            toplevel.send_pending_configure();
            ResizeState::with(toplevel.wl_surface(), |s| {
                *s = ResizeState::WaitingForLastCommit { edges: self.edges, initial: self.initial }
            });
        }
    }

    forward_rest!();
}

#[derive(Debug, Clone, Copy, Default)]
enum ResizeState {
    #[default]
    Idle,
    Resizing { edges: Edges, initial: Rectangle<i32, Logical> },
    WaitingForLastCommit { edges: Edges, initial: Rectangle<i32, Logical> },
}

impl ResizeState {
    fn with<T>(surface: &WlSurface, f: impl FnOnce(&mut Self) -> T) -> T {
        compositor::with_states(surface, |states| {
            states.data_map.insert_if_missing(RefCell::<Self>::default);
            f(&mut states.data_map.get::<RefCell<Self>>().unwrap().borrow_mut())
        })
    }
}

/// Keeps the opposite edge still while resizing from the top or left.
pub fn handle_commit(desk: &mut Desk, surface: &WlSurface) {
    let Some(window) = desk.window_for_surface(surface) else { return };
    let Some(mut loc) = desk.space.element_location(&window) else { return };
    let size = window.geometry().size;
    let pending = ResizeState::with(surface, |s| match *s {
        ResizeState::Resizing { edges, initial } => Some((edges, initial)),
        ResizeState::WaitingForLastCommit { edges, initial } => {
            *s = ResizeState::Idle;
            Some((edges, initial))
        }
        ResizeState::Idle => None,
    });
    let Some((edges, initial)) = pending else { return };
    if edges.contains(Edges::LEFT) {
        loc.x = initial.loc.x + (initial.size.w - size.w);
    }
    if edges.contains(Edges::TOP) {
        loc.y = initial.loc.y + (initial.size.h - size.h);
    }
    if edges.intersects(Edges::LEFT | Edges::TOP) {
        desk.space.map_element(window, loc, false);
    }
}
