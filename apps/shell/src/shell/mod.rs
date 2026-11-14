//! The shell app: one `App` drawing three layer surfaces.
//!
//! - `y7-menubar`: always up, top edge, reserves its height.
//! - `y7-panel`: a transparent full-screen overlay below the menu bar while a
//!   menu, Spotlight, Control Center, Notification Center or a dialog is open.
//!   Clicking its empty area closes it, and it holds the keyboard.
//! - `y7-banners`: notification banners at the top right, sized to the stack.

mod control;
mod icons;
mod menubar;
mod notifications;
mod spotlight;
mod theme;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ui::layer::{Anchor, KeyboardInteractivity, Layer, LayerOptions};
use ui::*;

use crate::apps::{self, DesktopEntry};
use crate::files::FileIndex;
use crate::ipc::Command;
use crate::notify::{self, Store};
use crate::search::Section;
use crate::system::{Request, Services, SessionAction, Status, System};
use crate::toplevel::{Toplevels, Window};
use theme::Theme;

pub const BAR: &str = "y7-menubar";
pub const PANEL: &str = "y7-panel";
pub const BANNERS: &str = "y7-banners";
pub const BAR_H: f32 = 24.0;

pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub fn bar_options() -> LayerOptions {
    LayerOptions {
        layer: Layer::Top,
        anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
        size: (0, BAR_H as u32),
        exclusive_zone: BAR_H as i32,
        namespace: BAR.into(),
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    System,
    App,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    None,
    Menu(MenuKind),
    Spotlight,
    ControlCenter,
    NotificationCenter,
    Confirm(SessionAction),
    About,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    Brightness,
    Volume,
}

pub struct Config {
    pub system: Arc<dyn System>,
    /// Serve org.freedesktop.Notifications and listen on the IPC socket.
    pub services: bool,
    /// Where Spotlight searches for files.
    pub files_root: Option<PathBuf>,
}

pub struct Shell {
    services: Services,
    status: Status,
    toplevels: Option<Toplevels>,
    active: Option<Window>,
    apps: Vec<DesktopEntry>,
    files: Arc<Mutex<Option<FileIndex>>>,
    files_root: Option<PathBuf>,
    pub store: Arc<Mutex<Store>>,
    server: Option<notify::Server>,
    inbox: Arc<Mutex<Vec<Command>>>,
    wake: Wake,
    pub panel: Panel,
    query: Rc<RefCell<TextInput>>,
    results: Vec<Section>,
    selected: usize,
    /// The query `results` were found for.
    last_query: String,
    /// Where things were drawn last frame, for positioning menus and sliders.
    menu_x: [Rc<Cell<f32>>; 2],
    sliders: [Rc<Cell<Bounds>>; 2],
    dragging: Option<Slider>,
    /// What we last asked the compositor for, so unchanged layers aren't touched.
    shown_panel: bool,
    shown_banners: Option<(u32, u32)>,
    dnd_file: Option<PathBuf>,
}

impl Shell {
    pub fn new(cfg: Config, cx: &mut Cx) -> Shell {
        let waker = cx.waker();
        let wake: Wake = Arc::new(move || waker.wake());
        let services = Services::start(cfg.system, wake.clone());
        let data_dir = std::env::var_os("Y7_SHELL_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|d| d.join("y7-shell")));
        let dnd_file = data_dir.map(|d| d.join("focus"));
        let mut store = Store::default();
        store.dnd = dnd_file.as_ref().is_some_and(|f| f.exists());
        let store = Arc::new(Mutex::new(store));
        let inbox: Arc<Mutex<Vec<Command>>> = Arc::default();
        let (mut toplevels, mut server) = (None, None);
        if cfg.services {
            toplevels = Some(Toplevels::start(wake.clone()));
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

        let mut query = TextInput::new(&mut cx.text.fonts, TextStyle::default(), true);
        query.placeholder = "Spotlight Search".into();
        let shell = Shell {
            services,
            status: Status::default(),
            toplevels,
            active: None,
            apps: apps::load(&apps::dirs()),
            files: Arc::default(),
            files_root: cfg.files_root,
            store,
            server,
            inbox,
            wake,
            panel: Panel::None,
            query: Rc::new(RefCell::new(query)),
            results: Vec::new(),
            selected: 0,
            last_query: String::new(),
            menu_x: Default::default(),
            sliders: Default::default(),
            dragging: None,
            shown_panel: false,
            shown_banners: None,
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

    // -- panels ---------------------------------------------------------------

    pub fn open(&mut self, panel: Panel, cx: &mut Cx) {
        if self.panel == panel {
            return self.close(cx);
        }
        self.panel = panel;
        self.dragging = None;
        cx.blur();
        if panel == Panel::Spotlight {
            self.query.borrow_mut().set_text(&mut cx.text.fonts, "");
            self.results.clear();
            self.selected = 0;
            self.last_query.clear();
            cx.focus("spotlight-input");
            self.apps = apps::load(&apps::dirs());
            self.index_files();
        }
        if matches!(panel, Panel::ControlCenter) {
            // Things nobody signals about: refresh on open.
            for p in [crate::system::Part::Bluetooth, crate::system::Part::Brightness, crate::system::Part::Wifi] {
                self.services.send(Request::Refresh(p));
            }
        }
        cx.request_redraw();
    }

    pub fn close(&mut self, cx: &mut Cx) {
        self.panel = Panel::None;
        self.dragging = None;
        cx.blur();
        cx.request_redraw();
    }

    /// Shows, hides and sizes the panel and banner surfaces to match state.
    fn sync_layers(&mut self, cx: &mut Cx) {
        let want_panel = self.panel != Panel::None;
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

        let banners = if self.panel == Panel::NotificationCenter { None } else { self.banner_size(cx) };
        let stale = banners != self.shown_banners || (banners.is_some() && !cx.layer_shown(BANNERS));
        if stale {
            match banners {
                Some(size) => cx.show_layer(LayerOptions {
                    layer: Layer::Overlay,
                    anchor: Anchor::TOP | Anchor::RIGHT,
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
            Command::Spotlight => self.open(Panel::Spotlight, cx),
            Command::ControlCenter => self.open(Panel::ControlCenter, cx),
            Command::NotificationCenter => self.open(Panel::NotificationCenter, cx),
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
            BAR => self.menubar(cx, &t),
            PANEL => self.panel_view(cx, &t),
            BANNERS => self.banners(cx, &t),
            _ => div(),
        }
    }

    fn base_text_style(&self, cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Self::theme(cx).label }
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        if key.is_named(NamedKey::Escape) {
            if self.panel == Panel::Spotlight && !self.query.borrow().is_empty() {
                self.query.borrow_mut().set_text(&mut cx.text.fonts, "");
                self.on_input(eid("spotlight-input"), cx);
            } else {
                self.close(cx);
            }
            return true;
        }
        match self.panel {
            Panel::Spotlight => self.spotlight_key(key, cx),
            Panel::Confirm(action) if key.is_named(NamedKey::Enter) => {
                self.session(action, cx);
                true
            }
            Panel::None => false,
            _ => true,
        }
    }

    fn on_input(&mut self, id: ElementId, _cx: &mut Cx) {
        if id == eid("spotlight-input") {
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
        let clock = Instant::now() + Duration::from_secs((60 - secs.rem_euclid(60)) as u64);
        let banners = self.store.lock().unwrap().next_expiry();
        Some(banners.map_or(clock, |b| b.min(clock)))
    }

    fn on_wake(&mut self, cx: &mut Cx) {
        let hovered = self.hovered_banner(cx);
        self.store.lock().unwrap().expire(Instant::now(), hovered);
        self.flush_signals();
        self.sync_layers(cx);
    }

    fn on_external(&mut self, cx: &mut Cx) {
        self.status = self.services.status();
        if let Some(t) = &self.toplevels {
            self.active = t.active();
        }
        let commands = std::mem::take(&mut *self.inbox.lock().unwrap());
        for cmd in commands {
            self.run_command(cmd, cx);
        }
        if self.panel == Panel::Spotlight {
            // The file index may have finished.
            self.update_results();
        }
        self.flush_signals();
        self.sync_layers(cx);
    }
}
