//! wlr-foreign-toplevel-management: tells the shell (menu bar) which windows
//! exist and which one is active, and lets it activate, minimize, zoom or
//! close them. State is diffed once per event-loop round (`refresh`), so the
//! rest of the compositor doesn't need to know about this protocol.

use smithay::desktop::Window;
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

use crate::state::{Desk, win_data};

#[derive(Default)]
pub struct ForeignToplevels {
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    entries: Vec<Entry>,
    next_id: u64,
}

struct Entry {
    id: u64,
    window: Window,
    handles: Vec<ZwlrForeignToplevelHandleV1>,
    info: Info,
}

#[derive(Clone, Default, PartialEq)]
struct Info {
    title: String,
    app_id: String,
    activated: bool,
    minimized: bool,
    maximized: bool,
}

impl ForeignToplevels {
    pub fn init(dh: &DisplayHandle) -> Self {
        dh.create_global::<Desk, ZwlrForeignToplevelManagerV1, ()>(3, ());
        Self::default()
    }
}

fn info(desk: &Desk, window: &Window) -> Info {
    let (title, app_id) = window
        .toplevel()
        .map(|t| {
            with_states(t.wl_surface(), |states| {
                let data = states.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap();
                (data.title.clone().unwrap_or_default(), data.app_id.clone().unwrap_or_default())
            })
        })
        .unwrap_or_default();
    Info {
        title,
        app_id,
        // Activated, not keyboard focus: Spotlight holding the keys keeps the app active.
        activated: window.toplevel().is_some_and(|t| t.with_pending_state(|s| s.states.contains(xdg_toplevel::State::Activated))),
        minimized: desk.minimized.contains(window),
        maximized: win_data(window).borrow().restore.is_some(),
    }
}

fn send(handle: &ZwlrForeignToplevelHandleV1, old: Option<&Info>, new: &Info) {
    if old.is_none_or(|o| o.title != new.title) {
        handle.title(new.title.clone());
    }
    if old.is_none_or(|o| o.app_id != new.app_id) {
        handle.app_id(new.app_id.clone());
    }
    let mut states: Vec<u8> = Vec::new();
    for (on, state) in [
        (new.maximized, zwlr_foreign_toplevel_handle_v1::State::Maximized),
        (new.minimized, zwlr_foreign_toplevel_handle_v1::State::Minimized),
        (new.activated, zwlr_foreign_toplevel_handle_v1::State::Activated),
    ] {
        if on {
            states.extend_from_slice(&(state as u32).to_ne_bytes());
        }
    }
    handle.state(states);
    handle.done();
}

impl Desk {
    /// Sends what changed since the last call to every listening client.
    pub fn refresh_foreign_toplevels(&mut self) {
        if self.foreign_toplevels.managers.is_empty() {
            return;
        }
        let windows: Vec<Window> = self.space.elements().chain(self.minimized.iter()).cloned().collect();
        let mut ft = std::mem::take(&mut self.foreign_toplevels);
        ft.entries.retain(|e| {
            let alive = windows.contains(&e.window);
            if !alive {
                e.handles.iter().for_each(|h| h.closed());
            }
            alive
        });
        for window in &windows {
            let new = info(self, window);
            match ft.entries.iter_mut().find(|e| &e.window == window) {
                Some(e) if e.info != new => {
                    e.handles.iter().for_each(|h| send(h, Some(&e.info), &new));
                    e.info = new;
                }
                Some(_) => {}
                None => {
                    ft.next_id += 1;
                    let mut e = Entry { id: ft.next_id, window: window.clone(), handles: Vec::new(), info: new };
                    for m in ft.managers.clone() {
                        announce(&self.display_handle, &m, &mut e);
                    }
                    ft.entries.push(e);
                }
            }
        }
        self.foreign_toplevels = ft;
    }

    fn foreign_window(&self, handle: &ZwlrForeignToplevelHandleV1) -> Option<Window> {
        let id = *handle.data::<u64>()?;
        self.foreign_toplevels.entries.iter().find(|e| e.id == id).map(|e| e.window.clone())
    }
}

/// Creates a handle for `e` on manager `m`'s client and sends its state.
fn announce(dh: &DisplayHandle, m: &ZwlrForeignToplevelManagerV1, e: &mut Entry) {
    let Some(client) = m.client() else { return };
    let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, u64, Desk>(dh, m.version(), e.id) else {
        return;
    };
    m.toplevel(&handle);
    send(&handle, None, &e.info);
    e.handles.push(handle);
}

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ()> for Desk {
    fn bind(
        desk: &mut Self,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let m = init.init(resource, ());
        let mut ft = std::mem::take(&mut desk.foreign_toplevels);
        for e in &mut ft.entries {
            announce(dh, &m, e);
        }
        ft.managers.push(m);
        desk.foreign_toplevels = ft;
        // Windows that existed before the first client bound are picked up here.
        desk.refresh_foreign_toplevels();
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Desk {
    fn request(
        desk: &mut Self,
        _: &Client,
        m: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            desk.foreign_toplevels.managers.retain(|x| x != m);
            m.finished();
        }
    }

    fn destroyed(desk: &mut Self, _: smithay::reexports::wayland_server::backend::ClientId, m: &ZwlrForeignToplevelManagerV1, _: &()) {
        desk.foreign_toplevels.managers.retain(|x| x != m);
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, u64> for Desk {
    fn request(
        desk: &mut Self,
        _: &Client,
        handle: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        _: &u64,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Request;
        let Some(window) = desk.foreign_window(handle) else { return };
        let maximized = win_data(&window).borrow().restore.is_some();
        match request {
            Request::SetMaximized if !maximized => desk.zoom(&window),
            Request::UnsetMaximized if maximized => desk.zoom(&window),
            Request::SetMinimized if !desk.minimized.contains(&window) => desk.minimize(&window),
            Request::UnsetMinimized | Request::Activate { .. } => {
                if let Some(i) = desk.minimized.iter().position(|w| w == &window) {
                    let w = desk.minimized.remove(i);
                    let loc = win_data(&w).borrow_mut().parked_at.take();
                    desk.place(&w, loc);
                }
                desk.focus(Some(window));
            }
            Request::Close => desk.close(&window),
            _ => {}
        }
    }

    fn destroyed(desk: &mut Self, _: smithay::reexports::wayland_server::backend::ClientId, handle: &ZwlrForeignToplevelHandleV1, _: &u64) {
        for e in &mut desk.foreign_toplevels.entries {
            e.handles.retain(|h| h != handle);
        }
    }
}
