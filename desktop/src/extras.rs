//! Smaller protocols apps (and nested desktops like KWin) expect: one-color
//! buffers, raw pointer motion, pointer lock and confinement, and letting a
//! focused app take the keyboard shortcuts (a nested desktop or a remote
//! session wants Super for itself).

use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::pointer_constraints::{PointerConstraint, PointerConstraintsHandler, with_pointer_constraint};
use smithay::wayland::keyboard_shortcuts_inhibit::{
    KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
};
use smithay::{
    delegate_keyboard_shortcuts_inhibit, delegate_pointer_constraints, delegate_presentation, delegate_relative_pointer,
    delegate_single_pixel_buffer,
};

use crate::state::Desk;

impl KeyboardShortcutsInhibitHandler for Desk {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // Only while that window has the keyboard: the seat checks focus.
        inhibitor.activate();
    }
}

impl PointerConstraintsHandler for Desk {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if pointer.current_focus().as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |c| {
                if let Some(c) = c {
                    c.activate();
                }
            });
        }
    }

    fn cursor_position_hint(&mut self, _surface: &WlSurface, _pointer: &PointerHandle<Self>, _location: Point<f64, Logical>) {}
}

/// What an app has done to the pointer over it.
pub enum Constraint {
    None,
    /// The pointer stays put; only relative motion goes through.
    Locked,
    /// The pointer stays inside the window.
    Confined,
}

impl Desk {
    /// The constraint in force: only an active one, on the window that also
    /// has the keyboard, so switching away always frees the pointer.
    pub fn pointer_constraint(&self) -> Constraint {
        let pointer = self.seat.get_pointer().unwrap();
        let Some(surface) = pointer.current_focus() else { return Constraint::None };
        let owner = self.window_for_surface(&surface);
        if owner.is_none() || owner != self.focused_window() {
            return Constraint::None;
        }
        with_pointer_constraint(&surface, &pointer, |c| match c.as_deref() {
            Some(c @ PointerConstraint::Locked(_)) if c.is_active() => Constraint::Locked,
            Some(c @ PointerConstraint::Confined(_)) if c.is_active() => Constraint::Confined,
            _ => Constraint::None,
        })
    }
}

delegate_pointer_constraints!(Desk);
delegate_presentation!(Desk);
delegate_single_pixel_buffer!(Desk);
delegate_relative_pointer!(Desk);
delegate_keyboard_shortcuts_inhibit!(Desk);
