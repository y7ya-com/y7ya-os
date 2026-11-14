mod files;

use std::path::PathBuf;

use files::places::Places;
use files::{FilesApp, Location, Options, ViewMode};
use ui::{Point, WindowOptions};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let has = |name: &str| args.iter().any(|a| a == name);

    // `files --snapshot out.png --root <fixture home> [--light] [--view icons|list|columns]
    //  [--dir <sub/dir>] [--column-root <sub/dir>] [--select a,b] [--quicklook]
    //  [--menu] [--trash] [--recents] [--empty-trash] [--search text] [--size WxH]`
    // renders offscreen, treating the fixture as the home folder.
    if let Some(out) = flag("--snapshot") {
        let root = PathBuf::from(flag("--root").expect("--snapshot needs --root <fixture home>"));
        let root = std::fs::canonicalize(&root).expect("fixture root");
        let dark = !has("--light");
        let size = flag("--size")
            .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
            .unwrap_or((1100.0, 680.0));
        let opts = Options {
            start: flag("--dir").map(|d| root.join(d)),
            view: flag("--view").and_then(|v| ViewMode::parse(&v)),
            snapshot: true,
        };
        let places = Places::rooted(&root);
        ui::snapshot(size, 2.0, dark, &PathBuf::from(out), |cx| FilesApp::new(places, opts, cx), |app, cx| {
            if has("--trash") {
                app.show_location(Location::Trash, cx);
            }
            if has("--recents") {
                app.show_location(Location::Recents, cx);
            }
            if let Some(r) = flag("--column-root") {
                app.set_column_root(root.join(r));
            }
            if let Some(q) = flag("--search") {
                app.set_query(&q, cx);
            }
            if let Some(sel) = flag("--select") {
                let names: Vec<&str> = sel.split(',').collect();
                app.select_names(&names);
            }
            if has("--quicklook") {
                app.open_quicklook();
            }
            if has("--menu") {
                app.open_context_menu(Point { x: 520.0, y: 220.0 }, cx);
                cx.mouse = Point { x: 560.0, y: 270.0 };
            }
            if has("--empty-trash") {
                app.confirm_empty_trash();
            }
        });
        return;
    }

    let start = args.get(1).filter(|a| !a.starts_with('-')).map(PathBuf::from).and_then(|p| {
        let p = std::fs::canonicalize(p).ok()?;
        Some(if p.is_dir() { p } else { p.parent()?.to_path_buf() })
    });
    ui::run(
        WindowOptions {
            title: "Files".into(),
            app_id: "files".into(),
            size: (1100.0, 680.0),
            min_size: (640.0, 400.0),
            decorations: false,
        },
        move |cx| FilesApp::new(Places::user(), Options { start, ..Default::default() }, cx),
    );
}
