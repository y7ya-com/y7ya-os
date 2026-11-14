use std::cell::RefCell;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Instant;

use smithay::desktop::{PopupManager, Space, Window, WindowSurfaceType};
use smithay::input::{Seat, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecoMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Size};
use smithay::wayland::compositor::{CompositorClientState, CompositorState, get_parent};
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufState};
use smithay::wayland::selection::wlr_data_control::DataControlState;
use smithay::wayland::shell::wlr_layer::{Layer, WlrLayerShellState};
use smithay::wayland::viewporter::ViewporterState;

use crate::anim::Anim;
use crate::grabs::Edges;

/// A window flying into its taskbar button.
pub struct Minimizing {
    pub window: Window,
    pub loc: Point<i32, Logical>,
    pub from: Rectangle<f64, Logical>,
    pub to: Rectangle<f64, Logical>,
    pub anim: Anim,
}

/// Spring settings (response in seconds, damping ratio) for window motion.
pub const OPEN_SPRING: (f64, f64) = (0.36, 0.8);
pub const MINIMIZE_SPRING: (f64, f64) = (0.42, 1.0);
pub const ZOOM_SPRING: (f64, f64) = (0.38, 0.92);
/// Margin around and between tiled windows (macOS keeps a little air).
pub const TILE_GAP: i32 = 8;

/// Height of the title bar drawn above windows that let us decorate them.
pub const TITLEBAR_H: i32 = 32;
/// Window corner radius (macOS 26 uses generous radii).
pub const RADIUS: f32 = 14.0;
/// How far outside a window's edge the resize handles reach.
const RESIZE_BAND: i32 = 6;
/// Traffic lights: first center from the left edge, spacing between centers, radius.
pub const LIGHT_X: f64 = 18.0;
pub const LIGHT_GAP: f64 = 20.0;
pub const LIGHT_R: f64 = 6.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Close,
    Minimize,
    Zoom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Button(Button),
    Title,
    Resize(Edges),
}

/// Per-window bookkeeping, kept in the window's user data.
#[derive(Default)]
pub struct WinData {
    /// The opening animation (0 → 1), while it runs.
    pub open: Option<Anim>,
    /// The window is shown morphing from one frame to another (zoom, coming
    /// back from the taskbar), with whether it also fades in.
    pub morph: Option<(Rectangle<f64, Logical>, Rectangle<f64, Logical>, Anim, bool)>,
    /// Where its taskbar button is, so minimizing can fly into it.
    pub taskbar_rect: Option<Rectangle<i32, Logical>>,
    /// Where the window was before zooming, to restore it.
    pub restore: Option<Rectangle<i32, Logical>>,
    /// Where a minimized window was.
    pub parked_at: Option<Point<i32, Logical>>,
}

#[allow(dead_code)] // Some protocol states only need to stay alive.
pub struct Desk {
    pub start: Instant,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, Desk>,
    pub loop_signal: LoopSignal,
    pub socket_name: OsString,

    pub space: Space<Window>,
    pub popups: PopupManager,
    pub minimized: Vec<Window>,
    pub minimizing: Vec<Minimizing>,
    /// Where a window being dragged would tile if dropped now.
    pub snap_preview: Option<Rectangle<i32, Logical>>,
    /// New windows wait here until their first frame arrives.
    pub unmapped: Vec<Window>,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Desk>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub viewporter_state: ViewporterState,
    pub layer_shell_state: WlrLayerShellState,
    pub fractional_scale_state: smithay::wayland::fractional_scale::FractionalScaleManagerState,
    pub activation_state: smithay::wayland::xdg_activation::XdgActivationState,
    pub data_control_state: DataControlState,
    pub dmabuf: Option<(DmabufState, DmabufGlobal)>,
    pub seat: Seat<Desk>,
    /// A layer surface (like Spotlight) holding the keyboard.
    pub layer_focus: Option<WlSurface>,
    pub toplevels: crate::toplevels::ForeignToplevels,
    pub spaces: crate::overview::Spaces,

    pub output: Option<Output>,
    pub dark: bool,
    /// The traffic lights under the pointer (they show their glyphs), and the
    /// one being pressed.
    pub hovered_lights: Option<Window>,
    pub pressed: Option<(Window, Button)>,
    pub last_title_click: Option<(Window, Instant)>,
    pub needs_redraw: bool,
    /// The X11 display served by xwayland-satellite, for X11-only apps.
    pub x_display: Option<String>,
    /// What the pointer looks like (a client's surface or a named cursor).
    pub cursor_status: smithay::input::pointer::CursorImageStatus,
    /// Whether we draw the pointer ourselves (true in the real session).
    pub draws_cursor: bool,
    /// Switches virtual terminals (only in the real session).
    pub vt_switch: Option<Box<dyn FnMut(i32)>>,
    pub settings_watcher: Option<settings::Watcher>,
    /// Running in a window inside another desktop.
    pub nested: bool,
    pub wallpaper_path: String,
    pub wallpaper: Option<std::sync::Arc<crate::config::WallpaperImage>>,
}

impl Desk {
    pub fn new(display: Display<Desk>, loop_handle: LoopHandle<'static, Desk>, loop_signal: LoopSignal) -> Self {
        let dh = display.handle();
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let fractional_scale_state = smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(&dh);
        let activation_state = smithay::wayland::xdg_activation::XdgActivationState::new::<Self>(&dh);
        let toplevels = crate::toplevels::ForeignToplevels::init(&dh);
        let data_control_state = DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        seat.add_keyboard(Default::default(), 300, 30).expect("keyboard");
        seat.add_pointer();

        let socket = ListeningSocketSource::new_auto().expect("wayland socket");
        let socket_name = socket.socket_name().to_os_string();
        loop_handle
            .insert_source(socket, |stream, _, desk: &mut Desk| {
                if let Err(e) = desk.display_handle.insert_client(stream, Arc::new(ClientState::default())) {
                    tracing::warn!("client failed to connect: {e}");
                }
            })
            .expect("socket source");
        loop_handle
            .insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, desk: &mut Desk| {
                // Safety: the display is never dropped while the loop runs.
                unsafe {
                    display.get_mut().dispatch_clients(desk).unwrap();
                }
                Ok(PostAction::Continue)
            })
            .expect("display source");

        Desk {
            start: Instant::now(),
            display_handle: dh.clone(),
            loop_handle,
            loop_signal,
            socket_name,
            space: Space::default(),
            popups: PopupManager::default(),
            minimized: Vec::new(),
            minimizing: Vec::new(),
            snap_preview: None,
            unmapped: Vec::new(),
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            viewporter_state,
            layer_shell_state,
            fractional_scale_state,
            activation_state,
            data_control_state,
            dmabuf: None,
            seat,
            layer_focus: None,
            toplevels,
            spaces: Default::default(),
            output: None,
            dark: detect_dark(),
            hovered_lights: None,
            pressed: None,
            last_title_click: None,
            needs_redraw: true,
            x_display: None,
            cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
            draws_cursor: false,
            vt_switch: None,
            settings_watcher: None,
            nested: false,
            wallpaper_path: String::new(),
            wallpaper: None,
        }
    }

    pub fn redraw(&mut self) {
        self.needs_redraw = true;
    }

    pub fn output_geometry(&self) -> Rectangle<i32, Logical> {
        self.output
            .as_ref()
            .and_then(|o| self.space.output_geometry(o))
            .unwrap_or_else(|| Rectangle::from_size((1280, 800).into()))
    }

    /// The window whose toplevel owns `surface` (directly or via subsurfaces).
    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        self.space
            .elements()
            .chain(self.minimized.iter())
            .chain(self.unmapped.iter())
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &root))
            .cloned()
    }

    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.layer_under(&[Layer::Overlay, Layer::Top], pos)
            .or_else(|| {
                self.space.element_under(pos).and_then(|(window, loc)| {
                    window
                        .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                        .map(|(s, p)| (s, (p + loc).to_f64()))
                })
            })
            .or_else(|| self.layer_under(&[Layer::Bottom, Layer::Background], pos))
    }

    pub fn focused_window(&self) -> Option<Window> {
        let surface = self.seat.get_keyboard()?.current_focus()?;
        self.window_for_surface(&surface)
    }

    /// Smallest y for a window's content, so its title bar stays on screen.
    pub fn min_content_y(&self, window: &Window) -> i32 {
        self.usable_area().loc.y + if is_ssd(window) { TITLEBAR_H } else { 0 }
    }

    /// The window's visible frame: its content plus our title bar.
    pub fn frame(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        let loc = self.space.element_location(window)?;
        Some(frame_of(window, loc))
    }

    /// What the pointer would hit in our window chrome at `pos`, if anything.
    pub fn deco_hit(&self, pos: Point<f64, Logical>) -> Option<(Window, Hit)> {
        for window in self.space.elements().rev() {
            let Some(frame) = self.frame(window) else { continue };
            if !is_ssd(window) {
                // Client-drawn windows handle their own edges; just stop at them.
                if self.space.element_bbox(window).is_some_and(|b| b.to_f64().contains(pos)) {
                    return None;
                }
                continue;
            }
            let band = Rectangle::new(
                frame.loc - Point::from((RESIZE_BAND, RESIZE_BAND)),
                frame.size + Size::from((2 * RESIZE_BAND, 2 * RESIZE_BAND)),
            );
            if !band.to_f64().contains(pos) {
                continue;
            }
            let f = frame.to_f64();
            let inside = f.contains(pos);
            // Corners get a slightly larger grab area, like macOS.
            let corner = 12.0;
            let mut edges = Edges::empty();
            if pos.x < f.loc.x || (!inside || pos.y > f.loc.y + f.size.h - 3.0) && pos.x < f.loc.x + corner {
                edges |= Edges::LEFT;
            }
            if pos.x >= f.loc.x + f.size.w || (!inside || pos.y > f.loc.y + f.size.h - 3.0) && pos.x > f.loc.x + f.size.w - corner {
                edges |= Edges::RIGHT;
            }
            if pos.y < f.loc.y {
                edges |= Edges::TOP;
            }
            if pos.y >= f.loc.y + f.size.h || (pos.y > f.loc.y + f.size.h - 3.0) {
                edges |= Edges::BOTTOM;
            }
            if !inside || edges.contains(Edges::BOTTOM) {
                if edges.is_empty() {
                    continue;
                }
                return Some((window.clone(), Hit::Resize(edges)));
            }
            if pos.y < f.loc.y + TITLEBAR_H as f64 {
                let local = pos - f.loc;
                for (i, button) in [Button::Close, Button::Minimize, Button::Zoom].into_iter().enumerate() {
                    let c = Point::<f64, Logical>::from((LIGHT_X + i as f64 * LIGHT_GAP, TITLEBAR_H as f64 / 2.0));
                    let d = local - c;
                    if (d.x * d.x + d.y * d.y).sqrt() <= LIGHT_R + 2.0 {
                        return Some((window.clone(), Hit::Button(button)));
                    }
                }
                return Some((window.clone(), Hit::Title));
            }
            return None;
        }
        None
    }

    /// Whether the pointer is over a window's traffic-light group.
    pub fn lights_hovered(&self, pos: Point<f64, Logical>) -> Option<Window> {
        let window = self.space.element_under(pos).map(|(w, _)| w.clone()).or_else(|| {
            self.space.elements().rev().find(|w| self.frame(w).is_some_and(|f| f.to_f64().contains(pos))).cloned()
        })?;
        if !is_ssd(&window) {
            return None;
        }
        let f = self.frame(&window)?.to_f64();
        let local = pos - f.loc;
        let group = Rectangle::<f64, Logical>::new(
            (LIGHT_X - LIGHT_R - 4.0, 0.0).into(),
            (2.0 * LIGHT_GAP + 2.0 * LIGHT_R + 8.0, TITLEBAR_H as f64).into(),
        );
        group.contains(local).then_some(window)
    }

    pub fn focus(&mut self, window: Option<Window>) {
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(w) = &window {
            self.space.raise_element(w, true);
            self.note_focus(w);
        }
        for w in self.space.elements() {
            let active = window.as_ref() == Some(w);
            if w.set_activated(active) {
                if let Some(t) = w.toplevel() {
                    t.send_pending_configure();
                }
            }
        }
        let surface = window.and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()));
        let surface = self.layer_focus.clone().or(surface);
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, surface, serial);
        self.redraw();
    }

    /// Focus the top-most visible window, if any.
    pub fn focus_top(&mut self) {
        let top = self.space.elements().last().cloned();
        self.focus(top);
    }

    pub fn close(&mut self, window: &Window) {
        if let Some(t) = window.toplevel() {
            t.send_close();
        }
    }

    pub fn minimize(&mut self, window: &Window) {
        let loc = self.space.element_location(window);
        win_data(window).borrow_mut().parked_at = loc;
        if let Some(loc) = loc {
            let from = frame_of(window, loc).to_f64();
            let to = self.taskbar_target(window);
            let (r, d) = MINIMIZE_SPRING;
            self.minimizing.push(Minimizing { window: window.clone(), loc, from, to, anim: Anim::spring(0.0, 1.0, r, d) });
        }
        self.space.unmap_elem(window);
        self.minimized.push(window.clone());
        self.focus_top();
    }

    /// Brings back the most recently minimized window.
    pub fn unminimize_last(&mut self) {
        if let Some(window) = self.minimized.pop() {
            self.restore(window);
        }
    }

    /// Puts a minimized window back, growing it out of its taskbar button.
    pub fn restore(&mut self, window: Window) {
        self.minimized.retain(|w| w != &window);
        self.minimizing.retain(|m| m.window != window);
        let loc = win_data(&window).borrow_mut().parked_at.take();
        self.place(&window, loc);
        if let Some(loc) = self.space.element_location(&window) {
            let from = self.taskbar_target(&window);
            let to = frame_of(&window, loc).to_f64();
            let (r, d) = MINIMIZE_SPRING;
            win_data(&window).borrow_mut().morph = Some((from, to, Anim::spring(0.0, 1.0, r, d), true));
        }
        self.focus(Some(window));
    }

    /// Where a window shrinks to when minimized: its taskbar button, or the
    /// middle of the bottom edge.
    fn taskbar_target(&self, window: &Window) -> Rectangle<f64, Logical> {
        if let Some(r) = win_data(window).borrow().taskbar_rect {
            return r.to_f64();
        }
        let out = self.output_geometry().to_f64();
        Rectangle::new((out.loc.x + out.size.w / 2.0 - 24.0, out.loc.y + out.size.h - 44.0).into(), (48.0, 36.0).into())
    }

    /// Fills the screen with the window, or puts it back where it was.
    pub fn zoom(&mut self, window: &Window) {
        let Some(toplevel) = window.toplevel() else { return };
        let data = win_data(window);
        let before = self.space.element_location(window).map(|l| frame_of(window, l).to_f64());
        let restore = data.borrow_mut().restore.take();
        match restore {
            Some(rect) => {
                toplevel.with_pending_state(|s| {
                    s.states.unset(xdg_toplevel::State::Maximized);
                    s.size = Some(rect.size);
                });
                self.space.map_element(window.clone(), rect.loc, true);
            }
            None => {
                let Some(loc) = self.space.element_location(window) else { return };
                data.borrow_mut().restore = Some(Rectangle::new(loc, window.geometry().size));
                let out = self.usable_area();
                let top = if is_ssd(window) { TITLEBAR_H } else { 0 };
                toplevel.with_pending_state(|s| {
                    s.states.set(xdg_toplevel::State::Maximized);
                    s.size = Some((out.size.w, out.size.h - top).into());
                });
                self.space.map_element(window.clone(), (out.loc.x, out.loc.y + top), true);
            }
        }
        toplevel.send_pending_configure();
        // Grow (or shrink) smoothly to the new frame while the app resizes.
        if let (Some(from), Some(loc)) = (before, self.space.element_location(window)) {
            let size = toplevel.with_pending_state(|s| s.size).unwrap_or(window.geometry().size);
            let top = if is_ssd(window) { TITLEBAR_H } else { 0 };
            let to = Rectangle::<i32, Logical>::new((loc.x, loc.y - top).into(), (size.w, size.h + top).into()).to_f64();
            let (r, d) = ZOOM_SPRING;
            data.borrow_mut().morph = Some((from, to, Anim::spring(0.0, 1.0, r, d), false));
        }
        self.redraw();
    }

    /// The tiling target for a window dragged to `p`: the left or right half
    /// at the side edges, the whole screen at the top.
    pub fn snap_zone(&self, p: Point<f64, Logical>) -> Option<Rectangle<i32, Logical>> {
        let a = self.usable_area();
        let edge = 6.0;
        let gap = TILE_GAP;
        let inner = Rectangle::<i32, Logical>::new(
            (a.loc.x + gap, a.loc.y + gap).into(),
            (a.size.w - 2 * gap, a.size.h - 2 * gap).into(),
        );
        let half = (inner.size.w - gap) / 2;
        if p.x <= a.loc.x as f64 + edge {
            Some(Rectangle::new(inner.loc, (half, inner.size.h).into()))
        } else if p.x >= (a.loc.x + a.size.w) as f64 - edge {
            Some(Rectangle::new((inner.loc.x + half + gap, inner.loc.y).into(), (half, inner.size.h).into()))
        } else if p.y <= a.loc.y as f64 + edge {
            Some(inner)
        } else {
            None
        }
    }

    /// Fits a window's frame into `rect` (from [`snap_zone`]).
    pub fn tile(&mut self, window: &Window, rect: Rectangle<i32, Logical>) {
        let Some(toplevel) = window.toplevel() else { return };
        let Some(loc) = self.space.element_location(window) else { return };
        let data = win_data(window);
        let from = frame_of(window, loc).to_f64();
        if data.borrow().restore.is_none() {
            data.borrow_mut().restore = Some(Rectangle::new(loc, window.geometry().size));
        }
        let top = if is_ssd(window) { TITLEBAR_H } else { 0 };
        toplevel.with_pending_state(|s| {
            s.states.set(xdg_toplevel::State::TiledLeft);
            s.states.set(xdg_toplevel::State::TiledRight);
            s.states.set(xdg_toplevel::State::TiledTop);
            s.states.set(xdg_toplevel::State::TiledBottom);
            s.size = Some((rect.size.w, rect.size.h - top).into());
        });
        toplevel.send_pending_configure();
        self.space.map_element(window.clone(), (rect.loc.x, rect.loc.y + top), true);
        let (r, d) = ZOOM_SPRING;
        data.borrow_mut().morph = Some((from, rect.to_f64(), Anim::spring(0.0, 1.0, r, d), false));
        self.focus(Some(window.clone()));
    }

    /// Gives a tiled or zoomed window its own size back while it's being
    /// dragged by `pointer`. Returns its new location if it changed.
    pub fn untile(&mut self, window: &Window, pointer: Point<f64, Logical>) -> Option<Point<i32, Logical>> {
        let restore = win_data(window).borrow_mut().restore.take()?;
        let toplevel = window.toplevel()?;
        let loc = self.space.element_location(window)?;
        let width = window.geometry().size.w.max(1) as f64;
        // Keep the grab point at the same fraction across the title bar.
        let frac = ((pointer.x - loc.x as f64) / width).clamp(0.0, 1.0);
        toplevel.with_pending_state(|s| {
            for st in [
                xdg_toplevel::State::Maximized,
                xdg_toplevel::State::TiledLeft,
                xdg_toplevel::State::TiledRight,
                xdg_toplevel::State::TiledTop,
                xdg_toplevel::State::TiledBottom,
            ] {
                s.states.unset(st);
            }
            s.size = Some(restore.size);
        });
        toplevel.send_pending_configure();
        let new_loc = Point::from(((pointer.x - frac * restore.size.w as f64) as i32, loc.y));
        self.space.map_element(window.clone(), new_loc, true);
        Some(new_loc)
    }

    /// Maps a window, cascading new ones from the top-left like macOS.
    pub fn place(&mut self, window: &Window, at: Option<Point<i32, Logical>>) {
        let out = self.usable_area();
        let loc = at.unwrap_or_else(|| {
            let size = window.geometry().size;
            let n = self.space.elements().count() as i32;
            let x = out.loc.x + (out.size.w - size.w) / 2 + (n % 6) * 26 - 65;
            let y = out.loc.y + ((out.size.h - size.h) / 3).max(TITLEBAR_H + 8) + (n % 6) * 26 - 40;
            (x.max(out.loc.x), y.max(out.loc.y + TITLEBAR_H)).into()
        });
        self.space.map_element(window.clone(), loc, true);
    }

    /// Tells clients which pixel formats shared-memory buffers may use.
    pub fn shm_formats(&mut self, formats: impl IntoIterator<Item = smithay::reexports::wayland_server::protocol::wl_shm::Format>) {
        self.shm_state.update_formats(formats);
    }

    /// Starts xwayland-satellite (if installed) so X11-only apps can run.
    pub fn start_xwayland(&mut self) {
        let found = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("xwayland-satellite").exists()));
        if !found {
            return;
        }
        // The first X display number nobody is using.
        let Some(n) = (1..64).find(|n| !std::path::Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()) else { return };
        let display = format!(":{n}");
        let started = std::process::Command::new("xwayland-satellite")
            .arg(&display)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env_remove("DISPLAY")
            .spawn();
        match started {
            Ok(_) => self.x_display = Some(display),
            Err(e) => tracing::warn!("couldn't start xwayland-satellite: {e}"),
        }
    }

    /// Runs `cmd` as a client of this desktop.
    pub fn spawn(&self, cmd: &str) {
        let mut parts = cmd.split_whitespace();
        let Some(program) = parts.next() else { return };
        let result = std::process::Command::new(program)
            .args(parts)
            // Qt, GTK and cursors in the Mac look, in the current appearance.
            .envs(settings::look::env(self.dark))
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env_remove("DISPLAY")
            .envs(self.x_display.iter().map(|d| ("DISPLAY", d.as_str())))
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XDG_CURRENT_DESKTOP", "y7ya-os")
            .env("QT_QPA_PLATFORM", "wayland")
            .env("GDK_BACKEND", "wayland")
            .env("MOZ_ENABLE_WAYLAND", "1")
            .env("ELECTRON_OZONE_PLATFORM_HINT", "wayland")
            .spawn();
        if let Err(e) = result {
            tracing::warn!("couldn't start {cmd}: {e}");
        }
    }
}

pub fn win_data(window: &Window) -> &RefCell<WinData> {
    window.user_data().insert_if_missing(|| RefCell::new(WinData::default()));
    window.user_data().get::<RefCell<WinData>>().unwrap()
}

/// Whether we draw this window's title bar (it asked for server-side decorations).
pub fn is_ssd(window: &Window) -> bool {
    window.toplevel().is_some_and(|t| {
        t.current_state().decoration_mode == Some(DecoMode::ServerSide)
    })
}

pub fn frame_of(window: &Window, loc: Point<i32, Logical>) -> Rectangle<i32, Logical> {
    let size = window.geometry().size;
    if is_ssd(window) {
        Rectangle::new((loc.x, loc.y - TITLEBAR_H).into(), (size.w, size.h + TITLEBAR_H).into())
    } else {
        Rectangle::new(loc, size)
    }
}

/// KDE's color scheme says whether the user prefers dark.
pub fn detect_dark() -> bool {
    if let Ok(theme) = std::env::var("Y7_THEME") {
        return theme == "dark";
    }
    let path = dirs::config_dir().map(|d| d.join("kdeglobals"));
    let Some(s) = path.and_then(|p| std::fs::read_to_string(p).ok()) else { return true };
    s.lines().find_map(|l| l.strip_prefix("ColorScheme=")).is_some_and(|v| v.to_lowercase().contains("dark"))
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
