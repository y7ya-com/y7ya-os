//! Applies `~/.config/y7ya-os/settings.toml` (written by the assistant) live:
//! appearance, wallpaper, keyboard repeat and display scale.

use std::sync::Arc;

use settings::{Mode, Settings};
use smithay::output::Scale;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::calloop::channel::{Event, channel};

use crate::state::Desk;

/// A decoded wallpaper, already scaled to cover the screen.
pub struct WallpaperImage {
    /// Premultiplied BGRA.
    pub px: Vec<u8>,
    pub size: (i32, i32),
    /// Integer scale the pixels were made for.
    pub scale: i32,
    /// Changes whenever the image does.
    pub generation: u64,
}

pub fn init(handle: &LoopHandle<'static, Desk>, desk: &mut Desk) {
    apply(desk, &Settings::load_or_default());
    let (tx, rx) = channel::<Settings>();
    match settings::watch(&settings::path(), move |s| {
        let _ = tx.send(s.clone());
    }) {
        Ok(w) => desk.settings_watcher = Some(w),
        Err(e) => tracing::warn!("not watching settings: {e}"),
    }
    let _ = handle.insert_source(rx, |event, _, desk: &mut Desk| {
        if let Event::Msg(s) = event {
            apply(desk, &s);
        }
    });
}

fn apply(desk: &mut Desk, s: &Settings) {
    // "Auto" defers to the system color scheme (which the assistant also sets).
    if std::env::var_os("Y7_THEME").is_none() {
        desk.dark = match s.appearance.mode {
            Mode::Dark => true,
            Mode::Light => false,
            Mode::Auto => crate::state::detect_dark(),
        };
    }

    if let Some(keyboard) = desk.seat.get_keyboard() {
        keyboard.change_repeat_info(s.keyboard.repeat_rate as i32, s.keyboard.repeat_delay as i32);
    }

    // Only an explicitly chosen scale overrides the one picked for the panel.
    let explicit_scale = std::fs::read_to_string(settings::path()).is_ok_and(|t| t.contains("scale"));
    if explicit_scale && std::env::var_os("Y7_SCALE").is_none() {
        if let Some(output) = &desk.output {
            if output.current_scale().fractional_scale() != s.display.scale {
                output.change_current_state(None, None, Some(Scale::Fractional(s.display.scale)), None);
            }
        }
    }

    if s.wallpaper.path != desk.wallpaper_path {
        desk.wallpaper_path = s.wallpaper.path.clone();
        load_wallpaper(desk);
    }
    desk.redraw();
}

/// Decodes and scales the wallpaper on a background thread.
pub fn load_wallpaper(desk: &mut Desk) {
    let path = desk.wallpaper_path.trim_start_matches("file://").to_string();
    if path.is_empty() {
        desk.wallpaper = None;
        return;
    }
    let out = desk.output_geometry();
    let scale = desk.output.as_ref().map_or(1.0, |o| o.current_scale().fractional_scale()).ceil() as i32;
    let (w, h) = ((out.size.w * scale) as u32, (out.size.h * scale) as u32);
    let generation = desk.wallpaper.as_ref().map_or(1, |w| w.generation + 1);
    let (tx, rx) = channel::<Option<WallpaperImage>>();
    std::thread::spawn(move || {
        let image = image::open(&path).ok().map(|img| {
            // Cover the screen, cropping the overflow like macOS "Fill Screen".
            let img = img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
            let px = img.pixels().flat_map(|p| [p[2], p[1], p[0], 255]).collect();
            WallpaperImage { px, size: (w as i32, h as i32), scale, generation }
        });
        if image.is_none() {
            tracing::warn!("couldn't load wallpaper {path}");
        }
        let _ = tx.send(image);
    });
    let _ = desk.loop_handle.insert_source(rx, |event, _, desk: &mut Desk| {
        if let Event::Msg(image) = event {
            desk.wallpaper = image.map(Arc::new);
            desk.redraw();
        }
    });
}
