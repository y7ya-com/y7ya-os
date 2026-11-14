mod reminders;

use std::sync::Arc;

use reminders::RemindersApp;
use reminders::model::{Smart, Store, View};
use sync_ui::SyncEvents;
use ui::WindowOptions;

const SIZE: (f32, f32) = (980.0, 660.0);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `reminders --snapshot out.png [--light] [--list today|scheduled|all|flagged|completed|<name>]
    // [--details] [--edit] [--search <text>] [--menu] [--list-menu] [--sync]` renders sample data offscreen.
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let path = std::path::PathBuf::from(args.get(i + 1).expect("--snapshot needs a path"));
        let dark = !args.iter().any(|a| a == "--light");
        let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
        let flag = |name: &str| args.iter().any(|a| a == name);
        let store = reminders::demo_store();
        let view = arg("--list").map(|name| match name.as_str() {
            "today" => View::Smart(Smart::Today),
            "scheduled" => View::Smart(Smart::Scheduled),
            "all" => View::Smart(Smart::All),
            "flagged" => View::Smart(Smart::Flagged),
            "completed" => View::Smart(Smart::Completed),
            other => View::List(store.lists.iter().find(|l| l.name.eq_ignore_ascii_case(other)).expect("no such list").id),
        });
        ui::snapshot(SIZE, 2.0, dark, &path, |cx| RemindersApp::new(store, None, cx), |app, cx| {
            if let Some(view) = view {
                app.set_view(view, cx);
            }
            if let Some(q) = arg("--search") {
                app.search_for(&q, cx);
            }
            if flag("--details") {
                app.show_details_preview(cx);
            }
            if flag("--edit") {
                app.edit_preview(cx);
            }
            if flag("--menu") || flag("--list-menu") {
                app.menu_preview(flag("--list-menu"), cx);
            }
            if flag("--sync") {
                app.open_sync();
            }
        });
        return;
    }

    // Reminders live in the y7-syncd daemon, shared with the other apps and
    // synced with paired devices; this starts it if needed.
    let events = Arc::new(SyncEvents::default());
    let client = match y7_sync::Client::connect_default(events.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("reminders: can't reach the sync service: {e:#}");
            std::process::exit(1);
        }
    };
    let core = reminders_core::Core::new(Arc::new(client));
    let store = Store::new(Arc::new(core));
    ui::run(
        WindowOptions {
            title: "Reminders".into(),
            app_id: "reminders".into(),
            size: SIZE,
            min_size: (680.0, 420.0),
            decorations: false,
        },
        move |cx| RemindersApp::new(store, Some(events), cx),
    );
}
