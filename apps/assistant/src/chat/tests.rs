use std::time::Duration;

use serde_json::json;

use super::*;
use crate::tools::Action;

/// Confirmed and undone calls in tests run read-only against a throwaway
/// settings file, so they fail harmlessly instead of changing anything.
fn harmless() -> Ctx {
    let dir = std::env::temp_dir().join(format!("assistant-ui-test-{}", std::process::id()));
    Ctx { read_only: true, settings: dir.join("settings.toml"), ..Ctx::system() }
}

fn wait_for_inbox(app: &mut AssistantApp, cx: &mut Cx) {
    for _ in 0..500 {
        if !app.inbox.queue.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    app.on_external(cx);
}

fn card<'a>(app: &'a AssistantApp, id: &str) -> &'a Card {
    app.items.iter().find_map(|i| if let Item::Tool(c) = i { (c.id == id).then_some(c) } else { None }).unwrap()
}

fn render(setup: impl FnOnce(&mut AssistantApp, &mut Cx)) {
    let path = std::env::var_os("ASSISTANT_TEST_PNG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("assistant-selftest-{}.png", std::process::id())));
    ui::snapshot((720.0, 720.0), 1.0, false, &path, |cx| AssistantApp::new(false, false, cx), |app, cx| {
        app.make_ctx = harmless;
        setup(app, cx);
    });
    assert!(path.exists());
}

#[test]
fn a_turn_with_cards_confirm_cancel_and_undo() {
    render(|app, cx| {
        app.set_status(Kind::Claude, Status::Ready);
        // Drive a turn by hand, without spawning a CLI.
        app.seq = 1;
        app.turn = Some(Turn::idle());
        let ev = |e| Msg::Backend(1, e);
        app.handle(ev(Event::Session("s1".into())));
        app.handle(ev(Event::ToolStart { id: "a".into(), name: "set_appearance".into(), args: json!({ "mode": "dark" }) }));
        assert!(app.busy());
        let outcome = Outcome {
            summary: "Changed appearance to Dark".into(),
            state: json!({ "mode": "dark" }),
            undo: Some(Action { tool: "set_appearance".into(), args: json!({ "mode": "light" }) }),
        };
        app.handle(ev(Event::ToolEnd { id: "a".into(), result: CallResult::Done(outcome) }));
        app.handle(ev(Event::Text("Done \u{2014} ".into())));
        app.handle(ev(Event::Text("you're in Dark mode.".into())));
        app.handle(ev(Event::ToolStart { id: "w".into(), name: "set_wifi_enabled".into(), args: json!({ "enabled": false }) }));
        let ask = tools::call(&Ctx { read_only: false, ..harmless() }, "set_wifi_enabled", &json!({ "enabled": false }), false);
        app.handle(ev(Event::ToolEnd { id: "w".into(), result: ask }));
        app.handle(ev(Event::ToolStart { id: "b".into(), name: "set_bluetooth_enabled".into(), args: json!({ "enabled": false }) }));
        let ask = tools::call(&Ctx { read_only: false, ..harmless() }, "set_bluetooth_enabled", &json!({ "enabled": false }), false);
        app.handle(ev(Event::ToolEnd { id: "b".into(), result: ask }));
        app.handle(ev(Event::Done));
        assert!(!app.running());
        assert_eq!(app.session.as_deref(), Some("s1"));
        assert!(matches!(&app.items[1], Item::Reply(r) if r == "Done \u{2014} you're in Dark mode."));
        assert!(matches!(card(app, "a").state, CardState::Done { undo: Undo::Available, .. }));
        assert!(matches!(card(app, "w").state, CardState::Ask(_)));

        // Late events from an old turn are dropped.
        app.handle(Msg::Backend(0, Event::Text("stale".into())));
        assert_eq!(app.items.len(), 4);

        // Cancel tells the model next turn; nothing runs.
        app.confirm("w", false);
        assert!(matches!(card(app, "w").state, CardState::Cancelled(_)));
        assert!(app.notes[0].contains("cancelled"));

        // Confirm runs the call (here it's refused by the read-only context).
        app.confirm("b", true);
        assert!(matches!(card(app, "b").state, CardState::Confirming(_)));
        wait_for_inbox(app, cx);
        assert!(matches!(&card(app, "b").state, CardState::Failed(e) if e.contains("read-only")));
        assert!(app.notes[1].contains("confirmed"));

        // Undo runs the card's undo action.
        app.undo("a");
        assert!(matches!(card(app, "a").state, CardState::Done { undo: Undo::Working, .. }));
        wait_for_inbox(app, cx);
        assert!(matches!(card(app, "a").state, CardState::Done { undo: Undo::Failed, .. }));
        assert!(app.notes[2].contains("Undo"));
        assert!(!app.busy());
    });
}

#[test]
fn stopping_marks_cards_and_missing_backends_show_setup() {
    render(|app, cx| {
        app.seq = 1;
        app.turn = Some(Turn::idle());
        app.handle(Msg::Backend(1, Event::ToolStart { id: "v".into(), name: "set_volume".into(), args: json!({ "percent": 30 }) }));
        app.stop();
        assert!(!app.running());
        assert!(matches!(&card(app, "v").state, CardState::Failed(e) if e == "Stopped"));

        app.new_chat(cx);
        assert!(app.items.is_empty());
        app.set_status(Kind::Claude, Status::NotInstalled);
        app.send("Switch to dark mode", cx);
        assert!(matches!(app.items.as_slice(), [Item::User(_), Item::Setup(Kind::Claude)]));
        assert!(!app.running(), "no CLI is started when it isn't installed");

        app.set_backend(Kind::Codex, cx);
        assert_eq!(app.kind, Kind::Codex);
        assert!(matches!(app.items.last(), Some(Item::Divider(_))));
    });
}

#[test]
fn backend_menu_works_from_the_keyboard() {
    render(|app, cx| {
        app.set_status(Kind::Claude, Status::Ready);
        app.set_status(Kind::Codex, Status::Ready);
        let key = |k: NamedKey| KeyPress { key: Key::Named(k), text: None, ctrl: false, shift: false, alt: false };

        // Escape closes the menu without switching.
        app.toggle_picker();
        assert!(app.on_key(&key(NamedKey::Escape), cx));
        assert!(app.picker.is_none());
        assert_eq!(app.kind, Kind::Claude);

        // Down, Down, Return picks the second row.
        app.toggle_picker();
        app.on_key(&key(NamedKey::ArrowDown), cx);
        app.on_key(&key(NamedKey::ArrowDown), cx);
        app.on_key(&key(NamedKey::Enter), cx);
        assert!(app.picker.is_none());
        assert_eq!(app.kind, Kind::Codex);
    });
}
