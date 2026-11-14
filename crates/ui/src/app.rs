use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, ResizeDirection, Theme, Window, WindowId};

use super::cx::{CARET_BLINK_MS, Cx, WindowCommand};
use super::element::{AnyElement, Div, ElementId, Frame, Handler, render_tree};
use super::geom::{Bounds, Point};
use super::input::{KeyPress, TextInput};
use super::renderer::Renderer;
use super::scene::Scene;
use super::text::TextStyle;
use super::widget::Widget;

pub trait App: Sized + 'static {
    fn render(&mut self, cx: &mut Cx) -> Div<Self>;
    fn base_text_style(&self, cx: &Cx) -> TextStyle;
    /// Called before the focused input sees the key. Return true if handled.
    fn on_key(&mut self, _key: &KeyPress, _cx: &mut Cx) -> bool {
        false
    }
    /// The text of input `id` changed.
    fn on_input(&mut self, _id: ElementId, _cx: &mut Cx) {}
    /// Next time `on_wake` should run (autosave and the like).
    fn next_wake(&self) -> Option<Instant> {
        None
    }
    fn on_wake(&mut self, _cx: &mut Cx) {}
    /// Another thread called [`Waker::wake`](super::cx::Waker::wake).
    fn on_external(&mut self, _cx: &mut Cx) {}
    /// The mouse moved with the left button held (`cx.mouse` is where).
    /// Call `cx.request_redraw()` if that changed anything.
    fn on_drag(&mut self, _cx: &mut Cx) {}
    /// The pointer moved, button held or not (drags, rubber bands). Call
    /// `cx.request_redraw()` when that changes anything.
    fn on_mouse_move(&mut self, _cx: &mut Cx) {}
    /// The left button went up, wherever the pointer is.
    fn on_mouse_up(&mut self, _cx: &mut Cx) {}
    /// A wheel or touchpad scroll in logical pixels (positive `y` scrolls
    /// toward the top). Return true to skip the default `scroll_y` handling.
    fn on_scroll(&mut self, _delta: Point, _cx: &mut Cx) -> bool {
        false
    }
    fn on_exit(&mut self) {}
    /// The window is about to close. Return false to keep it open (e.g. to
    /// ask about unsaved changes first).
    fn should_close(&mut self, _cx: &mut Cx) -> bool {
        true
    }
}

pub struct WindowOptions {
    pub title: String,
    pub app_id: String,
    pub size: (f32, f32),
    pub min_size: (f32, f32),
    /// Use the desktop's own title bar and window buttons. When false the app
    /// draws its own chrome and the window gets rounded, transparent corners.
    pub decorations: bool,
}

type Make<A> = Box<dyn FnOnce(&mut Cx) -> A>;

struct Runner<A: App> {
    opts: WindowOptions,
    make: Option<Make<A>>,
    app: Option<A>,
    cx: Cx,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    scene: Scene,
    frame: Frame<A>,
    modifiers: ModifiersState,
    pressed_click: Option<(Handler<A>, Bounds)>,
    pressed_input: Option<(Rc<RefCell<TextInput>>, Bounds)>,
    pressed_widget: Option<(Rc<RefCell<dyn Widget>>, Bounds)>,
    last_click: (Instant, Point, u32),
    cursor: CursorIcon,
}

pub fn run<A: App>(opts: WindowOptions, make: impl FnOnce(&mut Cx) -> A + 'static) {
    let event_loop = EventLoop::new().expect("event loop");
    let mut runner = Runner {
        opts,
        make: Some(Box::new(make)),
        app: None,
        cx: Cx::new(),
        window: None,
        renderer: None,
        scene: Scene::default(),
        frame: Frame::default(),
        modifiers: ModifiersState::empty(),
        pressed_click: None,
        pressed_input: None,
        pressed_widget: None,
        last_click: (Instant::now() - Duration::from_secs(10), Point::default(), 0),
        cursor: CursorIcon::Default,
    };
    runner.cx.waker = super::cx::Waker(Some(event_loop.create_proxy()));
    event_loop.run_app(&mut runner).expect("run app");
}

/// Renders one frame offscreen to a PNG, to check the look without a window.
pub fn snapshot<A: App>(
    size: (f32, f32),
    scale: f32,
    dark: bool,
    path: &std::path::Path,
    make: impl FnOnce(&mut Cx) -> A,
    setup: impl FnOnce(&mut A, &mut Cx),
) {
    let mut cx = Cx::new();
    cx.window_size = size;
    cx.scale = scale;
    cx.dark = dark;
    cx.transparent = false;
    let mut renderer = Renderer::new_headless();
    let mut app = make(&mut cx);
    setup(&mut app, &mut cx);
    let mut scene = Scene::default();
    for _ in 0..2 {
        scene.clear();
        let base = app.base_text_style(&cx);
        let root = AnyElement::Div(app.render(&mut cx));
        let frame = render_tree(&root, &mut cx, &mut scene, base);
        cx.hovered = frame.hits(cx.mouse).iter().filter_map(|h| h.id).collect();
    }
    renderer.upload_atlas(cx.text.atlas.take_uploads());
    renderer.snapshot(&scene, size, scale, cx.effective_window_radius(), path);
}

/// winit's theme is often unknown on Linux; fall back to KDE's color scheme.
fn detect_dark(window: &Window) -> bool {
    if let Some(t) = window.theme() {
        return t == Theme::Dark;
    }
    let path = dirs::config_dir().map(|d| d.join("kdeglobals"));
    if let Some(s) = path.and_then(|p| std::fs::read_to_string(p).ok()) {
        if let Some(line) = s.lines().find(|l| l.starts_with("ColorScheme=")) {
            return line.to_lowercase().contains("dark");
        }
    }
    false
}

const RESIZE_MARGIN: f32 = 6.0;

impl<A: App> Runner<A> {
    fn resize_direction(&self, p: Point) -> Option<ResizeDirection> {
        if self.cx.maximized || self.opts.decorations {
            return None;
        }
        let (w, h) = self.cx.window_size;
        let m = RESIZE_MARGIN;
        let (l, r, t, b) = (p.x < m, p.x > w - m, p.y < m, p.y > h - m);
        Some(match (l, r, t, b) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (.., true) => ResizeDirection::South,
            _ => return None,
        })
    }

    fn update_hover(&mut self) {
        let hits = self.frame.hits(self.cx.mouse);
        let hovered: Vec<ElementId> = hits.iter().filter_map(|h| h.id).collect();
        if hovered != self.cx.hovered {
            self.cx.hovered = hovered;
            self.cx.redraw = true;
        }
        let cursor = match self.resize_direction(self.cx.mouse) {
            Some(dir) => match dir {
                ResizeDirection::North | ResizeDirection::South => CursorIcon::NsResize,
                ResizeDirection::East | ResizeDirection::West => CursorIcon::EwResize,
                ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::NwseResize,
                ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::NeswResize,
            },
            None => hits.iter().find_map(|h| h.cursor).unwrap_or(CursorIcon::Default),
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            if let Some(w) = &self.window {
                w.set_cursor(cursor);
            }
        }
    }

    fn after_event(&mut self) {
        let Some(window) = self.window.clone() else { return };
        for cmd in std::mem::take(&mut self.cx.commands) {
            match cmd {
                WindowCommand::Close => {
                    if let Some(app) = self.app.as_mut() {
                        if !app.should_close(&mut self.cx) {
                            continue;
                        }
                        app.on_exit();
                    }
                    std::process::exit(0);
                }
                WindowCommand::Resize(w, h) => {
                    // Some platforms apply the size at once instead of sending `Resized`.
                    if let Some(size) = window.request_inner_size(LogicalSize::new(w, h)) {
                        if let Some(r) = self.renderer.as_mut() {
                            r.resize(size.width, size.height);
                        }
                        self.cx.window_size = (size.width as f32 / self.cx.scale, size.height as f32 / self.cx.scale);
                        self.cx.redraw = true;
                    }
                }
                WindowCommand::Minimize => window.set_minimized(true),
                WindowCommand::ToggleMaximize => window.set_maximized(!window.is_maximized()),
            }
        }
        if self.cx.redraw {
            self.cx.redraw = false;
            window.request_redraw();
        }
    }

    fn focused_input(&self) -> Option<Rc<RefCell<TextInput>>> {
        let focused = self.cx.focused?;
        self.frame
            .hitboxes
            .iter()
            .find(|h| h.id == Some(focused) && h.input.is_some())
            .and_then(|h| h.input.as_ref().map(|(s, _)| s.clone()))
    }

    fn focused_widget(&self) -> Option<Rc<RefCell<dyn Widget>>> {
        let focused = self.cx.focused?;
        self.frame.hitboxes.iter().find(|h| h.id == Some(focused)).and_then(|h| h.widget.as_ref().map(|(w, _)| w.clone()))
    }

    fn mouse_down(&mut self, button: MouseButton) {
        let p = self.cx.mouse;
        let resize = self.resize_direction(p);
        let Some(app) = self.app.as_mut() else { return };
        let window = self.window.clone().unwrap();

        if button == MouseButton::Right {
            let mut handler = None;
            for h in self.frame.hits(p) {
                if h.on_right_click.is_some() || h.occlude_like() {
                    handler = h.on_right_click.clone();
                    break;
                }
            }
            if let Some(f) = handler {
                f(app, &mut self.cx);
            }
            self.cx.redraw = true;
            return;
        }
        if button != MouseButton::Left {
            return;
        }
        if let Some(dir) = resize {
            let _ = window.drag_resize_window(dir);
            return;
        }

        let (t, lp, n) = self.last_click;
        let clicks = if t.elapsed() < Duration::from_millis(400) && (lp.x - p.x).abs() < 4.0 && (lp.y - p.y).abs() < 4.0 {
            n + 1
        } else {
            1
        };
        self.last_click = (Instant::now(), p, clicks);

        let hits = self.frame.hits(p);
        if let Some(id) = hits.iter().find(|h| h.focusable).and_then(|h| h.id) {
            self.cx.focused = Some(id);
        }

        enum Act<A> {
            Call(Handler<A>),
            Press(Handler<A>, Bounds),
            Input(Rc<RefCell<TextInput>>, Bounds),
            Widget(Rc<RefCell<dyn Widget>>, Bounds),
            Drag,
            Maximize,
        }
        let mut acts: Vec<Act<A>> = Vec::new();
        for h in &hits {
            if let Some((state, b)) = &h.input {
                acts.push(Act::Input(state.clone(), *b));
                break;
            }
            if let Some((state, b)) = &h.widget {
                acts.push(Act::Widget(state.clone(), *b));
                break;
            }
            let mut handled = false;
            if clicks >= 2 && h.on_double_click.is_some() {
                acts.push(Act::Call(h.on_double_click.clone().unwrap()));
                handled = true;
            } else if let Some(f) = &h.on_mouse_down {
                acts.push(Act::Call(f.clone()));
                handled = true;
            }
            if let Some(f) = &h.on_click {
                acts.push(Act::Press(f.clone(), h.bounds));
                handled = true;
            }
            if !handled && h.window_drag {
                acts.push(if clicks == 2 { Act::Maximize } else { Act::Drag });
                handled = true;
            }
            if handled || h.occlude_like() {
                break;
            }
        }
        for act in acts {
            match act {
                Act::Call(f) => f(app, &mut self.cx),
                Act::Press(f, b) => self.pressed_click = Some((f, b)),
                Act::Input(state, b) => {
                    let local = Point { x: p.x - b.x, y: p.y - b.y };
                    state.borrow_mut().mouse_down(&mut self.cx.text.fonts, local, clicks.min(3), self.modifiers.shift_key());
                    self.pressed_input = Some((state, b));
                }
                Act::Widget(state, b) => {
                    let local = Point { x: p.x - b.x, y: p.y - b.y };
                    state.borrow_mut().mouse_down(local, clicks.min(3), self.modifiers.shift_key(), &mut self.cx);
                    self.pressed_widget = Some((state, b));
                }
                Act::Drag => {
                    let _ = window.drag_window();
                }
                Act::Maximize => window.set_maximized(!window.is_maximized()),
            }
        }
        self.cx.redraw = true;
    }

    fn mouse_up(&mut self) {
        if let Some(app) = self.app.as_mut() {
            app.on_mouse_up(&mut self.cx);
        }
        let p = self.cx.mouse;
        if let (Some((f, b)), Some(app)) = (self.pressed_click.take(), self.app.as_mut()) {
            if b.contains(p) {
                f(app, &mut self.cx);
            }
        }
        self.pressed_input = None;
        self.pressed_widget = None;
        self.cx.redraw = true;
    }

    fn scroll(&mut self, delta: MouseScrollDelta) {
        let (dx, dy) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (x * 48.0, y * 48.0),
            MouseScrollDelta::PixelDelta(p) => (p.x as f32 / self.cx.scale, p.y as f32 / self.cx.scale),
        };
        if self.app.as_mut().is_some_and(|app| app.on_scroll(Point { x: dx, y: dy }, &mut self.cx)) {
            return;
        }
        let p = self.cx.mouse;
        if let Some((w, _)) = self.frame.hits(p).iter().find_map(|h| h.widget.clone()) {
            if w.borrow_mut().scroll(dy, &mut self.cx) {
                self.cx.redraw = true;
                return;
            }
        }
        let top = self.frame.scrolls.iter().filter(|s| s.bounds.contains(p)).map(|s| s.layer).max();
        let target = self.frame.scrolls.iter().rev().find(|s| Some(s.layer) == top && s.bounds.contains(p));
        if let Some(s) = target {
            let off = self.cx.scroll_offset(s.id) - dy;
            self.cx.set_scroll_offset(s.id, off);
            self.cx.redraw = true;
        }
    }

    fn key(&mut self, kp: KeyPress) {
        let focused = self.focused_input();
        let widget = self.focused_widget();
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
        } else if let (Some(w), Some(id)) = (widget, self.cx.focused) {
            if w.borrow_mut().key(&kp, &mut self.cx) {
                app.on_input(id, &mut self.cx);
            }
        }
    }

    fn draw(&mut self) {
        let (Some(app), Some(renderer)) = (self.app.as_mut(), self.renderer.as_mut()) else { return };
        for attempt in 0..2 {
            self.cx.ime_area = None;
            self.scene.clear();
            let base = app.base_text_style(&self.cx);
            let root = AnyElement::Div(app.render(&mut self.cx));
            self.frame = render_tree(&root, &mut self.cx, &mut self.scene, base);
            if self.cx.text.atlas.overflowed && attempt == 0 {
                self.cx.text.atlas.reset();
                continue;
            }
            break;
        }
        renderer.upload_atlas(self.cx.text.atlas.take_uploads());
        renderer.render(&self.scene, self.cx.window_size, self.cx.scale, self.cx.effective_window_radius());
        self.cx.text.end_frame();
        self.update_hover();
    }
}

impl<S> super::element::Hitbox<S> {
    fn occlude_like(&self) -> bool {
        // Anything that registered a hitbox without a handler is a plain
        // blocker (e.g. `.occlude()` panels and menus).
        self.on_mouse_down.is_none()
            && self.on_click.is_none()
            && self.on_right_click.is_none()
            && self.on_double_click.is_none()
            && !self.window_drag
            && self.cursor.is_none()
            && !self.focusable
            && self.input.is_none()
            && self.widget.is_none()
    }
}

impl<A: App> ApplicationHandler for Runner<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title(self.opts.title.clone())
            .with_inner_size(LogicalSize::new(self.opts.size.0, self.opts.size.1))
            .with_min_inner_size(LogicalSize::new(self.opts.min_size.0, self.opts.min_size.1))
            .with_decorations(self.opts.decorations)
            .with_transparent(!self.opts.decorations);
        #[cfg(target_os = "linux")]
        {
            attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(
                attrs,
                self.opts.app_id.clone(),
                self.opts.app_id.clone(),
            );
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let renderer = Renderer::new(window.clone(), event_loop.owned_display_handle());

        self.cx.scale = window.scale_factor() as f32;
        let size = window.inner_size();
        self.cx.window_size = (size.width as f32 / self.cx.scale, size.height as f32 / self.cx.scale);
        self.cx.dark = detect_dark(&window);
        self.cx.transparent = renderer.transparent && !self.opts.decorations;
        self.cx.maximized = window.is_maximized();

        self.renderer = Some(renderer);
        self.window = Some(window.clone());
        if let Some(make) = self.make.take() {
            self.app = Some(make(&mut self.cx));
        }
        window.request_redraw();
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            if let Some(app) = self.app.as_mut() {
                app.on_wake(&mut self.cx);
            }
            self.cx.redraw = true;
            self.after_event();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(app) = self.app.as_mut() {
            app.on_external(&mut self.cx);
        }
        self.cx.redraw = true;
        self.after_event();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if let Some(app) = self.app.as_mut() {
                    if !app.should_close(&mut self.cx) {
                        self.after_event();
                        return;
                    }
                    app.on_exit();
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
                self.cx.window_size = (size.width as f32 / self.cx.scale, size.height as f32 / self.cx.scale);
                if let Some(w) = &self.window {
                    self.cx.maximized = w.is_maximized();
                }
                self.cx.redraw = true;
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.cx.scale = scale_factor as f32;
                if let Some(w) = &self.window {
                    let size = w.inner_size();
                    self.cx.window_size = (size.width as f32 / self.cx.scale, size.height as f32 / self.cx.scale);
                }
                self.cx.redraw = true;
            }
            WindowEvent::ThemeChanged(theme) => {
                self.cx.dark = theme == Theme::Dark;
                self.cx.redraw = true;
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                self.cx.modifiers = m.state();
            }
            WindowEvent::Focused(focused) => {
                self.cx.window_focused = focused;
                self.cx.redraw = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cx.mouse = Point { x: position.x as f32 / self.cx.scale, y: position.y as f32 / self.cx.scale };
                if let Some((state, b)) = &self.pressed_input {
                    let local = Point { x: self.cx.mouse.x - b.x, y: self.cx.mouse.y - b.y };
                    state.borrow_mut().mouse_drag(&mut self.cx.text.fonts, local);
                    self.cx.redraw = true;
                } else if let (true, Some(app)) = (self.cx.mouse_down, self.app.as_mut()) {
                    app.on_drag(&mut self.cx);
                }
                if let Some((w, b)) = &self.pressed_widget {
                    let local = Point { x: self.cx.mouse.x - b.x, y: self.cx.mouse.y - b.y };
                    w.borrow_mut().mouse_drag(local, &mut self.cx);
                    self.cx.redraw = true;
                }
                self.update_hover();
                if let Some(app) = self.app.as_mut() {
                    app.on_mouse_move(&mut self.cx);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.cx.mouse = Point { x: -1.0, y: -1.0 };
                self.update_hover();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    self.cx.mouse_down = state == ElementState::Pressed;
                    self.cx.redraw = true;
                }
                match state {
                    ElementState::Pressed => self.mouse_down(button),
                    ElementState::Released if button == MouseButton::Left => self.mouse_up(),
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => self.scroll(delta),
            WindowEvent::KeyboardInput { event, is_synthetic: false, .. } if event.state == ElementState::Pressed => {
                let m = self.modifiers;
                self.key(KeyPress {
                    key: event.logical_key.clone(),
                    text: event.text.as_ref().map(|t| t.to_string()),
                    ctrl: m.control_key(),
                    shift: m.shift_key(),
                    alt: m.alt_key(),
                });
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
        self.after_event();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut wake: Option<Instant> = self.app.as_ref().and_then(|a| a.next_wake());
        let since = match self.focused_input() {
            Some(state) => Some(state.borrow().last_activity),
            None => self.focused_widget().and_then(|w| w.borrow().last_activity()),
        };
        if let Some(since) = since {
            let n = (since.elapsed().as_millis() / CARET_BLINK_MS + 1) as u32;
            let next = since + Duration::from_millis(CARET_BLINK_MS as u64) * n;
            wake = Some(wake.map_or(next, |w| w.min(next)));
        }
        event_loop.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}
