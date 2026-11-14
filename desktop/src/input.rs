//! Keyboard shortcuts, pointer focus and our window chrome's buttons.

use std::time::{Duration, Instant};

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::{FilterResult, Keycode, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent};
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER};

use crate::grabs::{MoveGrab, ResizeGrab};
use crate::state::{Button, Desk, Hit};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

#[derive(Debug, Clone)]
pub enum Action {
    Quit,
    Close,
    Minimize,
    Unminimize,
    Zoom,
    NextWindow,
    Spawn(&'static str),
}

impl Desk {
    pub fn process_input<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => {
                self.key(event.key_code(), event.state(), event.time_msec());
            }
            InputEvent::PointerMotion { event } => {
                let pos = self.seat.get_pointer().unwrap().current_location() + event.delta();
                self.pointer_to(pos, event.time_msec());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let out = self.output_geometry();
                let pos = event.position_transformed(out.size) + out.loc.to_f64();
                self.pointer_to(pos, event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                self.pointer_button(event.button_code(), event.state(), event.time_msec());
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let amount = |axis| {
                    event.amount(axis).unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0)
                };
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let v = amount(axis);
                    if v != 0.0 {
                        frame = frame.value(axis, v);
                        if let Some(d) = event.amount_v120(axis) {
                            frame = frame.v120(axis, d as i32);
                        }
                    }
                    if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                        frame = frame.stop(axis);
                    }
                }
                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    pub fn key(&mut self, code: Keycode, state: KeyState, time: u32) {
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.seat.get_keyboard().unwrap();
        let action = keyboard.input::<Action, _>(self, code, state, serial, time, |_, mods, handle| {
            if state != KeyState::Pressed {
                return FilterResult::Forward;
            }
            let sym = handle.modified_sym();
            // The unshifted key, so Super+Shift+M still reads as "m".
            let lower = handle.raw_latin_sym_or_raw_current_sym().unwrap_or(sym).raw();
            if mods.ctrl && mods.alt && sym.raw() == keysyms::KEY_BackSpace {
                return FilterResult::Intercept(Action::Quit);
            }
            if !mods.logo {
                return FilterResult::Forward;
            }
            let action = match lower {
                keysyms::KEY_q | keysyms::KEY_w => Action::Close,
                keysyms::KEY_m if mods.shift => Action::Unminimize,
                keysyms::KEY_m => Action::Minimize,
                keysyms::KEY_f => Action::Zoom,
                keysyms::KEY_Tab => Action::NextWindow,
                keysyms::KEY_Return | keysyms::KEY_t => Action::Spawn("konsole"),
                keysyms::KEY_e => Action::Spawn("dolphin"),
                _ => return FilterResult::Forward,
            };
            FilterResult::Intercept(action)
        });
        if let Some(action) = action {
            self.run(action);
        }
    }

    pub fn run(&mut self, action: Action) {
        let focused = self.focused_window();
        match action {
            Action::Quit => self.loop_signal.stop(),
            Action::Close => focused.iter().for_each(|w| self.close(w)),
            Action::Minimize => focused.iter().for_each(|w| self.minimize(w)),
            Action::Unminimize => self.unminimize_last(),
            Action::Zoom => focused.iter().for_each(|w| self.zoom(w)),
            Action::NextWindow => {
                // Raise the bottom-most window: cycles through all of them.
                let bottom = self.space.elements().next().cloned();
                if bottom != focused {
                    self.focus(bottom);
                }
            }
            Action::Spawn(cmd) => self.spawn(cmd),
        }
    }

    pub fn pointer_to(&mut self, pos: Point<f64, Logical>, time: u32) {
        let out = self.output_geometry().to_f64();
        let pos = Point::from((
            pos.x.clamp(out.loc.x, out.loc.x + out.size.w - 1.0),
            pos.y.clamp(out.loc.y, out.loc.y + out.size.h - 1.0),
        ));
        let pointer = self.seat.get_pointer().unwrap();
        let under = if self.deco_hit(pos).is_some() { None } else { self.surface_under(pos) };
        pointer.motion(self, under, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
        let hovered = self.lights_hovered(pos);
        if hovered != self.hovered_lights {
            self.hovered_lights = hovered;
            self.redraw();
        }
    }

    pub fn pointer_button(&mut self, button: u32, state: ButtonState, time: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        let pos = pointer.current_location();
        let serial = SERIAL_COUNTER.next_serial();
        let event = ButtonEvent { button, state, serial, time };

        if state == ButtonState::Released {
            if let Some((window, b)) = self.pressed.take() {
                self.redraw();
                // Like any button: it only fires if you let go on it.
                if self.deco_hit(pos).is_some_and(|(w, hit)| w == window && hit == Hit::Button(b)) {
                    match b {
                        Button::Close => self.close(&window),
                        Button::Minimize => self.minimize(&window),
                        Button::Zoom => self.zoom(&window),
                    }
                }
                return;
            }
        }

        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            match self.deco_hit(pos) {
                Some((window, Hit::Button(b))) => {
                    self.pressed = Some((window, b));
                    self.redraw();
                    return;
                }
                Some((window, Hit::Title)) => {
                    self.focus(Some(window.clone()));
                    let double = self
                        .last_title_click
                        .take()
                        .is_some_and(|(w, t)| w == window && t.elapsed() < DOUBLE_CLICK);
                    if double {
                        self.zoom(&window);
                    } else {
                        self.last_title_click = Some((window.clone(), Instant::now()));
                        if let Some(initial_loc) = self.space.element_location(&window) {
                            let start_data = GrabStartData { focus: None, button, location: pos };
                            pointer.set_grab(self, MoveGrab { start_data, window, initial_loc }, serial, Focus::Clear);
                        }
                    }
                }
                Some((window, Hit::Resize(edges))) => {
                    self.focus(Some(window.clone()));
                    if let Some(loc) = self.space.element_location(&window) {
                        let initial = Rectangle::new(loc, window.geometry().size);
                        let start_data = GrabStartData { focus: None, button, location: pos };
                        pointer.set_grab(self, ResizeGrab::start(start_data, window, edges, initial), serial, Focus::Clear);
                    }
                }
                None => {
                    let window = self.space.element_under(pos).map(|(w, _)| w.clone());
                    if window != self.focused_window() || window.is_none() {
                        self.focus(window);
                    }
                }
            }
        }
        pointer.button(self, &event);
        pointer.frame(self);
    }
}
