//! Live reload: an inotify watch on the settings directory. A background
//! thread sleeps in `poll` until something changes, so idle costs nothing.

use std::ffi::{CString, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::thread::JoinHandle;

use super::Settings;

/// Stops watching when dropped.
pub struct Watcher {
    stop_fd: libc::c_int,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            libc::write(self.stop_fd, [1u8].as_ptr().cast(), 1);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        unsafe {
            libc::close(self.stop_fd);
        }
    }
}

/// Calls `on_change` with the new settings whenever the file at `path` is
/// written, replaced or removed (removal gives the defaults). Bursts of
/// events are coalesced, and a file that doesn't parse is skipped.
pub fn watch(path: &Path, mut on_change: impl FnMut(&Settings) + Send + 'static) -> io::Result<Watcher> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| io::Error::other("settings path has no file name"))?.to_owned();
    std::fs::create_dir_all(dir)?;

    let inotify = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
    if inotify < 0 {
        return Err(io::Error::last_os_error());
    }
    let c_dir = CString::new(dir.as_os_str().as_bytes()).map_err(io::Error::other)?;
    let mask = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_DELETE | libc::IN_MOVED_FROM;
    if unsafe { libc::inotify_add_watch(inotify, c_dir.as_ptr(), mask) } < 0 {
        let e = io::Error::last_os_error();
        unsafe { libc::close(inotify) };
        return Err(e);
    }
    let mut pipe = [0 as libc::c_int; 2];
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        let e = io::Error::last_os_error();
        unsafe { libc::close(inotify) };
        return Err(e);
    }
    let (stop_read, stop_write) = (pipe[0], pipe[1]);
    let path = path.to_owned();

    let thread = std::thread::Builder::new().name("settings-watch".into()).spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            let mut fds = [
                libc::pollfd { fd: inotify, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: stop_read, events: libc::POLLIN, revents: 0 },
            ];
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
            if n < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
            if fds[1].revents != 0 {
                break;
            }
            // Drain everything queued and note whether our file was touched.
            let mut ours = false;
            loop {
                let len = unsafe { libc::read(inotify, buf.as_mut_ptr().cast(), buf.len()) };
                if len <= 0 {
                    break;
                }
                ours |= events_mention(&buf[..len as usize], &name);
            }
            if ours {
                if let Ok(s) = Settings::load_from(&path) {
                    on_change(&s);
                }
            }
        }
        unsafe {
            libc::close(inotify);
            libc::close(stop_read);
        }
    })?;
    Ok(Watcher { stop_fd: stop_write, thread: Some(thread) })
}

/// Whether a buffer of `inotify_event`s names `name`.
fn events_mention(buf: &[u8], name: &OsStr) -> bool {
    let header = std::mem::size_of::<libc::inotify_event>();
    let mut off = 0;
    while off + header <= buf.len() {
        let ev: libc::inotify_event = unsafe { std::ptr::read_unaligned(buf[off..].as_ptr().cast()) };
        let start = off + header;
        let end = (start + ev.len as usize).min(buf.len());
        let raw = &buf[start..end];
        let len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        if &raw[..len] == name.as_bytes() {
            return true;
        }
        off = end;
    }
    false
}
