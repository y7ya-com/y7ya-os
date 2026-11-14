//! Open windows, from wlr-foreign-toplevel-management, for the taskbar.
//!
//! The protocol is bound on the layer surfaces' own Wayland connection so
//! `set_rectangle` can name the taskbar's surface. Its event queue is
//! dispatched on the UI thread after each batch of events the layer runner
//! reads (`Cx::poll_after_events`): a second thread reading the same
//! connection would swallow the UI's own events. Desktops without the
//! protocol (KWin) just never report windows.

use std::cell::RefCell;
use std::rc::Rc;

use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

/// One window, as the taskbar sees it. `id` is ours: stable for the
/// window's life and increasing in the order windows opened.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Window {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    pub maximized: bool,
    pub minimized: bool,
    pub activated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    Activate,
    Close,
    Minimize,
}

/// The UI's handle on the window list (UI thread only).
#[derive(Clone)]
pub struct Toplevels(Rc<RefCell<Inner>>);

struct Inner {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
}

struct Entry {
    handle: ZwlrForeignToplevelHandleV1,
    current: Window,
    pending: Window,
    /// Not shown until the first `done`.
    ready: bool,
}

struct State {
    /// In the order the desktop announced them.
    entries: Vec<(ObjectId, Entry)>,
    next_id: u64,
    seat: Option<wl_seat::WlSeat>,
    changed: bool,
}

impl Toplevels {
    /// Binds the protocol on `conn`; `None` if the desktop doesn't offer it.
    /// Call [`Toplevels::dispatch`] after each batch of events.
    pub fn start(conn: Connection) -> Option<Toplevels> {
        let (globals, queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        if globals.bind::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 1..=3, ()).is_err() {
            eprintln!("y7-shell: the desktop doesn't share its windows (no wlr-foreign-toplevel-management)");
            return None;
        }
        // Activating a window names the seat doing it.
        let seat = globals.bind::<wl_seat::WlSeat, _, _>(&qh, 1..=1, ()).ok();
        let state = State { entries: Vec::new(), next_id: 1, seat, changed: false };
        let _ = conn.flush();
        Some(Toplevels(Rc::new(RefCell::new(Inner { conn, queue, state }))))
    }

    /// Handles the events read so far; true if the window list changed.
    pub fn dispatch(&self) -> bool {
        let mut inner = self.0.borrow_mut();
        let Inner { queue, state, .. } = &mut *inner;
        if let Err(e) = queue.dispatch_pending(state) {
            eprintln!("y7-shell: window list: {e}");
        }
        std::mem::take(&mut state.changed)
    }

    pub fn windows(&self) -> Vec<Window> {
        self.0.borrow().state.entries.iter().filter(|(_, e)| e.ready).map(|(_, e)| e.current.clone()).collect()
    }

    fn handle(&self, id: u64) -> Option<ZwlrForeignToplevelHandleV1> {
        self.0.borrow().state.entries.iter().find(|(_, e)| e.current.id == id).map(|(_, e)| e.handle.clone())
    }

    pub fn act(&self, id: u64, action: WindowAction) {
        let Some(handle) = self.handle(id) else { return };
        match action {
            WindowAction::Activate => match self.0.borrow().state.seat.clone() {
                Some(seat) => handle.activate(&seat),
                None => return,
            },
            WindowAction::Close => handle.close(),
            WindowAction::Minimize => handle.set_minimized(),
        }
        self.flush();
    }

    /// Tells the desktop where each window's taskbar button is on `surface`,
    /// so minimizing can animate into it.
    pub fn set_rectangles(&self, surface: &WlSurface, rects: &[(u64, [i32; 4])]) {
        for (id, [x, y, w, h]) in rects {
            if let Some(handle) = self.handle(*id) {
                handle.set_rectangle(surface, *x, *y, *w, *h);
            }
        }
        self.flush();
    }

    fn flush(&self) {
        let _ = self.0.borrow().conn.flush();
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
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
            let id = state.next_id;
            state.next_id += 1;
            let window = Window { id, ..Default::default() };
            let entry = Entry { handle: toplevel.clone(), current: window.clone(), pending: window, ready: false };
            state.entries.push((toplevel.id(), entry));
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
            state.entries.retain(|(oid, _)| *oid != id);
            handle.destroy();
            state.changed = true;
            return;
        }
        // Changes are double-buffered until `done`.
        let Some((_, entry)) = state.entries.iter_mut().find(|(oid, _)| *oid == id) else { return };
        let pending = &mut entry.pending;
        match event {
            Event::Title { title } => pending.title = title,
            Event::AppId { app_id } => pending.app_id = app_id,
            Event::State { state: raw } => {
                let states: Vec<u32> =
                    raw.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
                pending.activated = states.contains(&(S::Activated as u32));
                pending.maximized = states.contains(&(S::Maximized as u32));
                pending.minimized = states.contains(&(S::Minimized as u32));
            }
            Event::Done => {
                if !entry.ready || entry.current != entry.pending {
                    entry.current = entry.pending.clone();
                    entry.ready = true;
                    state.changed = true;
                }
            }
            _ => {}
        }
    }
}
