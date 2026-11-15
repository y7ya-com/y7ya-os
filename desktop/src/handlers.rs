//! Wayland protocol plumbing: which protocols we speak and how windows come
//! and go.


use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::desktop::{
    PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy, Window, find_popup_root_surface,
    get_popup_toplevel_coords,
};
use smithay::input::pointer::{CursorImageStatus, Focus};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecoMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_seat, wl_surface::WlSurface};
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::{Rectangle, Serial};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{CompositorClientState, CompositorHandler, CompositorState, is_sync_subsurface, with_states};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler, set_data_device_focus,
};
use smithay::wayland::selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState, set_primary_focus};
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_primary_selection, delegate_seat,
    delegate_shm, delegate_viewporter, delegate_xdg_decoration, delegate_xdg_shell,
};

use crate::grabs::{self, MoveGrab, ResizeGrab};
use crate::state::{ClientState, Desk, win_data};

impl CompositorHandler for Desk {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.send_preferred_scale(surface);
        self.redraw();
        if is_sync_subsurface(surface) {
            return;
        }
        if self.commit_layer(surface) {
            return;
        }
        if let Some(window) = self.window_for_surface(surface) {
            window.on_commit();
            let toplevel = window.toplevel().unwrap();
            if toplevel.wl_surface() == surface {
                let initial_sent = with_states(surface, |states| {
                    states.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap().initial_configure_sent
                });
                if !initial_sent {
                    toplevel.send_configure();
                } else if let Some(i) = self.unmapped.iter().position(|w| w == &window) {
                    // First real frame: show it, centered, with an open animation.
                    let has_buffer = with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
                    if has_buffer {
                        self.unmapped.remove(i);
                        let (r, d) = crate::state::OPEN_SPRING;
                        win_data(&window).borrow_mut().open = Some(crate::anim::Anim::spring(0.0, 1.0, r, d));
                        self.place(&window, None);
                        self.focus(Some(window.clone()));
                    }
                }
            }
        }
        grabs::handle_commit(self, surface);

        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
            if !popup.is_initial_configure_sent() {
                let _ = popup.send_configure();
            }
        }
    }
}

impl BufferHandler for Desk {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Desk {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for Desk {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Desk> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
        if self.draws_cursor {
            self.redraw();
        }
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }
}

impl SelectionHandler for Desk {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Desk {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for Desk {}
impl ServerDndGrabHandler for Desk {}

impl PrimarySelectionHandler for Desk {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl OutputHandler for Desk {}

impl XdgShellHandler for Desk {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Mac windows are decorated by the system unless the app draws its own.
        surface.with_pending_state(|s| s.decoration_mode = Some(DecoMode::ServerSide));
        self.unmapped.push(Window::new_wayland_window(surface));
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let gone = |w: &Window| w.toplevel().is_some_and(|t| t == &surface);
        let mapped = self.space.elements().find(|w| gone(w)).cloned();
        if let Some(w) = mapped {
            self.space.unmap_elem(&w);
            let (r, d) = crate::state::CLOSE_SPRING;
            self.closing.push(crate::state::Closing { window: w, anim: crate::anim::Anim::spring(0.0, 1.0, r, d) });
        }
        self.minimized.retain(|w| !gone(w));
        self.unmapped.retain(|w| !gone(w));
        if self.hovered_lights.as_ref().is_some_and(|w| gone(w)) {
            self.hovered_lights = None;
        }
        if self.focused_window().is_none() {
            self.focus_top();
        }
        self.redraw();
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|s| {
            s.geometry = positioner.get_geometry();
            s.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::<Desk>::from_resource(&seat) else { return };
        let Some(start_data) = check_grab(&seat, surface.wl_surface(), serial) else { return };
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(initial_loc) = self.space.element_location(&window) else { return };
        let pointer = seat.get_pointer().unwrap();
        pointer.set_grab(self, MoveGrab { start_data, window, initial_loc }, serial, Focus::Clear);
    }

    fn resize_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial, edges: xdg_toplevel::ResizeEdge) {
        let Some(seat) = Seat::<Desk>::from_resource(&seat) else { return };
        let Some(start_data) = check_grab(&seat, surface.wl_surface(), serial) else { return };
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(loc) = self.space.element_location(&window) else { return };
        let initial = Rectangle::new(loc, window.geometry().size);
        let pointer = seat.get_pointer().unwrap();
        pointer.set_grab(self, ResizeGrab::start(start_data, window, edges.into(), initial), serial, Focus::Clear);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            if win_data(&window).borrow().restore.is_none() {
                self.zoom(&window);
            }
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            if win_data(&window).borrow().restore.is_some() {
                self.zoom(&window);
            }
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.minimize(&window);
        }
    }

    fn title_changed(&mut self, _surface: ToplevelSurface) {
        self.redraw();
    }

    /// Menus grab input so a click outside closes them.
    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::<Desk>::from_resource(&seat) else { return };
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else { return };
        let Ok(mut grab) = self.popups.grab_popup(root, kind, &seat, serial) else { return };
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed() && !(keyboard.has_grab(serial) || keyboard.has_grab(grab.previous_serial().unwrap_or(serial))) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed() && !(pointer.has_grab(serial) || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial()))) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }
}

impl XdgDecorationHandler for Desk {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecoMode::ServerSide));
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecoMode) {
        // Apps that draw their own Mac-style toolbar (ours, Firefox...) keep
        // it; everything else gets our title bar.
        toplevel.with_pending_state(|s| s.decoration_mode = Some(mode));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecoMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

impl Desk {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else { return };
        let Some(window) = self.window_for_surface(&root) else { return };
        let Some(window_geo) = self.space.element_geometry(&window) else { return };
        let mut target = self.output_geometry();
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;
        popup.with_pending_state(|s| s.geometry = s.positioner.get_unconstrained_geometry(target));
    }
}

fn check_grab(
    seat: &Seat<Desk>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<smithay::input::pointer::GrabStartData<Desk>> {
    let pointer = seat.get_pointer()?;
    if !pointer.has_grab(serial) {
        return None;
    }
    let start_data = pointer.grab_start_data()?;
    let (focus, _) = start_data.focus.as_ref()?;
    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }
    Some(start_data)
}

delegate_compositor!(Desk);
delegate_shm!(Desk);
delegate_seat!(Desk);
delegate_data_device!(Desk);
delegate_primary_selection!(Desk);
delegate_output!(Desk);
delegate_xdg_shell!(Desk);
delegate_xdg_decoration!(Desk);
delegate_viewporter!(Desk);
