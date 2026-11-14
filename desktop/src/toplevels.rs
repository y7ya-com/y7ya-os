//! wlr-foreign-toplevel-management: tells system UI (the menu bar, Task
//! Manager) which windows exist and which is active, and lets it activate,
//! minimize, zoom or close them.

use smithay::desktop::Window;
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::{
    self as handle, ZwlrForeignToplevelHandleV1,
};
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::{
    self as manager, ZwlrForeignToplevelManagerV1,
};
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

use crate::state::{Desk, win_data};

#[derive(Default)]
pub struct ForeignToplevels {
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    entries: Vec<Entry>,
}

struct Entry {
    window: Window,
    handles: Vec<ZwlrForeignToplevelHandleV1>,
    info: Info,
}

#[derive(Clone, PartialEq, Default)]
struct Info {
    title: String,
    app_id: String,
    states: Vec<handle::State>,
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
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .map(|d| {
                        let d = d.lock().unwrap();
                        (d.title.clone().unwrap_or_default(), d.app_id.clone().unwrap_or_default())
                    })
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default();
    let mut states = Vec::new();
    if desk.minimized.contains(window) {
        states.push(handle::State::Minimized);
    }
    if win_data(window).borrow().restore.is_some() {
        states.push(handle::State::Maximized);
    }
    if desk.focused_window().as_ref() == Some(window) {
        states.push(handle::State::Activated);
    }
    Info { title, app_id, states }
}

fn send(h: &ZwlrForeignToplevelHandleV1, info: &Info, old: Option<&Info>) {
    if old.is_none_or(|o| o.title != info.title) {
        h.title(info.title.clone());
    }
    if old.is_none_or(|o| o.app_id != info.app_id) {
        h.app_id(info.app_id.clone());
    }
    if old.is_none_or(|o| o.states != info.states) {
        let bytes: Vec<u8> = info.states.iter().flat_map(|s| (*s as u32).to_ne_bytes()).collect();
        h.state(bytes);
    }
    h.done();
}

impl Desk {
    /// Brings every listener up to date. Cheap when nothing changed.
    pub fn refresh_toplevels(&mut self) {
        let windows: Vec<Window> = self.space.elements().chain(self.minimized.iter()).cloned().collect();
        let mut tl = std::mem::take(&mut self.toplevels);
        tl.entries.retain(|e| {
            let alive = windows.contains(&e.window);
            if !alive {
                for h in &e.handles {
                    h.closed();
                }
            }
            alive
        });
        for window in windows {
            let now = info(self, &window);
            match tl.entries.iter_mut().find(|e| e.window == window) {
                Some(entry) if entry.info != now => {
                    for h in &entry.handles {
                        send(h, &now, Some(&entry.info));
                    }
                    entry.info = now;
                }
                Some(_) => {}
                None => {
                    let handles = tl.managers.iter().filter_map(|m| self.announce(m, &window, &now)).collect();
                    tl.entries.push(Entry { window, handles, info: now });
                }
            }
        }
        self.toplevels = tl;
    }

    fn announce(&self, mgr: &ZwlrForeignToplevelManagerV1, window: &Window, info: &Info) -> Option<ZwlrForeignToplevelHandleV1> {
        let client = mgr.client()?;
        let h = client
            .create_resource::<ZwlrForeignToplevelHandleV1, Window, Desk>(&self.display_handle, mgr.version(), window.clone())
            .ok()?;
        mgr.toplevel(&h);
        send(&h, info, None);
        Some(h)
    }
}

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ()> for Desk {
    fn bind(
        desk: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _data: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let mgr = init.init(resource, ());
        let mut tl = std::mem::take(&mut desk.toplevels);
        for entry in &mut tl.entries {
            if let Some(h) = desk.announce(&mgr, &entry.window, &entry.info) {
                entry.handles.push(h);
            }
        }
        tl.managers.push(mgr);
        desk.toplevels = tl;
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Desk {
    fn request(
        desk: &mut Self,
        _client: &Client,
        mgr: &ZwlrForeignToplevelManagerV1,
        request: manager::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        if let manager::Request::Stop = request {
            desk.toplevels.managers.retain(|m| m != mgr);
            mgr.finished();
        }
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, Window> for Desk {
    fn request(
        desk: &mut Self,
        _client: &Client,
        h: &ZwlrForeignToplevelHandleV1,
        request: handle::Request,
        window: &Window,
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        let maximized = win_data(window).borrow().restore.is_some();
        match request {
            handle::Request::Activate { .. } => {
                if let Some(i) = desk.minimized.iter().position(|w| w == window) {
                    let w = desk.minimized.remove(i);
                    let loc = win_data(&w).borrow_mut().parked_at.take();
                    desk.place(&w, loc);
                }
                desk.focus(Some(window.clone()));
            }
            handle::Request::Close => desk.close(window),
            handle::Request::SetMinimized => {
                if !desk.minimized.contains(window) {
                    desk.minimize(window);
                }
            }
            handle::Request::UnsetMinimized => {
                if let Some(i) = desk.minimized.iter().position(|w| w == window) {
                    let w = desk.minimized.remove(i);
                    let loc = win_data(&w).borrow_mut().parked_at.take();
                    desk.place(&w, loc);
                    desk.focus(Some(w));
                }
            }
            handle::Request::SetMaximized if !maximized => desk.zoom(window),
            handle::Request::UnsetMaximized if maximized => desk.zoom(window),
            handle::Request::Destroy => {
                for entry in &mut desk.toplevels.entries {
                    entry.handles.retain(|x| x != h);
                }
            }
            _ => {}
        }
    }
}
