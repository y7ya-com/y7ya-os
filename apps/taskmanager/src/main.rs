mod sys;
mod tm;

use tm::{Page, TaskManager};
use ui::WindowOptions;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let page = arg("--page").and_then(|p| Page::parse(&p)).unwrap_or(Page::Processes);

    // `taskmanager --snapshot out.png [--light] [--page performance]` renders
    // live process data (with sample graph history) offscreen.
    if let Some(path) = arg("--snapshot") {
        let dark = !args.iter().any(|a| a == "--light");
        let perf = arg("--perf");
        ui::snapshot((1180.0, 760.0), 2.0, dark, path.as_ref(), |cx| TaskManager::new(page, cx), |app, cx| {
            // A second sample after a moment so CPU percentages are real.
            std::thread::sleep(std::time::Duration::from_millis(500));
            app.on_wake_now(cx);
            tm::demo_history(app);
            app.preview_setup(perf.as_deref(), args.iter().any(|a| a == "--menu"), cx);
        });
        return;
    }

    ui::run(
        WindowOptions {
            title: "Task Manager".into(),
            app_id: "taskmanager".into(),
            size: (1180.0, 760.0),
            min_size: (760.0, 460.0),
            decorations: false,
        },
        move |cx| TaskManager::new(page, cx),
    );
}
