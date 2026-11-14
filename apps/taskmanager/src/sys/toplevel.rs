//! The compositor's list of open windows, via `zwlr_foreign_toplevel_manager_v1`
//! (the y7ya-os desktop, sway, Hyprland…). Optional: when the protocol is
//! missing, [`Toplevels::get`] returns `None` and callers fall back to guessing.
//!
//! The protocol has no pids, only app ids and titles; `apps` maps those to
//! processes through the installed `.desktop` files.

use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub struct Toplevel {
    pub app_id: String,
    pub title: String,
}

#[derive(Default)]
struct Shared {
    available: bool,
    windows: Vec<Toplevel>,
}

/// A live window list, kept up to date by a background thread.
#[derive(Clone, Default)]
pub struct Toplevels(Arc<Mutex<Shared>>);

impl Toplevels {
    /// Starts watching; `on_change` runs (on the watcher thread) after each update.
    pub fn watch(on_change: impl Fn() + Send + 'static) -> Self {
        let this = Toplevels::default();
        #[cfg(feature = "foreign-toplevel")]
        {
            let shared = this.0.clone();
            std::thread::Builder::new()
                .name("toplevels".into())
                .spawn(move || {
                    let _ = wl::run(shared, on_change);
                })
                .ok();
        }
        #[cfg(not(feature = "foreign-toplevel"))]
        let _ = on_change;
        this
    }

    /// The open windows, or `None` if the compositor doesn't share them.
    pub fn get(&self) -> Option<Vec<Toplevel>> {
        let s = self.0.lock().unwrap();
        s.available.then(|| s.windows.clone())
    }
}

#[cfg(feature = "foreign-toplevel")]
mod wl {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry::WlRegistry;
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
    use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
        Event as HandleEvent, ZwlrForeignToplevelHandleV1,
    };
    use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
        EVT_TOPLEVEL_OPCODE, Event as ManagerEvent, ZwlrForeignToplevelManagerV1,
    };

    use super::{Shared, Toplevel};

    struct State {
        shared: Arc<Mutex<Shared>>,
        /// Per handle: (pending, committed) app id and title.
        windows: HashMap<wayland_client::backend::ObjectId, (Toplevel, Option<Toplevel>)>,
        on_change: Box<dyn Fn() + Send>,
        finished: bool,
    }

    impl State {
        fn publish(&self) {
            let mut s = self.shared.lock().unwrap();
            s.windows = self.windows.values().filter_map(|(_, c)| c.clone()).collect();
            s.windows.sort_by(|a, b| a.app_id.cmp(&b.app_id).then(a.title.cmp(&b.title)));
            drop(s);
            (self.on_change)();
        }
    }

    pub fn run(shared: Arc<Mutex<Shared>>, on_change: impl Fn() + Send + 'static) -> Result<(), Box<dyn std::error::Error>> {
        let conn = Connection::connect_to_env()?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
        let qh = queue.handle();
        let _manager: ZwlrForeignToplevelManagerV1 = globals.bind(&qh, 1..=3, ())?;
        shared.lock().unwrap().available = true;
        let mut state = State { shared, windows: HashMap::new(), on_change: Box::new(on_change), finished: false };
        while !state.finished {
            queue.blocking_dispatch(&mut state)?;
        }
        state.shared.lock().unwrap().available = false;
        Ok(())
    }

    impl Dispatch<WlRegistry, GlobalListContents> for State {
        fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
        fn event(state: &mut Self, _: &ZwlrForeignToplevelManagerV1, event: ManagerEvent, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            match event {
                ManagerEvent::Toplevel { toplevel } => {
                    let empty = Toplevel { app_id: String::new(), title: String::new() };
                    state.windows.insert(toplevel.id(), (empty, None));
                }
                ManagerEvent::Finished => state.finished = true,
                _ => {}
            }
        }

        event_created_child!(State, ZwlrForeignToplevelManagerV1, [
            EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
        ]);
    }

    impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
        fn event(state: &mut Self, handle: &ZwlrForeignToplevelHandleV1, event: HandleEvent, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            let id = handle.id();
            match event {
                HandleEvent::Title { title } => {
                    if let Some(w) = state.windows.get_mut(&id) {
                        w.0.title = title;
                    }
                }
                HandleEvent::AppId { app_id } => {
                    if let Some(w) = state.windows.get_mut(&id) {
                        w.0.app_id = app_id;
                    }
                }
                HandleEvent::Done => {
                    if let Some(w) = state.windows.get_mut(&id) {
                        w.1 = Some(w.0.clone());
                    }
                    state.publish();
                }
                HandleEvent::Closed => {
                    handle.destroy();
                    state.windows.remove(&id);
                    state.publish();
                }
                _ => {}
            }
        }
    }
}
