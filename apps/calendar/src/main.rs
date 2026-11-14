mod cal;

use std::sync::Arc;

use cal::layout::View;
use cal::model::Store;
use cal::{CalendarApp, today};
use sync_ui::SyncEvents;
use ui::WindowOptions;

const SIZE: (f32, f32) = (1280.0, 800.0);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `calendar --snapshot out.png [--light] [--view day|week|month|year] [--popover]`
    // renders sample data offscreen.
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let path = std::path::PathBuf::from(args.get(i + 1).expect("--snapshot needs a path"));
        let dark = !args.iter().any(|a| a == "--light");
        let view = match args.iter().position(|a| a == "--view").and_then(|i| args.get(i + 1)).map(String::as_str) {
            Some("day") => View::Day,
            Some("month") => View::Month,
            Some("year") => View::Year,
            _ => View::Week,
        };
        let popover = args.iter().any(|a| a == "--popover");
        let store = cal::model::demo_store(today());
        ui::snapshot(SIZE, 2.0, dark, &path, |cx| CalendarApp::new(store, None, cx), |app, cx| {
            app.set_view(view, cx);
            if popover {
                app.preview_popover(cx);
            }
        });
        return;
    }

    // `calendar --import file.ics`, or a file handed over by the desktop.
    let import = args
        .iter()
        .position(|a| a == "--import")
        .and_then(|i| args.get(i + 1))
        .or_else(|| args.iter().skip(1).find(|a| a.ends_with(".ics")))
        .cloned();

    // Calendars live in the y7-syncd daemon, shared with the other apps;
    // this starts it if needed.
    let events = Arc::new(SyncEvents::default());
    let client = match y7_sync::Client::connect_default(events.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("calendar: can't reach the sync service: {e:#}");
            std::process::exit(1);
        }
    };
    let core = calendar_core::Core::new(Arc::new(client));
    let mut store = Store::new(Arc::new(core));
    if let Some(path) = import {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let n = store.import(&text);
                store.save();
                println!("calendar: imported {n} events from {path}");
            }
            Err(e) => eprintln!("calendar: can't read {path}: {e}"),
        }
    }
    ui::run(
        WindowOptions {
            title: "Calendar".into(),
            app_id: "calendar".into(),
            size: SIZE,
            min_size: (760.0, 480.0),
            decorations: false,
        },
        move |cx| CalendarApp::new(store, Some(events), cx),
    );
}
