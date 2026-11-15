//! A socket for the shell (and scripts): what desktops there are and what's
//! on them, and commands to switch between them.
//!
//! Lines in, lines out, at `$XDG_RUNTIME_DIR/<wayland socket>.y7-desktop`
//! (or `$Y7_DESKTOP_SOCKET`):
//!
//! - `watch`: replies with the state as one JSON line, then another every
//!   time it changes: `{"active":0,"screen":[w,h],"spaces":[{"windows":
//!   [{"app_id":..,"title":..,"rect":[x,y,w,h],"minimized":false}]}]}`
//!   (windows bottom to top).
//! - `space <n>`: switches to desktop `n` (from 0).
//! - `activate <app_id or title>`: brings up that app's window, wherever it
//!   is; failing an app id, a window whose title contains the text (a
//!   nested KWin has no app id). Replies `ok` or `none`.

use std::ffi::OsStr;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use smithay::desktop::Window;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};
use smithay::utils::{Logical, Point};

use crate::render::app_id_and_title;
use crate::state::{Desk, frame_of};

#[derive(Default)]
pub struct Ipc {
    path: Option<PathBuf>,
    watchers: Vec<UnixStream>,
    /// The state last sent, so unchanged state isn't sent again.
    last: String,
}

impl Drop for Ipc {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn socket_path(wayland_socket: &OsStr) -> PathBuf {
    if let Some(p) = std::env::var_os("Y7_DESKTOP_SOCKET") {
        return p.into();
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let name = Path::new(wayland_socket).file_name().unwrap_or(OsStr::new("wayland-0"));
    dir.join(format!("{}.y7-desktop", name.to_string_lossy()))
}

pub fn init(desk: &mut Desk) {
    let path = socket_path(&desk.socket_name);
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path).and_then(|l| l.set_nonblocking(true).map(|_| l)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("no desktop socket at {}: {e}", path.display());
            return;
        }
    };
    desk.ipc.path = Some(path);
    let source = Generic::new(listener, Interest::READ, Mode::Level);
    let _ = desk.loop_handle.insert_source(source, |_, listener, desk: &mut Desk| {
        while let Ok((stream, _)) = listener.accept() {
            add_client(desk, stream);
        }
        Ok(PostAction::Continue)
    });
}

fn add_client(desk: &mut Desk, stream: UnixStream) {
    let Ok(writer) = stream.try_clone() else { return };
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let mut pending = Vec::new();
    let source = Generic::new(stream, Interest::READ, Mode::Level);
    let _ = desk.loop_handle.insert_source(source, move |_, stream, desk: &mut Desk| {
        let mut buf = [0u8; 1024];
        let mut reader: &UnixStream = stream.as_ref();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return Ok(PostAction::Remove),
                Ok(n) => pending.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => return Ok(PostAction::Remove),
            }
        }
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            command(desk, line.trim(), &writer);
        }
        // Nobody sends commands this long.
        if pending.len() > 4096 {
            return Ok(PostAction::Remove);
        }
        Ok(PostAction::Continue)
    });
}

fn command(desk: &mut Desk, line: &str, writer: &UnixStream) {
    let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
    match cmd {
        "watch" => {
            if let Ok(w) = writer.try_clone() {
                let mut state = state_json(desk);
                state.push('\n');
                if (&w).write_all(state.as_bytes()).is_ok() {
                    let _ = w.set_nonblocking(true);
                    desk.ipc.watchers.push(w);
                    state.pop();
                    desk.ipc.last = state;
                }
            }
        }
        "space" => {
            if let Ok(n) = arg.trim().parse::<usize>() {
                if desk.mc_open() {
                    desk.close_mission_control(None);
                }
                desk.switch_space(n);
                desk.redraw();
            }
        }
        "activate" => {
            let found = desk.activate_app(arg.trim());
            let _ = (&*writer).write_all(if found { b"ok\n" } else { b"none\n" });
        }
        _ => {
            let _ = (&*writer).write_all(b"unknown command\n");
        }
    }
}

impl Desk {
    /// Tells watchers about the desktops if anything changed since last time.
    pub fn publish_spaces(&mut self) {
        if self.ipc.watchers.is_empty() {
            return;
        }
        let state = state_json(self);
        if state == self.ipc.last {
            return;
        }
        let line = format!("{state}\n");
        // A watcher that can't keep up (or went away) is dropped.
        self.ipc.watchers.retain(|w| matches!((&*w).write(line.as_bytes()), Ok(n) if n == line.len()));
        self.ipc.last = state;
    }

    /// Finds a window of the app on any desktop and brings it up.
    pub fn activate_app(&mut self, app_id: &str) -> bool {
        if app_id.is_empty() {
            return false;
        }
        let by_id = |w: &Window| app_id_and_title(w).0.eq_ignore_ascii_case(app_id);
        let any = self.minimized.iter().chain(self.space.elements()).chain(self.spaces.parked.iter().flatten().map(|(w, _)| w));
        let use_id = any.into_iter().any(|w| by_id(w));
        let is_app = |w: &Window| if use_id { by_id(w) } else { app_id_and_title(w).1.contains(app_id) };
        if let Some(w) = self.minimized.iter().find(|w| is_app(w)).cloned() {
            self.restore(w);
            return true;
        }
        let parked = (0..self.spaces.parked.len())
            .find_map(|i| self.spaces.parked[i].iter().find(|(w, _)| is_app(w)).map(|(w, _)| (i, w.clone())));
        if let Some((n, w)) = parked {
            self.switch_space(n);
            self.focus(Some(w));
            return true;
        }
        let shown = self.space.elements().find(|w| is_app(w)).cloned();
        if let Some(w) = shown {
            self.focus(Some(w));
            return true;
        }
        false
    }
}

fn state_json(desk: &Desk) -> String {
    let screen = desk.output_geometry();
    let origin = screen.loc;
    let window = |w: &Window, loc: Point<i32, Logical>, minimized: bool| {
        let (app_id, title) = app_id_and_title(w);
        let r = frame_of(w, loc);
        format!(
            r#"{{"app_id":{},"title":{},"rect":[{},{},{},{}],"minimized":{minimized}}}"#,
            quote(&app_id),
            quote(&title),
            r.loc.x - origin.x,
            r.loc.y - origin.y,
            r.size.w,
            r.size.h
        )
    };
    let spaces: Vec<String> = (0..desk.spaces.parked.len())
        .map(|i| {
            let windows: Vec<String> = if i == desk.spaces.active {
                let shown = desk.space.elements().filter_map(|w| desk.space.element_location(w).map(|l| window(w, l, false)));
                // Minimized windows come back to whichever desktop is showing.
                let hidden = desk
                    .minimized
                    .iter()
                    .map(|w| window(w, crate::state::win_data(w).borrow().parked_at.unwrap_or_default(), true));
                hidden.chain(shown).collect()
            } else {
                desk.spaces.parked[i].iter().map(|(w, l)| window(w, *l, false)).collect()
            };
            format!(r#"{{"windows":[{}]}}"#, windows.join(","))
        })
        .collect();
    format!(
        r#"{{"active":{},"screen":[{},{}],"spaces":[{}]}}"#,
        desk.spaces.active,
        screen.size.w,
        screen.size.h,
        spaces.join(",")
    )
}

/// A JSON string literal.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn quotes_json_strings() {
        assert_eq!(super::quote(r#"a "b" \ c"#), r#""a \"b\" \\ c""#);
        assert_eq!(super::quote("tab\there\n"), r#""tab\u0009here\n""#);
    }
}
