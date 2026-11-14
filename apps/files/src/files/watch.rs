//! Watches the folders on screen with inotify. A background thread blocks
//! on the inotify descriptor, gathers bursts of events for a moment, then
//! wakes the UI once with the set of folders that changed.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ui::Waker;

/// Events are gathered for this long before the UI hears of them.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Default)]
struct State {
    by_wd: HashMap<i32, PathBuf>,
    by_path: HashMap<PathBuf, i32>,
    changed: HashSet<PathBuf>,
}

pub struct Watcher {
    fd: i32,
    state: Arc<Mutex<State>>,
}

impl Watcher {
    pub fn new(waker: Waker) -> Option<Watcher> {
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let state = Arc::new(Mutex::new(State::default()));
        let st = state.clone();
        std::thread::Builder::new().name("inotify".into()).spawn(move || read_loop(fd, st, waker)).ok()?;
        Some(Watcher { fd, state })
    }

    /// Watches exactly `dirs`, adding and dropping watches as needed.
    pub fn watch(&self, dirs: &[&Path]) {
        let mut st = self.state.lock().unwrap();
        let stale: Vec<PathBuf> = st.by_path.keys().filter(|p| !dirs.contains(&p.as_path())).cloned().collect();
        for p in stale {
            if let Some(wd) = st.by_path.remove(&p) {
                st.by_wd.remove(&wd);
                unsafe { libc::inotify_rm_watch(self.fd, wd) };
            }
        }
        for dir in dirs {
            if st.by_path.contains_key(*dir) {
                continue;
            }
            let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { continue };
            let mask = libc::IN_CREATE
                | libc::IN_DELETE
                | libc::IN_MOVED_FROM
                | libc::IN_MOVED_TO
                | libc::IN_CLOSE_WRITE
                | libc::IN_MODIFY
                | libc::IN_ATTRIB
                | libc::IN_DELETE_SELF
                | libc::IN_MOVE_SELF
                | libc::IN_ONLYDIR;
            let wd = unsafe { libc::inotify_add_watch(self.fd, c.as_ptr(), mask) };
            if wd >= 0 {
                st.by_wd.insert(wd, dir.to_path_buf());
                st.by_path.insert(dir.to_path_buf(), wd);
            }
        }
    }

    /// Folders that changed since the last call.
    pub fn take_changed(&self) -> Vec<PathBuf> {
        self.state.lock().unwrap().changed.drain().collect()
    }
}

fn read_loop(fd: i32, state: Arc<Mutex<State>>, waker: Waker) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        record(&buf[..n as usize], &state);
        // Let a burst (a copy, an unpacked archive) settle into one update.
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let deadline = std::time::Instant::now() + SETTLE;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() || unsafe { libc::poll(&mut pfd, 1, left.as_millis() as i32) } <= 0 {
                break;
            }
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            record(&buf[..n as usize], &state);
        }
        if !state.lock().unwrap().changed.is_empty() {
            waker.wake();
        }
    }
}

fn record(mut bytes: &[u8], state: &Mutex<State>) {
    let header = std::mem::size_of::<libc::inotify_event>();
    let mut st = state.lock().unwrap();
    while bytes.len() >= header {
        let ev: libc::inotify_event = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast()) };
        if let Some(dir) = st.by_wd.get(&ev.wd).cloned() {
            st.changed.insert(dir);
        }
        let len = header + ev.len as usize;
        bytes = &bytes[len.min(bytes.len())..];
    }
}

/// Sets `flag` and wakes the UI whenever something is mounted or unmounted
/// (the kernel flags `/proc/self/mounts` with POLLPRI).
pub fn watch_mounts(waker: Waker) -> Arc<std::sync::atomic::AtomicBool> {
    use std::io::{Read, Seek};
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f = flag.clone();
    let _ = std::thread::Builder::new().name("mounts".into()).spawn(move || {
        let Ok(mut file) = std::fs::File::open("/proc/self/mounts") else { return };
        let _ = file.read_to_end(&mut Vec::new());
        let mut pfd = libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(&file), events: libc::POLLPRI, revents: 0 };
        loop {
            let n = unsafe { libc::poll(&mut pfd, 1, -1) };
            if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return;
            }
            if n > 0 {
                f.store(true, std::sync::atomic::Ordering::Relaxed);
                waker.wake();
                // Reading the table again rearms the notification.
                let _ = file.seek(std::io::SeekFrom::Start(0));
                let _ = file.read_to_end(&mut Vec::new());
            }
        }
    });
    flag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_new_files() {
        let d = std::env::temp_dir().join(format!("files-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let w = Watcher::new(Waker::default()).unwrap();
        w.watch(&[d.as_path()]);
        std::fs::write(d.join("new.txt"), b"x").unwrap();
        let start = std::time::Instant::now();
        let mut changed = Vec::new();
        while changed.is_empty() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
            changed = w.take_changed();
        }
        assert_eq!(changed, vec![d.clone()]);
        w.watch(&[]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
