mod calc;

use calc::engine::{Func, Key, key_for_char};
use calc::{CalculatorApp, Mode};
use ui::WindowOptions;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = if args.iter().any(|a| a == "--scientific") { Mode::Scientific } else { Mode::Basic };
    // `calculator --snapshot out.png [--light] [--scientific] [--keys 12+3*4=] [--menu]`
    // renders offscreen after pressing the keys (n is ±, c clear, r Rad, s sin,
    // d 2nd), with the mode menu open for `--menu`.
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let path = std::path::PathBuf::from(args.get(i + 1).expect("--snapshot needs a path"));
        let dark = !args.iter().any(|a| a == "--light");
        let keys = args.iter().position(|a| a == "--keys").and_then(|i| args.get(i + 1)).cloned().unwrap_or("0.1+0.2=".into());
        let menu = args.iter().any(|a| a == "--menu");
        ui::snapshot(mode.window_size(), 2.0, dark, &path, |cx| CalculatorApp::new(mode, cx), |app, cx| {
            for c in keys.chars() {
                let key = match c {
                    'n' => Key::Negate,
                    'c' => Key::Clear,
                    'r' => Key::RadDeg,
                    's' => Key::Func(Func::Sin),
                    'd' => Key::Second,
                    _ => key_for_char(c).expect("unknown key"),
                };
                app.press(key);
            }
            if menu {
                app.open_menu(cx);
            }
        });
        return;
    }

    ui::run(
        WindowOptions {
            title: "Calculator".into(),
            app_id: "calculator".into(),
            size: mode.window_size(),
            min_size: Mode::Basic.window_size(),
            decorations: false,
        },
        move |cx| CalculatorApp::new(mode, cx),
    );
}
