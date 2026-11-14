mod daemon;
mod history;
mod picker;
mod serve;

use history::{Content, Store, now_ms};
use picker::Picker;
use ui::WindowOptions;

const USAGE: &str = "usage: clipboard [--pick | --daemon | --snapshot <png> [--light] [--select <n>]]";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();

    if args.iter().any(|a| a == "--daemon") {
        if let Err(e) = daemon::run() {
            eprintln!("clipboard: {e}\nThe compositor must support wlr-data-control (y7ya-os, KWin, wlroots).");
            std::process::exit(1);
        }
        return;
    }
    if let Some(id) = arg("--serve") {
        if let Err(e) = serve::run(&id) {
            eprintln!("clipboard: {e}");
            std::process::exit(1);
        }
        return;
    }
    // `clipboard --snapshot out.png [--light]` renders the picker over sample history.
    if let Some(path) = arg("--snapshot") {
        let dark = !args.iter().any(|a| a == "--light");
        let store = demo_store();
        let select = arg("--select").and_then(|n| n.parse().ok()).unwrap_or(0);
        ui::snapshot(picker::SIZE, 2.0, dark, path.as_ref(), |cx| Picker::new(store, true, cx), |app, _| app.select_nth(select));
        return;
    }
    if args.len() > 1 && !args.iter().any(|a| a == "--pick") {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let store = Store::open(Store::default_dir());
    ui::run(
        WindowOptions {
            title: "Clipboard".into(),
            app_id: "clipboard".into(),
            size: picker::SIZE,
            min_size: (560.0, 360.0),
            decorations: false,
        },
        move |cx| Picker::new(store, false, cx),
    );
}

/// A throwaway history with a mix of entries, for previews.
fn demo_store() -> Store {
    let dir = std::env::temp_dir().join(format!("clipboard-preview-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir);
    let now = now_ms();
    let min = 60_000;
    let texts = [
        ("https://github.com/y7ya/y7ya-os/pull/42", 2 * min),
        ("fn main() {\n    println!(\"Hello from y7ya-os\");\n}", 9 * min),
        ("Meet at the corner café on Rue Oberkampf, 7:30 pm", 26 * min),
        ("cargo test --workspace -- --nocapture", 75 * min),
        ("The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. How vexingly quick daft zebras jump! Sphinx of black quartz, judge my vow.\n\nSecond paragraph: clipboard history keeps the last two hundred things you copied, text and pictures alike, and remembers the ones you pin.", 3 * 60 * min),
        ("hello@example.com", 26 * 60 * min),
        ("#ff9f0a", 30 * 60 * min),
        ("sudo pacman -Syu", 3 * 24 * 60 * min),
    ];
    for (t, age) in texts.iter().rev() {
        let _ = store.add(&Content::Text(t.to_string()), now - age);
    }
    let _ = store.add(&Content::Png(demo_png()), now - 5 * min);
    if let Some(e) = store.load().into_iter().find(|e| store.read_text(e).is_some_and(|t| t.starts_with("cargo test"))) {
        let _ = store.set_pinned(&e.id, true);
    }
    store
}

/// A small landscape: sky gradient, sun and hills.
fn demo_png() -> Vec<u8> {
    let (w, h) = (480u32, 300u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let mut c = [70.0 + 120.0 * fy, 130.0 + 80.0 * fy, 230.0 - 30.0 * fy];
            let (dx, dy) = (fx - 0.72, fy - 0.32);
            if (dx * dx * 2.6 + dy * dy).sqrt() < 0.11 {
                c = [255.0, 200.0, 90.0];
            }
            let hill1 = 0.68 + 0.08 * (fx * 7.0).sin();
            let hill2 = 0.80 + 0.06 * (fx * 4.0 + 1.5).cos();
            if fy > hill2 {
                c = [52.0, 140.0, 72.0];
            } else if fy > hill1 {
                c = [92.0, 180.0, 98.0];
            }
            px.extend([c[0] as u8, c[1] as u8, c[2] as u8, 255]);
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&px)).expect("encode png");
    out
}
