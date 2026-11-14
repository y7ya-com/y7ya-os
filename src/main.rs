mod notes;
mod ui;

use notes::NotesApp;
use notes::model::Store;
use ui::WindowOptions;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `notes --snapshot out.png [--light] [--menu]` renders sample data offscreen.
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let path = std::path::PathBuf::from(args.get(i + 1).expect("--snapshot needs a path"));
        let dark = !args.iter().any(|a| a == "--light");
        ui::snapshot((1180.0, 740.0), 2.0, dark, &path, |cx| NotesApp::new(notes::demo_store(), cx), |_, _| {});
        return;
    }

    let store = Store::load(Store::data_path());
    ui::run(
        WindowOptions {
            title: "Notes".into(),
            app_id: "notes".into(),
            size: (1180.0, 740.0),
            min_size: (720.0, 420.0),
            decorations: false,
        },
        move |cx| NotesApp::new(store, cx),
    );
}
