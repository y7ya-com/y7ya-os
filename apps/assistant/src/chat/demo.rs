//! Scripted conversations for `--snapshot` renders and tests.

use serde_json::json;

use super::*;
use crate::tools::Action;

fn done(id: &str, tool: &str, summary: &str, undo: Option<(&str, Value)>) -> Item {
    let outcome = Outcome {
        summary: summary.into(),
        state: Value::Null,
        undo: undo.map(|(tool, args)| Action { tool: tool.into(), args }),
    };
    let undo = if outcome.undo.is_some() { Undo::Available } else { Undo::None };
    Item::Tool(Card { id: id.into(), tool: tool.into(), args: json!({}), state: CardState::Done { outcome, undo } })
}

pub fn conversation(app: &mut AssistantApp) {
    app.set_status(Kind::Claude, Status::Ready);
    app.set_status(Kind::Codex, Status::Ready);
    if app.quick {
        app.items = vec![
            Item::User("Turn the volume down a bit".into()),
            done("v0", "get_sound", "Checked sound", None),
            done("v1", "set_volume", "Set volume to 30%", Some(("set_volume", json!({ "percent": 45 })))),
            Item::Reply("Turned it down from 45% to 30%.".into()),
        ];
        return;
    }
    app.items = vec![
        Item::User("Switch to dark mode".into()),
        done("a1", "set_appearance", "Changed appearance to Dark", Some(("set_appearance", json!({ "mode": "light" })))),
        Item::Reply("Done \u{2014} you're in Dark mode now, and apps that follow the system switch too.".into()),
        Item::User("Set volume to 30% and tell me what Wi-Fi I'm on".into()),
        done("s1", "set_volume", "Set volume to 30%", Some(("set_volume", json!({ "percent": 55 })))),
        done("w1", "get_wifi", "Checked Wi-Fi", None),
        Item::Reply("Volume is at 30%. You're on \u{201c}Vodafone75DB8D_19\u{201d} with a good signal.".into()),
        Item::User("Connect to my phone's hotspot".into()),
        done("w2", "list_wifi_networks", "Found 5 Wi-Fi networks", None),
        Item::Tool(Card {
            id: "w3".into(),
            tool: "connect_wifi".into(),
            args: json!({ "ssid": "Pixel 9 Hotspot" }),
            state: CardState::Ask(Confirm {
                title: "Join \u{201c}Pixel 9 Hotspot\u{201d}?".into(),
                detail: "Your computer will leave its current Wi-Fi network to join this one.".into(),
                button: "Join".into(),
                destructive: false,
            }),
        }),
        Item::Reply("I found \u{201c}Pixel 9 Hotspot\u{201d}. Click Join on the card to switch to it.".into()),
        Item::User("And make the text bigger".into()),
        Item::Tool(Card { id: "t1".into(), tool: "set_text_size".into(), args: json!({ "scale": 1.2 }), state: CardState::Running }),
    ];
    // Show the turn as still going (spinner on the last card, stop button).
    app.turn = Some(Turn::idle());
}

/// The backend isn't installed: the setup card instead of suggestions.
pub fn signed_out(app: &mut AssistantApp) {
    app.set_status(Kind::Claude, Status::NotInstalled);
    app.set_status(Kind::Codex, Status::Ready);
}

pub fn open_picker(app: &mut AssistantApp) {
    app.picker = true;
}

pub fn empty(app: &mut AssistantApp) {
    app.set_status(Kind::Claude, Status::Ready);
    app.set_status(Kind::Codex, Status::SignedOut);
}
