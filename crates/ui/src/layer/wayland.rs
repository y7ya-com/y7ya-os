//! The layer-shell runner: smithay-client-toolkit for Wayland, one wgpu device
//! shared by every surface, and the same element tree and input handling as
//! the winit runner. Everything happens on one calloop loop that sleeps until
//! Wayland input, a `Waker`, or the app's `next_wake` timer.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle,
    WaylandWindowHandle,
};
use smithay_client_toolkit as sctk;
use sctk::compositor::{CompositorHandler, CompositorState};
use sctk::output::{OutputHandler, OutputState};
use sctk::reexports::calloop::EventLoop;
use sctk::reexports::calloop::ping::make_ping;
use sctk::reexports::calloop_wayland_source::WaylandSource;
use sctk::reexports::client::globals::registry_queue_init;
use sctk::reexports::client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface::WlSurface};
use sctk::reexports::client::{Connection, Proxy, QueueHandle};
use sctk::registry::{ProvidesRegistryState, RegistryState};
use sctk::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers};
use sctk::seat::pointer::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer};
use sctk::seat::{Capability, SeatHandler, SeatState};
use sctk::shell::WaylandSurface;
use sctk::shell::wlr_layer::{self as wlr, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure};
use sctk::shm::{Shm, ShmHandler};
use sctk::{
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, registry_handlers,
};
use winit::window::CursorIcon;

use super::{Anchor, KeyboardInteractivity, Layer, LayerCommand, LayerOptions, keys};
use crate::app::App;
use crate::cx::{CARET_BLINK_MS, Cx, WindowCommand};
use crate::element::{AnyElement, ElementId, Frame, Handler, Hitbox, render_tree};
use crate::geom::{Bounds, Point};
use crate::input::{KeyPress, TextInput};
use crate::renderer::Renderer;
use crate::scene::{Instance, Scene};

type Target = (wgpu::Surface<'static>, Option<wgpu::SurfaceConfiguration>);

struct Surface<A> {
    // Declared first so the GPU surface goes before the Wayland surface it draws to.
    target: Option<Target>,
    layer: LayerSurface,
    opts: LayerOptions,
    /// Logical size from the last configure.
    size: (u32, u32),
    scale: i32,
    buffer_scale: i32,
    configured: bool,
    dirty: bool,
    scene: Scene,
    frame: Frame<A>,
    /// What's on screen, to skip presenting an identical frame.
    shown: Vec<Instance>,
}

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    renderer: Renderer,
}

struct Runner<A: App> {
    conn: Connection,
    qh: QueueHandle<Self>,
    loop_handle: sctk::reexports::calloop::LoopHandle<'static, Self>,
    registry: RegistryState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    seat_state: SeatState,
    output_state: OutputState,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<ThemedPointer>,
    app: Option<A>,
    cx: Cx,
    gpu: Option<Gpu>,
    surfaces: Vec<Surface<A>>,
    pointer_on: Option<WlSurface>,
    keyboard_on: Option<WlSurface>,
    modifiers: Modifiers,
    pressed_click: Option<(Handler<A>, Bounds)>,
    pressed_input: Option<(Rc<RefCell<TextInput>>, Bounds)>,
    last_click: (Instant, Point, u32),
    cursor: CursorIcon,
    /// The app timer and caret blink deadlines the loop last slept toward.
    wake_at: Option<Instant>,
    blink_at: Option<Instant>,
    exit: bool,
}

/// Runs `make`'s app on a layer surface described by `opts`. The app can open
/// more surfaces with [`Cx::show_layer`]. Returns when the app closes
/// (`cx.close_window()`) or the compositor goes away.
pub fn run_layer<A: App>(opts: LayerOptions, make: impl FnOnce(&mut Cx) -> A + 'static) {
    let conn = Connection::connect_to_env().expect("no Wayland compositor");
    let (globals, mut queue) = registry_queue_init::<Runner<A>>(&conn).expect("wayland registry");
    let qh = queue.handle();
    let mut event_loop: EventLoop<'static, Runner<A>> = EventLoop::try_new().expect("event loop");
    let (ping, ping_source) = make_ping().expect("ping");

    let mut runner = Runner {
        conn: conn.clone(),
        qh: qh.clone(),
        loop_handle: event_loop.handle(),
        registry: RegistryState::new(&globals),
        compositor: CompositorState::bind(&globals, &qh).expect("wl_compositor"),
        layer_shell: LayerShell::bind(&globals, &qh).expect("the compositor doesn't support wlr-layer-shell"),
        shm: Shm::bind(&globals, &qh).expect("wl_shm"),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        keyboard: None,
        pointer: None,
        app: None,
        cx: Cx::new(),
        gpu: None,
        surfaces: Vec::new(),
        pointer_on: None,
        keyboard_on: None,
        modifiers: Modifiers::default(),
        pressed_click: None,
        pressed_input: None,
        last_click: (Instant::now() - Duration::from_secs(10), Point::default(), 0),
        cursor: CursorIcon::Default,
        wake_at: None,
        blink_at: None,
        exit: false,
    };
    runner.cx.waker = crate::cx::Waker(None, Some(Arc::new(move || ping.ping())));
    runner.cx.dark = super::system_dark();
    runner.cx.transparent = true;
    runner.cx.layers.conn = Some(conn.clone());
    // Learn the outputs (for their scale) and seats before the first surface.
    queue.roundtrip(&mut runner).expect("wayland roundtrip");

    WaylandSource::new(conn, queue).insert(event_loop.handle()).expect("wayland source");
    event_loop
        .handle()
        .insert_source(ping_source, |_, _, r: &mut Runner<A>| r.external())
        .expect("ping source");

    runner.cx.show_layer(opts);
    runner.app = Some(make(&mut runner.cx));
    while !runner.exit {
        runner.tick();
        // Drawing can read events too (wgpu waits on the compositor).
        runner.poll_queues();
        let timeout = runner.timeout();
        if let Err(e) = event_loop.dispatch(timeout, &mut runner) {
            eprintln!("wayland: {e}");
            break;
        }
        runner.poll_queues();
    }
    if let Some(app) = runner.app.as_mut() {
        app.on_exit();
    }
}

impl<A: App> Runner<A> {
    fn index(&self, surface: &WlSurface) -> Option<usize> {
        self.surfaces.iter().position(|s| s.layer.wl_surface() == surface)
    }

    /// Points `cx` at surface `i` before rendering it or handing it input.
    fn enter(&mut self, i: usize) {
        let s = &self.surfaces[i];
        self.cx.window_size = (s.size.0 as f32, s.size.1 as f32);
        self.cx.scale = s.scale as f32;
        self.cx.layers.current.clone_from(&s.opts.namespace);
        self.cx.window_focused = self.keyboard_on.as_ref() == Some(s.layer.wl_surface());
    }

    /// Runs the app's own event queues on what was just read (see
    /// [`Cx::poll_after_events`]); any that changed something wake the app.
    fn poll_queues(&mut self) {
        let mut pollers = std::mem::take(&mut self.cx.layers.pollers);
        let woke = pollers.iter_mut().fold(false, |woke, p| p() || woke);
        pollers.append(&mut self.cx.layers.pollers);
        self.cx.layers.pollers = pollers;
        if woke {
            self.external();
        }
    }

    fn external(&mut self) {
        self.cx.layers.current.clear();
        if let Some(app) = self.app.as_mut() {
            app.on_external(&mut self.cx);
        }
        self.cx.redraw = true;
    }

    /// When the loop should wake up next: now if anything needs drawing,
    /// otherwise for the app's timer or the caret blink. Remembers both
    /// deadlines so `tick` knows which one woke it.
    fn timeout(&mut self) -> Option<Duration> {
        self.wake_at = self.app.as_ref().and_then(|a| a.next_wake());
        self.blink_at = self.next_blink();
        if self.cx.redraw || !self.cx.layers.commands.is_empty() || self.surfaces.iter().any(|s| s.dirty) {
            return Some(Duration::ZERO);
        }
        let wake = [self.wake_at, self.blink_at].into_iter().flatten().min()?;
        Some(wake.saturating_duration_since(Instant::now()))
    }

    fn next_blink(&self) -> Option<Instant> {
        let since = self.focused_input()?.0.borrow().last_activity;
        let n = (since.elapsed().as_millis() / CARET_BLINK_MS + 1) as u32;
        Some(since + Duration::from_millis(CARET_BLINK_MS as u64) * n)
    }

    /// The focused text input, if it's on the surface holding the keyboard.
    fn focused_input(&self) -> Option<(Rc<RefCell<TextInput>>, usize)> {
        let focused = self.cx.focused?;
        let i = self.index(self.keyboard_on.as_ref()?)?;
        let h = self.surfaces[i].frame.hitboxes.iter().find(|h| h.id == Some(focused) && h.input.is_some())?;
        Some((h.input.as_ref()?.0.clone(), i))
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if let Some(app) = self.app.as_mut() {
            if self.wake_at.take().is_some_and(|t| t <= now) {
                self.cx.layers.current.clear();
                app.on_wake(&mut self.cx);
                self.cx.redraw = true;
            }
        }
        if self.blink_at.take().is_some_and(|t| t <= now) {
            if let Some((_, i)) = self.focused_input() {
                self.surfaces[i].dirty = true;
            }
        }
        self.apply_commands();
        let mut drew = false;
        for i in 0..self.surfaces.len() {
            if std::mem::take(&mut self.surfaces[i].dirty) {
                self.draw(i);
                drew = true;
            }
        }
        if drew {
            self.cx.text.end_frame();
        }
        let _ = self.conn.flush();
    }

    fn apply_commands(&mut self) {
        for cmd in std::mem::take(&mut self.cx.commands) {
            if let WindowCommand::Close = cmd {
                self.exit = true;
            }
        }
        for cmd in std::mem::take(&mut self.cx.layers.commands) {
            match cmd {
                LayerCommand::Show(opts) => self.show(opts),
                LayerCommand::Hide(ns) => {
                    if let Some(i) = self.surfaces.iter().position(|s| s.opts.namespace == ns) {
                        self.remove(i);
                    }
                }
            }
        }
        if std::mem::take(&mut self.cx.redraw) {
            self.surfaces.iter_mut().for_each(|s| s.dirty = true);
        }
    }

    fn show(&mut self, opts: LayerOptions) {
        if let Some(i) = self.surfaces.iter().position(|s| s.opts.namespace == opts.namespace) {
            if self.surfaces[i].opts.layer == opts.layer {
                let s = &mut self.surfaces[i];
                if s.opts != opts {
                    apply_options(&s.layer, &opts);
                    s.layer.commit();
                    s.opts = opts;
                }
                s.dirty = true;
                return;
            }
            self.remove(i);
        }
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            wlr_layer(opts.layer),
            Some(opts.namespace.clone()),
            None,
        );
        apply_options(&layer, &opts);
        // The first commit has no buffer; the compositor answers with a size.
        layer.commit();
        let scale = self
            .output_state
            .outputs()
            .filter_map(|o| self.output_state.info(&o))
            .map(|i| i.scale_factor)
            .max()
            .unwrap_or(1);
        self.cx.layers.shown.insert(opts.namespace.clone());
        self.cx.layers.wl_surfaces.insert(opts.namespace.clone(), layer.wl_surface().clone());
        self.surfaces.push(Surface {
            target: None,
            layer,
            opts,
            size: (0, 0),
            scale,
            buffer_scale: 1,
            configured: false,
            dirty: false,
            scene: Scene::default(),
            frame: Frame::default(),
            shown: Vec::new(),
        });
    }

    fn remove(&mut self, i: usize) {
        let s = self.surfaces.remove(i);
        let surface = s.layer.wl_surface().clone();
        if self.pointer_on.as_ref() == Some(&surface) {
            self.pointer_on = None;
            self.cx.hovered.clear();
            self.pressed_click = None;
            self.pressed_input = None;
        }
        if self.keyboard_on.as_ref() == Some(&surface) {
            self.keyboard_on = None;
        }
        self.cx.layers.shown.remove(&s.opts.namespace);
        self.cx.layers.wl_surfaces.remove(&s.opts.namespace);
        drop(s);
    }

    fn draw(&mut self, i: usize) {
        let Some(app) = self.app.as_mut() else { return };
        let s = &mut self.surfaces[i];
        if !s.configured || s.size.0 == 0 || s.size.1 == 0 {
            return;
        }
        let wl = s.layer.wl_surface().clone();
        let display = WlDisplay(NonNull::new(self.conn.backend().display_ptr() as *mut c_void).expect("wl_display"));
        if self.gpu.is_none() {
            let (gpu, target) = Gpu::new(display, &wl);
            self.gpu = Some(gpu);
            s.target = Some((target, None));
            s.shown.clear();
        }
        let gpu = self.gpu.as_mut().unwrap();
        if s.target.is_none() {
            match gpu.create_surface(display, &wl) {
                Some(t) => s.target = Some((t, None)),
                None => return,
            }
            s.shown.clear();
        }
        let phys = (s.size.0 * s.scale as u32, s.size.1 * s.scale as u32);
        let (target, config) = s.target.as_mut().unwrap();
        if config.as_ref().is_none_or(|c| (c.width, c.height) != phys) {
            *config = Some(gpu.configure(target, phys));
            s.shown.clear();
        }
        if s.buffer_scale != s.scale {
            wl.set_buffer_scale(s.scale);
            s.buffer_scale = s.scale;
        }

        self.cx.window_size = (s.size.0 as f32, s.size.1 as f32);
        self.cx.scale = s.scale as f32;
        self.cx.layers.current.clone_from(&s.opts.namespace);
        self.cx.window_focused = self.keyboard_on.as_ref() == Some(&wl);
        for attempt in 0..2 {
            self.cx.ime_area = None;
            s.scene.clear();
            let base = app.base_text_style(&self.cx);
            let root = AnyElement::Div(app.render(&mut self.cx));
            s.frame = render_tree(&root, &mut self.cx, &mut s.scene, base);
            if self.cx.text.atlas.overflowed && attempt == 0 {
                self.cx.text.atlas.reset();
                continue;
            }
            break;
        }
        gpu.renderer.upload_atlas(self.cx.text.atlas.take_uploads());

        let instances: Vec<Instance> = s.scene.instances().copied().collect();
        let same = bytemuck::cast_slice::<Instance, u8>(&instances) == bytemuck::cast_slice::<Instance, u8>(&s.shown);
        if !same {
            let (target, config) = s.target.as_ref().unwrap();
            let frame = match target.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    if let Some(config) = config {
                        target.configure(&gpu.renderer.device, config);
                    }
                    s.dirty = true;
                    None
                }
                // The compositor is holding every buffer (busy); try again next round.
                wgpu::CurrentSurfaceTexture::Timeout => {
                    s.dirty = true;
                    None
                }
                _ => None,
            };
            if let Some(frame) = frame {
                let view = frame.texture.create_view(&Default::default());
                let encoder = gpu.renderer.encode(&s.scene, &view, self.cx.window_size, self.cx.scale, 0.0);
                gpu.renderer.queue.submit([encoder.finish()]);
                gpu.renderer.queue.present(frame);
                s.shown = instances;
            }
        }
        if self.pointer_on.as_ref() == Some(&wl) {
            self.update_hover(i);
        }
    }

    // -- input ----------------------------------------------------------------

    fn update_hover(&mut self, i: usize) {
        let hits = self.surfaces[i].frame.hits(self.cx.mouse);
        let hovered: Vec<ElementId> = hits.iter().filter_map(|h| h.id).collect();
        let cursor = hits.iter().find_map(|h| h.cursor).unwrap_or(CursorIcon::Default);
        if hovered != self.cx.hovered {
            self.cx.hovered = hovered;
            self.surfaces[i].dirty = true;
        }
        if cursor != self.cursor {
            self.cursor = cursor;
            if let Some(p) = &self.pointer {
                let _ = p.set_cursor(&self.conn, cursor);
            }
        }
    }

    fn pointer_event(&mut self, e: &PointerEvent) {
        let Some(i) = self.index(&e.surface) else { return };
        self.enter(i);
        self.cx.mouse = Point { x: e.position.0 as f32, y: e.position.1 as f32 };
        match e.kind {
            PointerEventKind::Enter { .. } => {
                self.pointer_on = Some(e.surface.clone());
                self.cursor = CursorIcon::Default;
                if let Some(p) = &self.pointer {
                    let _ = p.set_cursor(&self.conn, CursorIcon::Default);
                }
                self.update_hover(i);
            }
            PointerEventKind::Leave { .. } => {
                self.pointer_on = None;
                self.cx.mouse = Point { x: -1.0, y: -1.0 };
                self.cx.mouse_down = false;
                self.pressed_click = None;
                self.pressed_input = None;
                if !self.cx.hovered.is_empty() {
                    self.cx.hovered.clear();
                    self.surfaces[i].dirty = true;
                }
            }
            PointerEventKind::Motion { .. } => {
                if let Some((state, b)) = &self.pressed_input {
                    let local = Point { x: self.cx.mouse.x - b.x, y: self.cx.mouse.y - b.y };
                    state.borrow_mut().mouse_drag(&mut self.cx.text.fonts, local);
                    self.surfaces[i].dirty = true;
                }
                if let Some(app) = self.app.as_mut() {
                    app.on_mouse_move(&mut self.cx);
                }
                self.update_hover(i);
            }
            PointerEventKind::Press { button, .. } => {
                if button == BTN_LEFT {
                    self.cx.mouse_down = true;
                }
                self.mouse_down(i, button);
            }
            PointerEventKind::Release { button, .. } => {
                if button == BTN_LEFT {
                    self.cx.mouse_down = false;
                    self.mouse_up();
                }
            }
            PointerEventKind::Axis { vertical, .. } => {
                // Wayland scrolls down with positive values; wheels also report steps.
                let dy = if vertical.discrete != 0 {
                    -(vertical.discrete as f32) * 48.0
                } else {
                    -vertical.absolute as f32
                };
                self.scroll(i, dy);
            }
        }
    }

    fn mouse_down(&mut self, i: usize, button: u32) {
        let p = self.cx.mouse;
        let Some(app) = self.app.as_mut() else { return };
        let frame = &self.surfaces[i].frame;
        self.cx.redraw = true;

        if button == BTN_RIGHT || button == BTN_MIDDLE {
            let pick = |h: &Hitbox<A>| if button == BTN_RIGHT { h.on_right_click.clone() } else { h.on_middle_click.clone() };
            let handler = frame.hits(p).into_iter().find(|h| pick(h).is_some() || blocks(h));
            if let Some(f) = handler.and_then(pick) {
                f(app, &mut self.cx);
            }
            return;
        }
        if button != BTN_LEFT {
            return;
        }
        let (t, lp, n) = self.last_click;
        let clicks = if t.elapsed() < Duration::from_millis(400) && (lp.x - p.x).abs() < 4.0 && (lp.y - p.y).abs() < 4.0 {
            n + 1
        } else {
            1
        };
        self.last_click = (Instant::now(), p, clicks);

        let hits = frame.hits(p);
        if let Some(id) = hits.iter().find(|h| h.focusable).and_then(|h| h.id) {
            self.cx.focused = Some(id);
        }
        let mut calls: Vec<Handler<A>> = Vec::new();
        for h in &hits {
            if let Some((state, b)) = &h.input {
                let local = Point { x: p.x - b.x, y: p.y - b.y };
                state.borrow_mut().mouse_down(&mut self.cx.text.fonts, local, clicks.min(3), self.modifiers.shift);
                self.pressed_input = Some((state.clone(), *b));
                break;
            }
            let mut handled = false;
            if clicks >= 2 && h.on_double_click.is_some() {
                calls.extend(h.on_double_click.clone());
                handled = true;
            } else if let Some(f) = &h.on_mouse_down {
                calls.push(f.clone());
                handled = true;
            }
            if let Some(f) = &h.on_click {
                self.pressed_click = Some((f.clone(), h.bounds));
                handled = true;
            }
            if handled || blocks(h) {
                break;
            }
        }
        for f in calls {
            f(app, &mut self.cx);
        }
    }

    fn mouse_up(&mut self) {
        let p = self.cx.mouse;
        if let (Some((f, b)), Some(app)) = (self.pressed_click.take(), self.app.as_mut()) {
            if b.contains(p) {
                f(app, &mut self.cx);
            }
        }
        self.pressed_input = None;
        self.cx.redraw = true;
    }

    fn scroll(&mut self, i: usize, dy: f32) {
        let p = self.cx.mouse;
        let scrolls = &self.surfaces[i].frame.scrolls;
        let top = scrolls.iter().filter(|s| s.bounds.contains(p)).map(|s| s.layer).max();
        if let Some(s) = scrolls.iter().rev().find(|s| Some(s.layer) == top && s.bounds.contains(p)) {
            let off = self.cx.scroll_offset(s.id) - dy;
            self.cx.set_scroll_offset(s.id, off);
            self.surfaces[i].dirty = true;
        }
    }

    fn key(&mut self, event: KeyEvent) {
        let Some(kp) = keys::key_press(&event, self.modifiers) else { return };
        let Some(i) = self.keyboard_on.as_ref().and_then(|s| self.index(s)) else { return };
        self.enter(i);
        self.handle_key(kp);
    }

    fn handle_key(&mut self, kp: KeyPress) {
        let focused = self.focused_input().map(|(s, _)| s);
        let Some(app) = self.app.as_mut() else { return };
        self.cx.redraw = true;
        if app.on_key(&kp, &mut self.cx) {
            return;
        }
        if let (Some(state), Some(id)) = (focused, self.cx.focused) {
            let out = state.borrow_mut().handle_key(&mut self.cx.text.fonts, &kp, &mut self.cx.clipboard);
            if out.changed {
                app.on_input(id, &mut self.cx);
            }
        }
    }
}

/// A hitbox without handlers just blocks what's under it (panels, menus).
fn blocks<S>(h: &Hitbox<S>) -> bool {
    h.on_mouse_down.is_none()
        && h.on_click.is_none()
        && h.on_right_click.is_none()
        && h.on_middle_click.is_none()
        && h.on_double_click.is_none()
        && !h.window_drag
        && h.cursor.is_none()
        && !h.focusable
        && h.input.is_none()
}

fn wlr_layer(layer: Layer) -> wlr::Layer {
    match layer {
        Layer::Background => wlr::Layer::Background,
        Layer::Bottom => wlr::Layer::Bottom,
        Layer::Top => wlr::Layer::Top,
        Layer::Overlay => wlr::Layer::Overlay,
    }
}

fn apply_options(layer: &LayerSurface, o: &LayerOptions) {
    let mut anchor = wlr::Anchor::empty();
    for (ours, theirs) in [
        (Anchor::TOP, wlr::Anchor::TOP),
        (Anchor::BOTTOM, wlr::Anchor::BOTTOM),
        (Anchor::LEFT, wlr::Anchor::LEFT),
        (Anchor::RIGHT, wlr::Anchor::RIGHT),
    ] {
        if o.anchor.contains(ours) {
            anchor |= theirs;
        }
    }
    layer.set_anchor(anchor);
    layer.set_size(o.size.0, o.size.1);
    layer.set_exclusive_zone(o.exclusive_zone);
    layer.set_margin(o.margin.0, o.margin.1, o.margin.2, o.margin.3);
    layer.set_keyboard_interactivity(match o.keyboard_interactivity {
        KeyboardInteractivity::None => wlr::KeyboardInteractivity::None,
        KeyboardInteractivity::Exclusive => wlr::KeyboardInteractivity::Exclusive,
        KeyboardInteractivity::OnDemand => wlr::KeyboardInteractivity::OnDemand,
    });
}

// -- GPU ------------------------------------------------------------------------

/// The Wayland display, handed to wgpu (which needs it for GL).
#[derive(Clone, Copy, Debug)]
struct WlDisplay(NonNull<c_void>);

// Safety: libwayland's display is thread-safe and outlives the runner.
unsafe impl Send for WlDisplay {}
unsafe impl Sync for WlDisplay {}

impl HasDisplayHandle for WlDisplay {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(self.0));
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

fn surface_target(display: WlDisplay, surface: &WlSurface) -> wgpu::SurfaceTargetUnsafe {
    let ptr = NonNull::new(surface.id().as_ptr() as *mut c_void).expect("wl_surface");
    wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display.0))),
        raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(ptr)),
    }
}

impl Gpu {
    /// Picks an adapter that can present to `surface` and builds the renderer.
    fn new(display: WlDisplay, surface: &WlSurface) -> (Gpu, wgpu::Surface<'static>) {
        // Like the winit runner: Vulkan alone first, so GL isn't loaded too.
        let (instance, target, adapter) = [wgpu::Backends::VULKAN, wgpu::Backends::all()]
            .into_iter()
            .find_map(|backends| {
                let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
                desc.backends = backends;
                let instance = wgpu::Instance::new(desc.with_env());
                // Safety: the surface is destroyed (with its `Surface`) before the wl_surface.
                let target = unsafe { instance.create_surface_unsafe(surface_target(display, surface)) }.ok()?;
                let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: Some(&target),
                    force_fallback_adapter: false,
                    apply_limit_buckets: false,
                }))
                .ok()?;
                Some((instance, target, adapter))
            })
            .expect("no suitable GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("layer"),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .expect("request device");
        let caps = target.get_capabilities(&adapter);
        // Blend in gamma space like the winit runner, so text weight matches.
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let transparent = caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied);
        let renderer = Renderer::build(device, queue, format, None, transparent);
        (Gpu { instance, adapter, renderer }, target)
    }

    fn create_surface(&self, display: WlDisplay, surface: &WlSurface) -> Option<wgpu::Surface<'static>> {
        // Safety: as in `new`.
        unsafe { self.instance.create_surface_unsafe(surface_target(display, surface)) }.ok()
    }

    fn configure(&self, target: &wgpu::Surface<'static>, (w, h): (u32, u32)) -> wgpu::SurfaceConfiguration {
        let caps = target.get_capabilities(&self.adapter);
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            caps.alpha_modes[0]
        };
        let mut config = target.get_default_config(&self.adapter, w.max(1), h.max(1)).expect("surface not supported");
        config.format = self.renderer.format;
        config.alpha_mode = alpha_mode;
        // We only draw on change, so there's nothing to pace; FIFO would block
        // the loop on frame callbacks from surfaces nobody is looking at.
        config.present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::AutoVsync
        };
        config.desired_maximum_frame_latency = 2;
        target.configure(&self.renderer.device, &config);
        config
    }
}

// -- Wayland handlers -----------------------------------------------------------

impl<A: App> CompositorHandler for Runner<A> {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, factor: i32) {
        if let Some(i) = self.index(surface) {
            let s = &mut self.surfaces[i];
            if s.scale != factor {
                s.scale = factor;
                s.dirty = true;
            }
        }
    }

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {}

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &wl_output::WlOutput) {}
}

impl<A: App> OutputHandler for Runner<A> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl<A: App> LayerShellHandler for Runner<A> {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(i) = self.surfaces.iter().position(|s| &s.layer == layer) {
            self.remove(i);
            self.cx.redraw = true;
        }
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, c: LayerSurfaceConfigure, _: u32) {
        let Some(s) = self.surfaces.iter_mut().find(|s| &s.layer == layer) else { return };
        let w = if c.new_size.0 == 0 { s.opts.size.0 } else { c.new_size.0 };
        let h = if c.new_size.1 == 0 { s.opts.size.1 } else { c.new_size.1 };
        s.size = (w, h);
        s.configured = true;
        s.dirty = true;
    }
}

impl<A: App> SeatHandler for Runner<A> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            let repeat = Box::new(|r: &mut Runner<A>, _: &wl_keyboard::WlKeyboard, e: KeyEvent| r.key(e));
            self.keyboard = self.seat_state.get_keyboard_with_repeat(qh, &seat, None, self.loop_handle.clone(), repeat).ok();
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            let surface = self.compositor.create_surface(qh);
            self.pointer =
                self.seat_state.get_pointer_with_theme(qh, &seat, self.shm.wl_shm(), surface, ThemeSpec::default()).ok();
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if cap == Capability::Pointer {
            self.pointer = None;
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl<A: App> KeyboardHandler for Runner<A> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        self.keyboard_on = Some(surface.clone());
        self.cx.redraw = true;
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, surface: &WlSurface, _: u32) {
        if self.keyboard_on.as_ref() == Some(surface) {
            self.keyboard_on = None;
            self.cx.redraw = true;
        }
    }

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key(event);
    }

    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: u32,
    ) {
        self.modifiers = modifiers;
    }
}

impl<A: App> PointerHandler for Runner<A> {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for e in events {
            self.pointer_event(e);
        }
    }
}

impl<A: App> ShmHandler for Runner<A> {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl<A: App> ProvidesRegistryState for Runner<A> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(@<A: App> Runner<A>);
delegate_output!(@<A: App> Runner<A>);
delegate_shm!(@<A: App> Runner<A>);
delegate_seat!(@<A: App> Runner<A>);
delegate_keyboard!(@<A: App> Runner<A>);
delegate_pointer!(@<A: App> Runner<A>);
delegate_layer!(@<A: App> Runner<A>);
delegate_registry!(@<A: App> Runner<A>);
