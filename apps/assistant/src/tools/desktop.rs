//! Settings kept in the y7ya-os settings file: appearance, accent color,
//! wallpaper, Do Not Disturb and key repeat. The desktop applies them live.

use std::path::{Path, PathBuf};

use chrono::Timelike;
use serde_json::{Value, json};
use settings::{ACCENTS, Fit, Mode};

use super::*;

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "get_appearance",
        category: Category::Appearance,
        doing: "Checking appearance",
        description: "Read the appearance (light, dark or auto) and accent color.",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_appearance,
    },
    Tool {
        name: "set_appearance",
        category: Category::Appearance,
        doing: "Changing appearance",
        description: "Switch between light mode, dark mode, or auto (dark from sunset to sunrise). Also switches KDE apps' color scheme so they follow.",
        effect: Effect::Change,
        schema: || object(&[("mode", json!({ "type": "string", "enum": ["light", "dark", "auto"] }))], &["mode"]),
        confirm: never,
        run: set_appearance,
    },
    Tool {
        name: "set_accent_color",
        category: Category::Appearance,
        doing: "Changing accent color",
        description: "Set the system accent color used for buttons, selections and highlights.",
        effect: Effect::Change,
        schema: || {
            let names: Vec<&str> = ACCENTS.iter().map(|(n, _)| *n).collect();
            object(&[("color", json!({ "type": "string", "enum": names }))], &["color"])
        },
        confirm: never,
        run: set_accent_color,
    },
    Tool {
        name: "get_wallpaper",
        category: Category::Wallpaper,
        doing: "Checking wallpaper",
        description: "Read the current desktop wallpaper.",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_wallpaper,
    },
    Tool {
        name: "set_wallpaper",
        category: Category::Wallpaper,
        doing: "Changing wallpaper",
        description: "Set the desktop wallpaper to an image file. Use path \"default\" to go back to the built-in wallpaper.",
        effect: Effect::Change,
        schema: || {
            object(
                &[
                    ("path", json!({ "type": "string", "description": "Absolute path or ~/path to a PNG, JPEG, WebP, AVIF, JPEG XL or SVG image, or \"default\"." })),
                    ("fit", json!({ "type": "string", "enum": ["fill", "fit", "center"] })),
                ],
                &["path"],
            )
        },
        confirm: never,
        run: set_wallpaper,
    },
    Tool {
        name: "get_notifications",
        category: Category::Notifications,
        doing: "Checking notifications",
        description: "Read whether Do Not Disturb is on.",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_notifications,
    },
    Tool {
        name: "set_do_not_disturb",
        category: Category::Notifications,
        doing: "Changing Do Not Disturb",
        description: "Turn Do Not Disturb on (silence notification banners) or off.",
        effect: Effect::Change,
        schema: || object(&[("enabled", json!({ "type": "boolean" }))], &["enabled"]),
        confirm: never,
        run: set_do_not_disturb,
    },
    Tool {
        name: "get_keyboard",
        category: Category::Keyboard,
        doing: "Checking keyboard",
        description: "Read the key repeat rate and the delay before repeat starts.",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_keyboard,
    },
    Tool {
        name: "set_key_repeat",
        category: Category::Keyboard,
        doing: "Changing key repeat",
        description: "Set how fast a held key repeats (repeats per second, 1-100; default 30) and/or how long to hold before it starts repeating (milliseconds, 100-2000; default 500).",
        effect: Effect::Change,
        schema: || {
            object(
                &[
                    ("rate", json!({ "type": "integer", "minimum": 1, "maximum": 100 })),
                    ("delay_ms", json!({ "type": "integer", "minimum": 100, "maximum": 2000 })),
                ],
                &[],
            )
        },
        confirm: never,
        run: set_key_repeat,
    },
];

fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Light => "Light",
        Mode::Dark => "Dark",
        Mode::Auto => "Auto",
    }
}

/// The KDE color scheme from `kdeglobals`, if KDE is around.
fn kde_scheme(ctx: &Ctx) -> Option<String> {
    let s = std::fs::read_to_string(ctx.config_home.join("kdeglobals")).ok()?;
    s.lines().find_map(|l| l.strip_prefix("ColorScheme=")).map(|v| v.trim().to_string())
}

/// Switches KDE's color scheme to match, so Qt/KDE (and through them GTK)
/// apps follow. Returns the scheme in effect afterwards.
fn apply_kde(ctx: &Ctx, dark: bool) -> Result<Option<String>, String> {
    if !ctx.run.has("plasma-apply-colorscheme") {
        return Ok(None);
    }
    let current = kde_scheme(ctx);
    if current.as_deref().is_some_and(|c| c.to_lowercase().contains("dark") == dark) {
        return Ok(current);
    }
    let target = if dark { "BreezeDark" } else { "BreezeLight" };
    ctx.cmd("plasma-apply-colorscheme", &[target])?;
    let after = kde_scheme(ctx);
    if after.as_deref() != Some(target) {
        return Err(format!("Saved, but KDE's color scheme is still {}.", after.unwrap_or_else(|| "unchanged".into())));
    }
    Ok(after)
}

fn appearance_state(ctx: &Ctx, s: &settings::Settings) -> Value {
    json!({
        "mode": s.appearance.mode.name(),
        "accent": s.appearance.accent,
        "kde_color_scheme": kde_scheme(ctx),
    })
}

fn get_appearance(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let s = ctx.load_settings()?;
    Ok(Outcome::new("Checked appearance", appearance_state(ctx, &s)))
}

fn set_appearance(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let mode = Mode::parse(&str_arg(args, "mode")?).ok_or("\u{201c}mode\u{201d} must be light, dark or auto.")?;
    let before = ctx.load_settings()?.appearance.mode;
    let s = ctx.update_settings(|s| s.appearance.mode = mode)?;
    if s.appearance.mode != mode {
        return Err("The appearance didn't change.".into());
    }
    apply_kde(ctx, mode.is_dark_at(chrono::Local::now().hour()))?;
    let summary = match mode {
        Mode::Auto => "Set appearance to Auto".to_string(),
        m => format!("Changed appearance to {}", mode_label(m)),
    };
    Ok(Outcome::new(summary, appearance_state(ctx, &s)).undo("set_appearance", json!({ "mode": before.name() })))
}

fn set_accent_color(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let color = str_arg(args, "color")?.to_lowercase();
    if !ACCENTS.iter().any(|(n, _)| *n == color) {
        let names: Vec<&str> = ACCENTS.iter().map(|(n, _)| *n).collect();
        return Err(format!("Accent colors are: {}.", names.join(", ")));
    }
    let before = ctx.load_settings()?.appearance.accent;
    let s = ctx.update_settings(|s| s.appearance.accent = color.clone())?;
    if s.appearance.accent != color {
        return Err("The accent color didn't change.".into());
    }
    Ok(Outcome::new(format!("Changed accent color to {}", capitalize(&color)), appearance_state(ctx, &s))
        .undo("set_accent_color", json!({ "color": before })))
}

const IMAGE_EXTENSIONS: [&str; 10] = ["png", "jpg", "jpeg", "webp", "avif", "jxl", "svg", "bmp", "gif", "heic"];

fn expand_home(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(p),
    }
}

fn file_label(p: &str) -> String {
    Path::new(p).file_name().map_or_else(|| p.to_string(), |n| n.to_string_lossy().into_owned())
}

fn get_wallpaper(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let s = ctx.load_settings()?;
    let path = if s.wallpaper.path.is_empty() { Value::Null } else { json!(s.wallpaper.path) };
    Ok(Outcome::new("Checked wallpaper", json!({ "path": path, "fit": fit_name(s.wallpaper.fit) })))
}

fn fit_name(f: Fit) -> &'static str {
    match f {
        Fit::Fill => "fill",
        Fit::Fit => "fit",
        Fit::Center => "center",
    }
}

fn set_wallpaper(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let raw = str_arg(args, "path")?;
    let fit = match opt_str(args, "fit")?.as_deref() {
        None => None,
        Some("fill") => Some(Fit::Fill),
        Some("fit") => Some(Fit::Fit),
        Some("center") => Some(Fit::Center),
        Some(_) => return Err("\u{201c}fit\u{201d} must be fill, fit or center.".into()),
    };
    let path = if raw == "default" {
        String::new()
    } else {
        let p = expand_home(&raw);
        if !p.is_absolute() {
            return Err("Give the wallpaper's full path.".into());
        }
        if !p.is_file() {
            return Err(format!("There's no image at {}.", p.display()));
        }
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            return Err(format!("{} isn't an image file.", file_label(&raw)));
        }
        p.to_string_lossy().into_owned()
    };
    let before = ctx.load_settings()?.wallpaper;
    let s = ctx.update_settings(|s| {
        s.wallpaper.path = path.clone();
        if let Some(f) = fit {
            s.wallpaper.fit = f;
        }
    })?;
    if s.wallpaper.path != path {
        return Err("The wallpaper didn't change.".into());
    }
    let summary = if path.is_empty() { "Restored the default wallpaper".to_string() } else { format!("Set wallpaper to {}", file_label(&path)) };
    let prev = if before.path.is_empty() { "default".to_string() } else { before.path };
    Ok(Outcome::new(summary, json!({ "path": s.wallpaper.path, "fit": fit_name(s.wallpaper.fit) }))
        .undo("set_wallpaper", json!({ "path": prev, "fit": fit_name(before.fit) })))
}

fn get_notifications(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let s = ctx.load_settings()?;
    Ok(Outcome::new("Checked notifications", json!({ "do_not_disturb": s.notifications.do_not_disturb })))
}

fn set_do_not_disturb(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let on = bool_arg(args, "enabled")?;
    let before = ctx.load_settings()?.notifications.do_not_disturb;
    let s = ctx.update_settings(|s| s.notifications.do_not_disturb = on)?;
    if s.notifications.do_not_disturb != on {
        return Err("Do Not Disturb didn't change.".into());
    }
    Ok(Outcome::new(format!("Turned Do Not Disturb {}", on_off(on)), json!({ "do_not_disturb": on }))
        .undo("set_do_not_disturb", json!({ "enabled": before })))
}

fn keyboard_state(s: &settings::Settings) -> Value {
    json!({ "repeat_rate": s.keyboard.repeat_rate, "repeat_delay_ms": s.keyboard.repeat_delay })
}

fn get_keyboard(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let s = ctx.load_settings()?;
    Ok(Outcome::new("Checked keyboard", keyboard_state(&s)))
}

fn set_key_repeat(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let rate = opt_num(args, "rate")?.map(|r| r.round() as u32);
    let delay = opt_num(args, "delay_ms")?.map(|d| d.round() as u32);
    if rate.is_none() && delay.is_none() {
        return Err("Give a repeat rate, a delay, or both.".into());
    }
    let (lo, hi) = settings::REPEAT_RATE_RANGE;
    if rate.is_some_and(|r| !(lo..=hi).contains(&r)) {
        return Err(format!("The repeat rate must be {lo}–{hi} per second."));
    }
    let (lo, hi) = settings::REPEAT_DELAY_RANGE;
    if delay.is_some_and(|d| !(lo..=hi).contains(&d)) {
        return Err(format!("The delay must be {lo}–{hi} ms."));
    }
    let before = ctx.load_settings()?.keyboard;
    let s = ctx.update_settings(|s| {
        if let Some(r) = rate {
            s.keyboard.repeat_rate = r;
        }
        if let Some(d) = delay {
            s.keyboard.repeat_delay = d;
        }
    })?;
    if rate.is_some_and(|r| r != s.keyboard.repeat_rate) || delay.is_some_and(|d| d != s.keyboard.repeat_delay) {
        return Err("Key repeat didn't change.".into());
    }
    let summary = match (rate, delay) {
        (Some(r), Some(d)) => format!("Set key repeat to {r}/s after {d} ms"),
        (Some(r), None) => format!("Set key repeat rate to {r}/s"),
        (None, Some(d)) => format!("Set key repeat delay to {d} ms"),
        (None, None) => unreachable!(),
    };
    Ok(Outcome::new(summary, keyboard_state(&s))
        .undo("set_key_repeat", json!({ "rate": before.repeat_rate, "delay_ms": before.repeat_delay })))
}

pub(super) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
