mod backend;
mod chat;
mod mcp;
mod tools;

use std::path::PathBuf;

use chat::AssistantApp;
use ui::WindowOptions;

/// `~/.local/share/assistant`, or `ASSISTANT_DATA_DIR`.
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ASSISTANT_DATA_DIR") {
        return PathBuf::from(d);
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("assistant")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| args.iter().position(|a| a == f).map(|i| args.get(i + 1).cloned().unwrap_or_default());
    let quick = has("--quick");

    // `assistant --mcp`: the settings tools as an MCP server on stdio (the CLIs start this).
    if has("--mcp") {
        mcp::serve(&tools::Ctx::system());
        return;
    }

    // `assistant --ask "prompt" [--backend codex] [--session id]` runs one turn in the terminal.
    if let Some(prompt) = value("--ask") {
        let kind = value("--backend").and_then(|b| backend::Kind::parse(&b)).unwrap_or(backend::Kind::Claude);
        let (tx, rx) = std::sync::mpsc::channel();
        let sink: backend::Sink = std::sync::Arc::new(move |e| {
            let _ = tx.send(e);
        });
        backend::start(kind, value("--session"), prompt, sink);
        for e in rx {
            println!("{e:?}");
            if e == backend::Event::Done {
                break;
            }
        }
        return;
    }

    // `assistant --snapshot out.png [--light] [--quick] [--empty | --signed-out] [--picker]` renders offscreen.
    if let Some(path) = value("--snapshot") {
        let dark = !has("--light");
        let size = if quick { (680.0, 440.0) } else { (720.0, 720.0) };
        ui::snapshot(size, 2.0, dark, &PathBuf::from(path), |cx| AssistantApp::new(quick, false, cx), |app, cx| {
            if has("--signed-out") {
                chat::demo::signed_out(app);
            } else if has("--empty") {
                chat::demo::empty(app);
            } else {
                chat::demo::conversation(app);
                cx.scroll_to_bottom("chat-scroll");
            }
            if has("--picker") {
                chat::demo::open_picker(app);
            }
        });
        return;
    }

    let (title, app_id, size, min_size) = if quick {
        ("Assistant", "assistant-quick", (680.0, 440.0), (480.0, 300.0))
    } else {
        ("Assistant", "assistant", (720.0, 680.0), (460.0, 420.0))
    };
    ui::run(
        WindowOptions { title: title.into(), app_id: app_id.into(), size, min_size, decorations: false },
        move |cx| AssistantApp::new(quick, true, cx),
    );
}
