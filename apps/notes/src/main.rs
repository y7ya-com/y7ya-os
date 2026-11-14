mod notes;


use std::sync::Arc;

use notes::NotesApp;
use notes::model::Store;
use notes::sync::{SyncEvents, device_name};
use ui::WindowOptions;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `notes --snapshot out.png [--light] [--sync]` renders sample data offscreen.
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let path = std::path::PathBuf::from(args.get(i + 1).expect("--snapshot needs a path"));
        let dark = !args.iter().any(|a| a == "--light");
        let mut store = notes::demo_store();
        // `--sync` shows the pairing sheet with a real code from a throwaway identity.
        let sync = args.iter().any(|a| a == "--sync");
        if sync {
            let dir = std::env::temp_dir().join(format!("notes-preview-{}", std::process::id()));
            store.core = Some(Arc::new(notes_core::Core::open(dir, device_name(), Arc::new(SyncEvents::default()))));
        }
        ui::snapshot((1180.0, 740.0), 2.0, dark, &path, |cx| NotesApp::new(store, None, cx), |app, _| {
            if sync {
                app.open_sync();
                app.wait_for_pairing_code();
            }
        });
        return;
    }

    let events = Arc::new(SyncEvents::default());
    let core = notes_core::Core::open(Store::data_dir(), device_name(), events.clone());
    let store = Store::new(Arc::new(core));
    ui::run(
        WindowOptions {
            title: "Notes".into(),
            app_id: "notes".into(),
            size: (1180.0, 740.0),
            min_size: (720.0, 420.0),
            decorations: false,
        },
        move |cx| NotesApp::new(store, Some(events), cx),
    );
}
