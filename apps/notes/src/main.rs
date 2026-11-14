mod notes;


use std::sync::Arc;

use notes::NotesApp;
use notes::model::Store;
use sync_ui::SyncEvents;
use ui::WindowOptions;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `notes --snapshot out.png [--light] [--sync] [--folder <name>|deleted] [--search <text>]
    // [--menu] [--folder-menu] [--rename] [--no-sidebar]` renders sample data offscreen.
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let path = std::path::PathBuf::from(args.get(i + 1).expect("--snapshot needs a path"));
        let dark = !args.iter().any(|a| a == "--light");
        let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
        let flag = |name: &str| args.iter().any(|a| a == name);
        let mut store = notes::demo_store();
        // `--sync` shows the pairing sheet with a real code from a throwaway identity.
        let sync = args.iter().any(|a| a == "--sync");
        if sync {
            let dir = std::env::temp_dir().join(format!("notes-preview-{}", std::process::id()));
            let engine = y7_sync::Engine::start(dir, y7_sync::device_name(), Arc::new(SyncEvents::default()));
            store.core = Some(Arc::new(notes_core::Core::with_hub(Arc::new(engine))));
        }
        ui::snapshot((1180.0, 740.0), 2.0, dark, &path, |cx| NotesApp::new(store, None, cx), |app, cx| {
            if let Some(name) = arg("--folder") {
                app.show_folder(&name, cx);
            }
            if let Some(q) = arg("--search") {
                app.search_for(&q, cx);
            }
            if flag("--no-sidebar") {
                app.hide_sidebar();
            }
            if flag("--rename") {
                app.rename_preview(cx);
            }
            if flag("--menu") || flag("--folder-menu") {
                app.menu_preview(flag("--folder-menu"), cx);
            }
            if sync {
                app.open_sync();
                app.wait_for_pairing_code();
            }
        });
        return;
    }

    // Sync (and the notebook) live in the y7-syncd daemon, shared with the
    // other apps; this starts it if needed.
    let events = Arc::new(SyncEvents::default());
    let client = match y7_sync::Client::connect_default(events.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("notes: can't reach the sync service: {e:#}");
            std::process::exit(1);
        }
    };
    let core = notes_core::Core::with_hub(Arc::new(client));
    core.import_legacy(&y7_sync::legacy_notes_dir());
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
