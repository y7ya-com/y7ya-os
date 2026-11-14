//! Putting an entry back on the clipboard. On Wayland the copying client has
//! to stay alive to hand the data over, so the picker starts a detached
//! `clipboard --serve <id>`, which serves until something else is copied.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use wl_clipboard_rs::copy::{MimeType, Options, Source};

use crate::history::{Kind, Store, now_ms};

/// Starts a background server for entry `id` (the picker then exits).
pub fn spawn(id: &str) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .args(["--serve", id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    // Reap it if we're still around when it exits.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// `clipboard --serve <id>`: offers the entry until the selection changes.
pub fn run(id: &str) -> Result<(), String> {
    let store = Store::open(Store::default_dir());
    let entry = store.load().into_iter().find(|e| e.id == id).ok_or("no such entry")?;
    let data = store.read(&entry).ok_or("entry content is missing")?;
    let _ = store.touch(id, now_ms());
    let mime = match entry.kind {
        Kind::Text => MimeType::Text,
        Kind::Image => MimeType::Specific("image/png".into()),
    };
    let mut opts = Options::new();
    opts.foreground(true);
    opts.copy(Source::Bytes(data.into()), mime).map_err(|e| e.to_string())
}
