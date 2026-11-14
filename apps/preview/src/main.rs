mod preview;

use std::path::PathBuf;

use preview::{PreviewApp, pages_for};
use ui::WindowOptions;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths: Vec<PathBuf> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && !(*i > 0 && ["--snapshot", "--rotate", "--zoom"].contains(&args[i - 1].as_str())))
        .map(|(_, a)| std::fs::canonicalize(a).unwrap_or_else(|_| PathBuf::from(a)))
        .collect();
    let (pages, first) = pages_for(&paths);

    // `preview --snapshot out.png [--light] [--rotate N] [--zoom Z] FILE...`
    // renders offscreen.
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let out = PathBuf::from(args.get(i + 1).expect("--snapshot needs a path"));
        let dark = !args.iter().any(|a| a == "--light");
        let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<f32>().ok());
        let (turns, zoom) = (value("--rotate").unwrap_or(0.0) as u32, value("--zoom"));
        ui::snapshot((1000.0, 700.0), 2.0, dark, &out, |cx| PreviewApp::new(pages, first, true, cx), |app, cx| {
            app.set_view(turns, zoom, cx)
        });
        return;
    }

    ui::run(
        WindowOptions {
            title: "Preview".into(),
            app_id: "preview".into(),
            size: (1000.0, 700.0),
            min_size: (480.0, 320.0),
            decorations: false,
        },
        move |cx| PreviewApp::new(pages, first, false, cx),
    );
}
