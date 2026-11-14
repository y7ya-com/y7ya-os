//! `clipboard --daemon`: watches the clipboard through `wlr-data-control`
//! (the y7ya-os desktop, KWin, wlroots) and records each new selection.

use std::io::Read;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use wl_clipboard_rs::paste::Seat;
use wl_clipboard_rs::watch::{ClipboardEvent, ClipboardType, Watcher};

use crate::history::{Content, MAX_IMAGE, MAX_TEXT, Store, now_ms};

/// Mime types password managers add to mark a secret (KDE's, and macOS's
/// convention as used by some apps).
const SENSITIVE: [&str; 2] = ["x-kde-passwordManagerHint", "application/x-nspasteboard-concealed-type"];
const TEXT: [&str; 5] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING", "TEXT"];
/// How long a source app gets to hand over its data.
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// What to read from an offer: text if there is any, else a PNG, else nothing.
pub fn choose(mime_types: &[String]) -> Option<&'static str> {
    if mime_types.iter().any(|m| SENSITIVE.contains(&m.as_str())) {
        return None;
    }
    TEXT.iter().find(|t| mime_types.iter().any(|m| m == *t)).copied().or_else(|| {
        // Images only when there's no text: rich-text copies often carry a
        // rendered picture of the selection too.
        mime_types.iter().any(|m| m == "image/png").then_some("image/png")
    })
}

pub fn run() -> Result<(), String> {
    let store = Store::open(Store::default_dir());
    // The compositor may restart (or not be up yet); keep trying.
    let mut failures = 0;
    loop {
        match watch(&store) {
            Ok(()) => return Ok(()),
            Err(e) if failures < 5 => {
                eprintln!("clipboard: {e}; retrying");
                failures += 1;
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(e) => return Err(e),
        }
    }
}

fn watch(store: &Store) -> Result<(), String> {
    let mut watcher = Watcher::new(ClipboardType::Regular, Seat::Unspecified).map_err(|e| e.to_string())?;
    while let Some(event) = watcher.next_event().map_err(|e| e.to_string())? {
        let ClipboardEvent::Changed { mime_types, mut offer, .. } = event else { continue };
        let Some(mime) = choose(&mime_types) else { continue };
        let cap = if mime == "image/png" { MAX_IMAGE } else { MAX_TEXT };
        let Ok(pipe) = offer.receive(mime) else { continue };
        let Some(bytes) = read_capped(pipe, cap, READ_TIMEOUT) else { continue };
        let content = if mime == "image/png" { Content::Png(bytes) } else { Content::Text(String::from_utf8_lossy(&bytes).into_owned()) };
        if let Err(e) = store.add(&content, now_ms()) {
            eprintln!("clipboard: couldn't save: {e}");
        }
    }
    Ok(())
}

/// Reads to EOF, giving up past `cap` bytes or `timeout` (a stuck source app
/// mustn't hang the daemon).
fn read_capped(mut r: impl Read + AsRawFd, cap: usize, timeout: Duration) -> Option<Vec<u8>> {
    let fd = r.as_raw_fd();
    unsafe { libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK) };
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut pfd, 1, left.as_millis() as i32) } <= 0 {
            return None;
        }
        match r.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > cap {
                    return None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn picks_text_then_images_and_skips_secrets() {
        assert_eq!(choose(&s(&["text/html", "text/plain", "UTF8_STRING"])), Some("UTF8_STRING"));
        assert_eq!(choose(&s(&["image/png", "text/html"])), Some("image/png"));
        assert_eq!(choose(&s(&["image/png", "text/plain;charset=utf-8"])), Some("text/plain;charset=utf-8"));
        assert_eq!(choose(&s(&["text/plain", "x-kde-passwordManagerHint"])), None);
        assert_eq!(choose(&s(&["application/x-foo"])), None);
    }

    #[test]
    fn reads_pipes_with_limits() {
        use std::io::Write;
        let (r, mut w) = std::io::pipe().unwrap();
        w.write_all(b"hello").unwrap();
        drop(w);
        assert_eq!(read_capped(r, 100, Duration::from_secs(1)).as_deref(), Some(&b"hello"[..]));

        let (r, mut w) = std::io::pipe().unwrap();
        w.write_all(&[0u8; 200]).unwrap();
        drop(w);
        assert_eq!(read_capped(r, 100, Duration::from_secs(1)), None);

        // A writer that never finishes times out.
        let (r, _w) = std::io::pipe().unwrap();
        assert_eq!(read_capped(r, 100, Duration::from_millis(100)), None);
    }
}
