//! The virtual desktops, for the taskbar's pager, from the y7ya-os desktop's
//! socket (`desktop/src/ipc.rs`): we send `watch` and it sends the state as
//! one JSON line now and another whenever it changes; `space <n>` switches.
//!
//! A thread reads the socket and wakes the UI when the state changes. Under
//! other desktops (or while ours restarts) there's no socket: the pager
//! hides and the thread tries again every few seconds, without waking the
//! UI until something answers.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

/// How long to wait before trying to connect again.
const RETRY: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spaces {
    /// The desktop showing, from 0.
    pub active: usize,
    /// The screen's logical size, which window rects are relative to.
    pub screen: (f32, f32),
    pub spaces: Vec<Space>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Space {
    /// Bottom to top.
    pub windows: Vec<SpaceWindow>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpaceWindow {
    pub app_id: String,
    pub title: String,
    /// x, y, width, height in logical pixels from the screen's top left.
    pub rect: [f32; 4],
    pub minimized: bool,
}

/// Parses one line of `watch` output.
pub fn parse(line: &str) -> Option<Spaces> {
    let v: Value = serde_json::from_str(line).ok()?;
    let num = |v: &Value| v.as_f64().map(|n| n as f32);
    let screen = v.get("screen")?.as_array()?;
    let screen = (num(screen.first()?)?, num(screen.get(1)?)?);
    let spaces = v
        .get("spaces")?
        .as_array()?
        .iter()
        .map(|s| {
            let windows = s.get("windows").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            let windows = windows
                .iter()
                .filter_map(|w| {
                    let r = w.get("rect")?.as_array()?;
                    let rect = [num(r.first()?)?, num(r.get(1)?)?, num(r.get(2)?)?, num(r.get(3)?)?];
                    let text = |k: &str| w.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                    let minimized = w.get("minimized").and_then(Value::as_bool).unwrap_or(false);
                    Some(SpaceWindow { app_id: text("app_id"), title: text("title"), rect, minimized })
                })
                .collect();
            Space { windows }
        })
        .collect();
    let active = v.get("active")?.as_u64()? as usize;
    Some(Spaces { active, screen, spaces })
}

/// `$Y7_DESKTOP_SOCKET`, else `$XDG_RUNTIME_DIR/<wayland socket>.y7-desktop`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("Y7_DESKTOP_SOCKET") {
        return p.into();
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let display = std::env::var_os("WAYLAND_DISPLAY").unwrap_or_else(|| "wayland-0".into());
    let name = Path::new(&display).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "wayland-0".into());
    dir.join(format!("{name}.y7-desktop"))
}

/// The UI's handle: the latest state, and a way to switch.
#[derive(Clone, Default)]
pub struct Desktops {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// `None` while not connected.
    state: Option<Spaces>,
    /// Our end of the connection, for commands.
    writer: Option<UnixStream>,
}

impl Desktops {
    /// Follows the desktop at `path`, calling `wake` whenever what
    /// [`Desktops::state`] returns changes.
    pub fn start(path: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Desktops {
        let d = Desktops::default();
        let inner = d.inner.clone();
        std::thread::Builder::new()
            .name("desktops".into())
            .spawn(move || {
                loop {
                    if let Ok(stream) = UnixStream::connect(&path) {
                        follow(stream, &inner, wake.as_ref());
                        let mut i = inner.lock().unwrap();
                        i.writer = None;
                        if i.state.take().is_some() {
                            drop(i);
                            wake();
                        }
                    }
                    std::thread::sleep(RETRY);
                }
            })
            .expect("desktops thread");
        d
    }

    /// A fixed state with no desktop behind it (for snapshots).
    pub fn fixed(spaces: Spaces) -> Desktops {
        Desktops { inner: Arc::new(Mutex::new(Inner { state: Some(spaces), writer: None })) }
    }

    pub fn state(&self) -> Option<Spaces> {
        self.inner.lock().unwrap().state.clone()
    }

    /// Switches to desktop `n` (from 0).
    pub fn switch(&self, n: usize) {
        let inner = self.inner.lock().unwrap();
        if let Some(mut w) = inner.writer.as_ref() {
            let _ = w.write_all(format!("space {n}\n").as_bytes());
        }
    }
}

/// Reads state lines until the desktop goes away.
fn follow(stream: UnixStream, inner: &Mutex<Inner>, wake: &(dyn Fn() + Send + Sync)) {
    let Ok(mut writer) = stream.try_clone() else { return };
    if writer.write_all(b"watch\n").is_err() {
        return;
    }
    inner.lock().unwrap().writer = Some(writer);
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        // Anything else (replies to other commands) isn't state.
        let Some(spaces) = parse(&line) else { continue };
        let mut i = inner.lock().unwrap();
        if i.state.as_ref() != Some(&spaces) {
            i.state = Some(spaces);
            drop(i);
            wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    const STATE: &str = r#"{"active":1,"screen":[1280,800],"spaces":[{"windows":[{"app_id":"notes","title":"A \"note\"","rect":[10,20,640,400.5],"minimized":false},{"app_id":"files","title":"","rect":[0,0,100,100],"minimized":true}]},{"windows":[]},{"windows":[]}]}"#;

    #[test]
    fn parses_state() {
        let s = parse(STATE).unwrap();
        assert_eq!(s.active, 1);
        assert_eq!(s.screen, (1280.0, 800.0));
        assert_eq!(s.spaces.len(), 3);
        let w = &s.spaces[0].windows;
        assert_eq!(w.len(), 2);
        assert_eq!(w[0], SpaceWindow { app_id: "notes".into(), title: "A \"note\"".into(), rect: [10.0, 20.0, 640.0, 400.5], minimized: false });
        assert!(w[1].minimized);
        assert!(s.spaces[1].windows.is_empty());
        // Not state: other replies, broken lines, missing fields.
        assert_eq!(parse("ok"), None);
        assert_eq!(parse(r#"{"active":0,"spaces":[]}"#), None);
        assert_eq!(parse(r#"{"active":0,"screen":[1,1],"spaces":[]}"#).map(|s| s.spaces.len()), Some(0));
    }

    fn wait_for(what: impl Fn() -> bool) {
        let start = Instant::now();
        while !what() {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn follows_a_desktop() {
        let path = std::env::temp_dir().join(format!("y7-desktops-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let d = Desktops::start(path.clone(), Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }));

        // A fake desktop: expects `watch`, sends the state, takes a command.
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "watch\n");
        (&stream).write_all(format!("{STATE}\n").as_bytes()).unwrap();
        wait_for(|| wakes.load(Ordering::SeqCst) == 1);
        assert_eq!(d.state().unwrap().active, 1);

        d.switch(2);
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "space 2\n");
        // The same state again doesn't wake anyone.
        (&stream).write_all(format!("{STATE}\n{}\n", STATE.replace(r#""active":1"#, r#""active":2"#)).as_bytes()).unwrap();
        wait_for(|| wakes.load(Ordering::SeqCst) == 2);
        assert_eq!(d.state().unwrap().active, 2);

        // The desktop goes away: no state, one more wake.
        drop((stream, reader));
        wait_for(|| wakes.load(Ordering::SeqCst) == 3);
        assert_eq!(d.state(), None);
        drop(listener);
        std::fs::remove_file(&path).unwrap();
    }
}
