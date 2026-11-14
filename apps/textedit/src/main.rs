mod textedit;

use std::path::PathBuf;

use textedit::TextEditApp;
use ui::{App, WindowOptions};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let snapshot = value("--snapshot");
    let skip: Vec<Option<String>> = vec![snapshot.clone(), value("--sheet")];
    let files: Vec<PathBuf> =
        args.iter().filter(|a| !a.starts_with("--") && !skip.contains(&Some((*a).clone()))).map(|a| PathBuf::from(a)).collect();

    // `textedit --snapshot out.png [--light] [--find] [--mono] [--sheet save|open|unsaved] [file]`
    // renders offscreen, with sample text unless a file is given.
    if let Some(png) = snapshot {
        let dark = !flag("--light");
        let sheet = value("--sheet");
        let file = files.first().cloned();
        ui::snapshot((720.0, 520.0), 2.0, dark, &PathBuf::from(png), |cx| TextEditApp::new(file.clone(), cx), |app, cx| {
            if file.is_none() {
                app.doc().borrow_mut().insert(textedit::DEMO_TEXT);
            }
            if flag("--mono") {
                app.doc().borrow_mut().set_monospace(&cx.text.fonts, true);
            }
            if flag("--find") {
                app.open_find(true, cx);
                app.set_find_text("the", cx);
            }
            match sheet.as_deref() {
                Some("save") => app.begin_save_as(false, cx),
                Some("open") => app.begin_open(cx),
                Some("unsaved") => {
                    app.should_close(cx);
                }
                _ => {}
            }
        });
        return;
    }

    // Each extra file gets its own window, as TextEdit does.
    for extra in files.iter().skip(1) {
        textedit::spawn_window(Some(extra));
    }
    let file = files.into_iter().next();
    ui::run(
        WindowOptions {
            title: "TextEdit".into(),
            app_id: "textedit".into(),
            size: (720.0, 560.0),
            min_size: (360.0, 240.0),
            decorations: false,
        },
        move |cx| TextEditApp::new(file, cx),
    );
}
