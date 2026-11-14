//! A headless device for testing: prints a pairing code, then logs the notes
//! whenever another device changes them.
//!
//! `cargo run -p notes-core --example peer -- <data-dir> [name]`
//! Type a line and press Enter to add it as a note.

use std::io::BufRead;
use std::sync::Arc;
use std::time::Duration;

use notes_core::{Core, DEFAULT_FOLDER, Listener, Note, now};

struct Log;

impl Listener for Log {
    fn notes_changed(&self) {
        println!("[changed]");
    }
    fn status_changed(&self) {}
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("data dir");
    let name = args.next().unwrap_or_else(|| "Test Peer".into());
    let core = Arc::new(Core::open(dir.into(), name, Arc::new(Log)));
    core.sync().start_pairing();
    let code = loop {
        if let Some(code) = core.sync().status().pairing_code {
            break code;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    println!("PAIRING CODE: {code}");

    let watcher = core.clone();
    std::thread::spawn(move || {
        let mut last = String::new();
        loop {
            let s = watcher.snapshot();
            let peers = watcher.sync().status().peers;
            let mut lines: Vec<String> = s
                .notes
                .iter()
                .map(|n| format!("  [{}{}] {:?}", n.id, if n.deleted_at.is_some() { " deleted" } else { "" }, n.body))
                .collect();
            lines.sort();
            let state = format!(
                "peers: {:?}\nfolders: {:?}\nnotes:\n{}",
                peers.iter().map(|p| (&p.name, p.connected)).collect::<Vec<_>>(),
                s.folders.iter().map(|f| &f.name).collect::<Vec<_>>(),
                lines.join("\n")
            );
            if state != last {
                println!("---\n{state}");
                last = state;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });

    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let s = core.snapshot();
        let mut notes = s.notes.clone();
        let t = now();
        notes.push(Note { id: notes_core::new_id(), folder: DEFAULT_FOLDER, body: line, created: t, modified: t, pinned: false, deleted_at: None });
        core.apply(&s.heads, &s.folders, &notes);
    }
    std::thread::sleep(Duration::from_secs(3600 * 24));
}
