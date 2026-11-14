//! The active window, from wlr-foreign-toplevel-management on its own Wayland
//! connection and thread. Desktops without the protocol (KWin) just never
//! report one, and the menu bar shows no app.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Window {
    pub app_id: String,
    pub title: String,
    pub maximized: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    Close,
    Minimize,
    ToggleMaximize,
}

#[derive(Default)]
struct Shared {
    active: Option<(Window, ZwlrForeignToplevelHandleV1)>,
}

/// The UI's view of the active window.
#[derive(Clone)]
pub struct Toplevels {
    shared: Arc<Mutex<Shared>>,
    conn: Option<Connection>,
}

#[derive(Default, Clone)]
struct Info {
    window: Window,
    activated: bool,
}

struct State {
    toplevels: HashMap<ObjectId, (ZwlrForeignToplevelHandleV1, Info, Info)>,
    shared: Arc<Mutex<Shared>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Toplevels {
    /// Starts listening; `wake` runs when the active window changes.
    pub fn start(wake: Arc<dyn Fn() + Send + Sync>) -> Toplevels {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let none = Toplevels { shared: shared.clone(), conn: None };
        let Ok(conn) = Connection::connect_to_env() else { return none };
        let Ok((globals, mut queue)) = registry_queue_init::<State>(&conn) else { return none };
        let qh = queue.handle();
        if globals.bind::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 1..=3, ()).is_err() {
            eprintln!("y7-shell: the desktop doesn't share its windows (no wlr-foreign-toplevel-management)");
            return none;
        }
        let mut state = State { toplevels: HashMap::new(), shared: shared.clone(), wake };
        std::thread::Builder::new()
            .name("toplevels".into())
            .spawn(move || while queue.blocking_dispatch(&mut state).is_ok() {})
            .expect("toplevel thread");
        Toplevels { shared, conn: Some(conn) }
    }

    pub fn active(&self) -> Option<Window> {
        self.shared.lock().unwrap().active.as_ref().map(|(w, _)| w.clone())
    }

    pub fn act(&self, action: WindowAction) {
        let shared = self.shared.lock().unwrap();
        let (Some((w, handle)), Some(conn)) = (&shared.active, &self.conn) else { return };
        match action {
            WindowAction::Close => handle.close(),
            WindowAction::Minimize => handle.set_minimized(),
            WindowAction::ToggleMaximize if w.maximized => handle.unset_maximized(),
            WindowAction::ToggleMaximize => handle.set_maximized(),
        }
        let _ = conn.flush();
    }
}

impl State {
    fn publish(&mut self) {
        let active = self
            .toplevels
            .values()
            .find(|(_, info, _)| info.activated)
            .map(|(h, info, _)| (info.window.clone(), h.clone()));
        let mut shared = self.shared.lock().unwrap();
        let before = shared.active.as_ref().map(|(w, _)| w.clone());
        if before != active.as_ref().map(|(w, _)| w.clone()) {
            shared.active = active;
            drop(shared);
            (self.wake)();
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.insert(toplevel.id(), (toplevel, Info::default(), Info::default()));
        }
    }

    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::{Event, State as S};
        let id = handle.id();
        if let Event::Closed = event {
            state.toplevels.remove(&id);
            handle.destroy();
            state.publish();
            return;
        }
        // Changes are double-buffered until `done`.
        let Some((_, current, pending)) = state.toplevels.get_mut(&id) else { return };
        match event {
            Event::Title { title } => pending.window.title = title,
            Event::AppId { app_id } => pending.window.app_id = app_id,
            Event::State { state: raw } => {
                let states: Vec<u32> =
                    raw.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
                pending.activated = states.contains(&(S::Activated as u32));
                pending.window.maximized = states.contains(&(S::Maximized as u32));
            }
            Event::Done => {
                *current = pending.clone();
                state.publish();
            }
            _ => {}
        }
    }
}
