//! The shell app: one `App` drawing three layer surfaces.
//!
//! - `y7-taskbar`: always up, bottom edge, reserves its height.
//! - `y7-panel`: a transparent overlay above the taskbar while a flyout
//!   (Start, Quick Settings, the calendar, a button's menu), Spotlight or a
//!   dialog is up. Clicking its empty area closes it, and it holds the keyboard.
//! - `y7-banners`: notification banners at the bottom right, sized to the stack.
//!
//! Flyouts slide and fade in and out; the animation runs on `next_wake`
//! ticks only while it's moving.

mod bar;
mod calendar;
mod control;
mod dialogs;
mod icons;
mod notifications;
mod spotlight;
mod start;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ui::layer::{Anchor, KeyboardInteractivity, Layer, LayerOptions};
use ui::*;

use crate::app_icons::IconCache;
use crate::apps::{self, DesktopEntry};
use crate::files::FileIndex;
use crate::ipc::Command;
use crate::notify::{self, Store};
use crate::search::Section;
use crate::system::{Request, Services, SessionAction, Status, System};
use crate::taskbar::{self, DesktopAction, Item, Pins, ShowDesktop};
use crate::toplevel::{Toplevels, Window, WindowAction};
use theme::Theme;

pub const BAR: &str = "y7-taskbar";
pub const PANEL: &str = "y7-panel";
pub const BANNERS: &str = "y7-banners";
pub const BAR_H: f32 = 44.0;

/// Flyouts float this far from the taskbar and screen edges.
const MARGIN: f32 = 8.0;
/// How far flyouts slide while fading.
const SLIDE: f32 = 8.0;
const OPEN: Duration = Duration::from_millis(180);
const CLOSE: Duration = Duration::from_millis(140);
const FRAME: Duration = Duration::from_millis(16);

pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub fn bar_options() -> LayerOptions {
    LayerOptions {
        layer: Layer::Top,
        anchor: Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        size: (0, BAR_H as u32),
        exclusive_zone: BAR_H as i32,
        namespace: BAR.into(),
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    None,
    Start,
    Spotlight,
    QuickSettings,
    Calendar,
    /// A taskbar button's right-click menu (see `Shell::task_menu`).
    TaskMenu,
    Confirm(SessionAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    Brightness,
    Volume,
}

pub struct Config {
    pub system: Arc<dyn System>,
    /// Serve org.freedesktop.Notifications, listen on the IPC socket and
    /// follow the desktop's windows.
    pub services: bool,
    /// Where Spotlight searches for files.
    pub files_root: Option<PathBuf>,
    /// Where pins are kept (`None` keeps them in memory).
    pub data_dir: Option<PathBuf>,
}

/// The signed-in user, for the Start menu.
pub struct User {
    pub name: String,
    pub avatar: Option<Image>,
}

pub struct Shell {
    services: Services,
    status: Status,
    toplevels: Option<Toplevels>,
    /// Open windows, in the order they opened.
    pub windows: Vec<Window>,
    /// The window that was active before a flyout took the keyboard.
    last_active: Option<u64>,
    apps: Vec<DesktopEntry>,
    icons: IconCache,
    pins: Pins,
    start_pins: Pins,
    show_desktop: ShowDesktop,
    files: Arc<Mutex<Option<FileIndex>>>,
    files_root: Option<PathBuf>,
    pub store: Arc<Mutex<Store>>,
    server: Option<notify::Server>,
    inbox: Arc<Mutex<Vec<Command>>>,
    wake: Wake,
    /// What's open, as far as input is concerned.
    pub panel: Panel,
    /// What's drawn: `panel`, or the one still fading out.
    drawn: Panel,
    /// When the current open or close animation started, and which it is.
    anim: Option<(Instant, bool)>,
    query: Rc<RefCell<TextInput>>,
    results: Vec<Section>,
    selected: usize,
    /// The query `results` were found for.
    last_query: String,
    sliders: [Rc<Cell<Bounds>>; 2],
    dragging: Option<Slider>,
    /// The taskbar button whose menu is open, and the button's x.
    task_menu: Option<(Item, f32)>,
    /// An app's right-click menu in Start, and where it was opened.
    start_menu: Option<(String, Point)>,
    /// Months away from this one in the calendar flyout.
    month_offset: i32,
    user: User,
    /// What we last asked the compositor for, so unchanged things aren't resent.
    shown_panel: bool,
    shown_banners: Option<(u32, u32)>,
    sent_rects: HashMap<u64, [i32; 4]>,
    dnd_file: Option<PathBuf>,
}

impl Shell {
    pub fn new(cfg: Config, cx: &mut Cx) -> Shell {
        let waker = cx.waker();
        let wake: Wake = Arc::new(move || waker.wake());
        let services = Services::start(cfg.system, wake.clone());
        let data_dir = cfg.data_dir;
        let dnd_file = data_dir.as_ref().map(|d| d.join("focus"));
        let mut store = Store::default();
        store.dnd = dnd_file.as_ref().is_some_and(|f| f.exists());
        let store = Arc::new(Mutex::new(store));
        let inbox: Arc<Mutex<Vec<Command>>> = Arc::default();
        let (mut toplevels, mut server) = (None, None);
        if cfg.services {
            toplevels = cx.wayland_connection().and_then(Toplevels::start);
            if let Some(t) = toplevels.clone() {
                cx.poll_after_events(move || t.dispatch());
            }
            match notify::Server::start(store.clone(), wake.clone()) {
                Ok(s) => server = Some(s),
                Err(e) => eprintln!("y7-shell: not showing notifications ({e}); is another notification daemon running?"),
            }
            let (inbox, wake) = (inbox.clone(), wake.clone());
            if let Err(e) = crate::ipc::serve(&crate::ipc::socket_path(), move |cmd| {
                inbox.lock().unwrap().push(cmd);
                wake();
            }) {
                eprintln!("y7-shell: no IPC socket: {e}");
            }
        }

        let query = TextInput::new(&mut cx.text.fonts, TextStyle::default(), true);
        let shell = Shell {
            services,
            status: Status::default(),
            toplevels,
            windows: Vec::new(),
            last_active: None,
            apps: apps::load(&apps::dirs()),
            icons: IconCache::new(wake.clone()),
            pins: Pins::load(data_dir.as_ref().map(|d| d.join("taskbar-pins")), &Pins::TASKBAR),
            start_pins: Pins::load(data_dir.as_ref().map(|d| d.join("start-pins")), &Pins::START),
            show_desktop: ShowDesktop::default(),
            files: Arc::default(),
            files_root: cfg.files_root,
            store,
            server,
            inbox,
            wake,
            panel: Panel::None,
            drawn: Panel::None,
            anim: None,
            query: Rc::new(RefCell::new(query)),
            results: Vec::new(),
            selected: 0,
            last_query: String::new(),
            sliders: Default::default(),
            dragging: None,
            task_menu: None,
            start_menu: None,
            month_offset: 0,
            user: dialogs::current_user(),
            shown_panel: false,
            shown_banners: None,
            sent_rects: HashMap::new(),
            dnd_file,
        };
        shell.index_files();
        shell
    }

    fn theme(cx: &Cx) -> Theme {
        Theme::new(cx.dark)
    }

    /// Picks up the worker's latest status (for snapshots).
    pub fn wait_for_status(&mut self) {
        for _ in 0..100 {
            self.status = self.services.status();
            if self.status != Status::default() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Builds the file index in the background, or refreshes it.
    fn index_files(&self) {
        let Some(root) = self.files_root.clone() else { return };
        let (files, wake) = (self.files.clone(), self.wake.clone());
        std::thread::Builder::new()
            .name("file-index".into())
            .spawn(move || {
                let fresh = files.lock().unwrap().is_none();
                if fresh {
                    let index = FileIndex::build(&root);
                    *files.lock().unwrap() = Some(index);
                } else {
                    // Take it out while refreshing so searches don't wait on disk.
                    let Some(mut index) = files.lock().unwrap().take() else { return };
                    index.refresh();
                    *files.lock().unwrap() = Some(index);
                }
                wake();
            })
            .expect("file index thread");
    }

    // -- apps and windows -------------------------------------------------------

    fn app(&self, id: &str) -> Option<&DesktopEntry> {
        apps::by_id(&self.apps, id)
    }

    /// The desktop-file ID behind a window.
    fn app_of(&self, w: &Window) -> Option<String> {
        taskbar::app_for(&self.apps, &w.app_id)
    }

    /// The taskbar's buttons, in order.
    fn items(&self) -> Vec<Item> {
        let pins: Vec<String> = self.pins.ids.iter().filter(|id| self.app(id).is_some()).cloned().collect();
        taskbar::items(&pins, &self.windows, |w| self.app_of(w))
    }

    fn window(&self, id: u64) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// The active window; while a flyout holds the keyboard, the one that
    /// was active before it opened.
    fn active_window(&self) -> Option<u64> {
        match self.windows.iter().find(|w| w.activated) {
            Some(w) => Some(w.id),
            None if self.drawn != Panel::None => self.last_active.filter(|id| self.window(*id).is_some_and(|w| !w.minimized)),
            None => None,
        }
    }

    /// An app's icon: its entry's `Icon=`, else an icon named like the window.
    fn icon_for(&self, app: Option<&str>, app_id: &str) -> Option<Image> {
        let name = app.and_then(|a| self.app(a)).map(|e| e.icon.as_str()).filter(|i| !i.is_empty()).unwrap_or(app_id);
        self.icons.get(name)
    }

    fn act(&self, id: u64, action: WindowAction) {
        if let Some(t) = &self.toplevels {
            t.act(id, action);
        }
    }

    /// Opens a new instance of an app.
    fn launch_app(&mut self, id: &str, cx: &mut Cx) {
        if let Some(e) = self.app(id) {
            launch(&e.command());
        }
        self.close(cx);
    }

    /// A window button was clicked: like Windows, the active window
    /// minimizes, any other comes forward.
    fn window_clicked(&mut self, id: u64, cx: &mut Cx) {
        let Some(w) = self.window(id).cloned() else { return };
        self.close(cx);
        if self.active_window() == Some(id) && !w.minimized {
            self.act(id, WindowAction::Minimize);
        } else {
            self.act(id, WindowAction::Activate);
        }
    }

    fn toggle_desktop(&mut self, cx: &mut Cx) {
        self.close(cx);
        match self.show_desktop.toggle(&self.windows) {
            DesktopAction::Minimize(ids) => ids.into_iter().for_each(|id| self.act(id, WindowAction::Minimize)),
            DesktopAction::Restore(ids) => ids.into_iter().for_each(|id| self.act(id, WindowAction::Activate)),
        }
    }

    /// Tells the desktop where window buttons are, for the minimize animation.
    fn send_rects(&mut self, rects: Vec<(u64, [i32; 4])>, cx: &Cx) {
        let (Some(t), Some(surface)) = (&self.toplevels, cx.layer_wl_surface(BAR)) else { return };
        let changed: Vec<(u64, [i32; 4])> = rects.into_iter().filter(|(id, r)| self.sent_rects.get(id) != Some(r)).collect();
        if changed.is_empty() {
            return;
        }
        t.set_rectangles(&surface, &changed);
        self.sent_rects.extend(changed);
        self.sent_rects.retain(|id, _| self.windows.iter().any(|w| w.id == *id));
    }

    // -- panels -----------------------------------------------------------------

    pub fn open(&mut self, panel: Panel, cx: &mut Cx) {
        if self.panel == panel {
            return self.close(cx);
        }
        self.panel = panel;
        self.drawn = panel;
        self.anim = Some((Instant::now(), false));
        self.dragging = None;
        self.start_menu = None;
        cx.blur();
        if matches!(panel, Panel::Spotlight | Panel::Start) {
            self.query.borrow_mut().set_text(&mut cx.text.fonts, "");
            self.results.clear();
            self.selected = 0;
            self.last_query.clear();
            cx.focus(if panel == Panel::Start { "start-input" } else { "spotlight-input" });
            self.apps = apps::load(&apps::dirs());
            self.index_files();
        }
        if panel == Panel::Start {
            cx.scroll_to_top("start-all");
        }
        if panel == Panel::Calendar {
            self.month_offset = 0;
        }
        if panel == Panel::QuickSettings {
            // Things nobody signals about: refresh on open.
            for p in [crate::system::Part::Bluetooth, crate::system::Part::Brightness, crate::system::Part::Wifi] {
                self.services.send(Request::Refresh(p));
            }
        }
        cx.request_redraw();
    }

    /// Starts fading out whatever is open.
    pub fn close(&mut self, cx: &mut Cx) {
        if self.panel == Panel::None {
            return;
        }
        self.panel = Panel::None;
        self.anim = Some((Instant::now(), true));
        self.dragging = None;
        self.start_menu = None;
        cx.blur();
        cx.request_redraw();
    }

    /// Opens a panel with its animation already finished (for snapshots).
    pub fn open_now(&mut self, panel: Panel, cx: &mut Cx) {
        self.open(panel, cx);
        self.anim = None;
    }

    /// How far the drawn panel is in: 0 hidden, 1 fully shown (ease-out).
    fn reveal(&self) -> f32 {
        let Some((start, closing)) = self.anim else { return if self.drawn == Panel::None { 0.0 } else { 1.0 } };
        let length = if closing { CLOSE } else { OPEN };
        let t = (start.elapsed().as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        if closing { 1.0 - eased } else { eased }
    }

    /// Ends a finished animation; a closed panel stops being drawn.
    fn settle(&mut self) {
        let Some((start, closing)) = self.anim else { return };
        if start.elapsed() >= if closing { CLOSE } else { OPEN } {
            self.anim = None;
            if closing {
                self.drawn = Panel::None;
                self.task_menu = None;
            }
        }
    }

    /// Shows, hides and sizes the panel and banner surfaces to match state.
    fn sync_layers(&mut self, cx: &mut Cx) {
        let want_panel = self.drawn != Panel::None;
        if want_panel && (!self.shown_panel || !cx.layer_shown(PANEL)) {
            cx.show_layer(LayerOptions {
                layer: Layer::Overlay,
                anchor: Anchor::ALL,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                namespace: PANEL.into(),
                ..Default::default()
            });
        } else if !want_panel && cx.layer_shown(PANEL) {
            cx.hide_layer(PANEL);
        }
        self.shown_panel = want_panel;

        let banners = if self.drawn == Panel::Calendar { None } else { self.banner_size(cx) };
        let stale = banners != self.shown_banners || (banners.is_some() && !cx.layer_shown(BANNERS));
        if stale {
            match banners {
                Some(size) => cx.show_layer(LayerOptions {
                    layer: Layer::Overlay,
                    anchor: Anchor::BOTTOM | Anchor::RIGHT,
                    size,
                    namespace: BANNERS.into(),
                    ..Default::default()
                }),
                None => cx.hide_layer(BANNERS),
            }
            self.shown_banners = banners;
        }
    }

    pub fn run_command(&mut self, cmd: Command, cx: &mut Cx) {
        match cmd {
            Command::Start => self.open(Panel::Start, cx),
            Command::Spotlight => self.open(Panel::Spotlight, cx),
            Command::QuickSettings => self.open(Panel::QuickSettings, cx),
            Command::Calendar => self.open(Panel::Calendar, cx),
            Command::ShowDesktop => self.toggle_desktop(cx),
            Command::Lock => self.services.send(Request::Session(SessionAction::Lock)),
            Command::Dismiss => self.close(cx),
            Command::Dnd(on) => {
                let on = on.unwrap_or(!self.store.lock().unwrap().dnd);
                self.set_dnd(on);
            }
            Command::Ping => {}
        }
    }

    fn set_dnd(&mut self, on: bool) {
        let mut store = self.store.lock().unwrap();
        store.dnd = on;
        if on {
            // Like Focus: what's showing goes quietly to Notification Center.
            let ids: Vec<u32> = store.banners.iter().map(|b| b.id).collect();
            ids.into_iter().for_each(|id| store.dismiss_banner(id));
        }
        drop(store);
        if let Some(f) = &self.dnd_file {
            let _ = if on {
                f.parent().map(std::fs::create_dir_all);
                std::fs::write(f, "")
            } else {
                std::fs::remove_file(f).or(Ok(()))
            };
        }
    }

    fn flush_signals(&self) {
        if let Some(server) = &self.server {
            server.flush(&mut self.store.lock().unwrap());
        } else {
            self.store.lock().unwrap().outbox.clear();
        }
    }

    pub fn session(&mut self, action: SessionAction, cx: &mut Cx) {
        self.close(cx);
        self.services.send(Request::Session(action));
    }

    /// Hands a request to the system worker.
    fn send(&self, r: Request) {
        self.services.send(r);
    }

    /// Loads every icon the shell might show now (for snapshots).
    pub fn preload_icons(&self) {
        let mut names: Vec<String> = self.apps.iter().map(|e| e.icon.clone()).collect();
        names.extend(self.windows.iter().map(|w| w.app_id.clone()));
        for name in names {
            self.icons.load_now(&name);
        }
    }
}

/// Starts a program detached from the shell.
pub fn launch(command: &str) {
    let r = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{command} >/dev/null 2>&1 &"))
        .stdin(std::process::Stdio::null())
        .status();
    if let Err(e) = r {
        eprintln!("y7-shell: couldn't start {command}: {e}");
    }
}

/// Single-quotes a path for `sh`.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl App for Shell {
    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        self.sync_layers(cx);
        let t = Self::theme(cx);
        match cx.layer() {
            BAR => self.taskbar(cx, &t),
            PANEL => self.panel_view(cx, &t),
            BANNERS => self.banners(cx, &t),
            _ => div(),
        }
    }

    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Self::theme(cx).label }
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if self.panel == Panel::None {
            return false;
        }
        if key.is_named(NamedKey::Escape) {
            let searching = matches!(self.panel, Panel::Spotlight | Panel::Start) && !self.query.borrow().is_empty();
            if self.start_menu.take().is_some() {
            } else if searching {
                self.query.borrow_mut().set_text(&mut cx.text.fonts, "");
                self.update_results();
            } else {
                self.close(cx);
            }
            return true;
        }
        match self.panel {
            Panel::Spotlight | Panel::Start => self.search_key(key, cx),
            Panel::Confirm(action) if key.is_named(NamedKey::Enter) => {
                self.session(action, cx);
                true
            }
            _ => true,
        }
    }

    fn on_input(&mut self, id: ElementId, _cx: &mut Cx) {
        if id == eid("spotlight-input") || id == eid("start-input") {
            self.update_results();
        }
    }

    fn on_mouse_move(&mut self, cx: &mut Cx) {
        if self.dragging.is_some() {
            self.drag(cx);
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        // The clock: once a minute, on the minute.
        let secs = chrono::Local::now().timestamp();
        let mut next = Instant::now() + Duration::from_secs((60 - secs.rem_euclid(60)) as u64);
        if let Some(b) = self.store.lock().unwrap().next_expiry() {
            next = next.min(b);
        }
        if self.anim.is_some() {
            next = next.min(Instant::now() + FRAME);
        }
        Some(next)
    }

    fn on_wake(&mut self, cx: &mut Cx) {
        self.settle();
        let hovered = self.hovered_banner(cx);
        self.store.lock().unwrap().expire(Instant::now(), hovered);
        self.flush_signals();
        self.sync_layers(cx);
    }

    fn on_external(&mut self, cx: &mut Cx) {
        self.status = self.services.status();
        if let Some(t) = &self.toplevels {
            self.windows = t.windows();
            if let Some(w) = self.windows.iter().find(|w| w.activated) {
                self.last_active = Some(w.id);
            }
        }
        let commands = std::mem::take(&mut *self.inbox.lock().unwrap());
        for cmd in commands {
            self.run_command(cmd, cx);
        }
        if matches!(self.panel, Panel::Spotlight | Panel::Start) {
            // The file index may have finished.
            self.update_results();
        }
        self.flush_signals();
        self.sync_layers(cx);
    }
}
