//! Display: UI and text scale and Night Light live in the settings file for
//! the desktop to apply; brightness goes straight to the backlight.

use serde_json::{Value, json};
use settings::Schedule;

use super::*;

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "get_display",
        category: Category::Display,
        doing: "Checking display",
        description: "Read display scale, text size, Night Light and screen brightness.",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_display,
    },
    Tool {
        name: "set_display_scale",
        category: Category::Display,
        doing: "Changing display scale",
        description: "Scale everything on screen (windows, text, icons). 1.0 is 100%, 2.0 is 200%; allowed 1.0-3.0.",
        effect: Effect::Change,
        schema: || object(&[("scale", json!({ "type": "number", "minimum": 1.0, "maximum": 3.0 }))], &["scale"]),
        confirm: never,
        run: set_display_scale,
    },
    Tool {
        name: "set_text_size",
        category: Category::Display,
        doing: "Changing text size",
        description: "Make text bigger or smaller without scaling the rest of the UI. 1.0 is normal; allowed 0.8-1.6. \"Bigger text\" is about 1.2.",
        effect: Effect::Change,
        schema: || object(&[("scale", json!({ "type": "number", "minimum": 0.8, "maximum": 1.6 }))], &["scale"]),
        confirm: never,
        run: set_text_size,
    },
    Tool {
        name: "set_night_light",
        category: Category::NightLight,
        doing: "Changing Night Light",
        description: "Night Light warms the screen colors. Set any of: enabled, temperature in Kelvin (2500 warm - 6500 neutral, default 4000), schedule (always, sunset = sunset to sunrise, custom with from/to as HH:MM).",
        effect: Effect::Change,
        schema: || {
            object(
                &[
                    ("enabled", json!({ "type": "boolean" })),
                    ("temperature", json!({ "type": "integer", "minimum": 2500, "maximum": 6500 })),
                    ("schedule", json!({ "type": "string", "enum": ["always", "sunset", "custom"] })),
                    ("from", json!({ "type": "string", "description": "HH:MM, for the custom schedule" })),
                    ("to", json!({ "type": "string", "description": "HH:MM, for the custom schedule" })),
                ],
                &[],
            )
        },
        confirm: never,
        run: set_night_light,
    },
    Tool {
        name: "set_brightness",
        category: Category::Display,
        doing: "Changing brightness",
        description: "Set the built-in screen's brightness in percent (1-100).",
        effect: Effect::Change,
        schema: || object(&[("percent", json!({ "type": "number", "minimum": 1, "maximum": 100 }))], &["percent"]),
        confirm: never,
        run: set_brightness,
    },
];

fn percent(v: f64) -> String {
    format!("{}%", (v * 100.0).round())
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

struct Backlight {
    device: String,
    value: u64,
    max: u64,
}

impl Backlight {
    fn percent(&self) -> u32 {
        if self.max == 0 { 0 } else { ((self.value as f64 / self.max as f64) * 100.0).round() as u32 }
    }
}

/// The first backlight under /sys/class/backlight (firmware ones last).
fn backlight(ctx: &Ctx) -> Option<Backlight> {
    let dir = ctx.sys("/sys/class/backlight");
    let mut names: Vec<String> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort_by_key(|n| (n.starts_with("acpi_video"), n.clone()));
    names.into_iter().find_map(|device| {
        let value = ctx.read(&format!("/sys/class/backlight/{device}/brightness"))?.parse().ok()?;
        let max = ctx.read(&format!("/sys/class/backlight/{device}/max_brightness"))?.parse().ok()?;
        Some(Backlight { device, value, max })
    })
}

fn display_state(ctx: &Ctx, s: &settings::Settings) -> Value {
    let n = &s.display.night_light;
    json!({
        "scale": s.display.scale,
        "text_scale": s.display.text_scale,
        "night_light": {
            "enabled": n.enabled,
            "temperature": n.temperature,
            "schedule": n.schedule.name(),
            "from": n.from,
            "to": n.to,
        },
        "brightness_percent": backlight(ctx).map(|b| b.percent()),
    })
}

fn get_display(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let s = ctx.load_settings()?;
    Ok(Outcome::new("Checked display", display_state(ctx, &s)))
}

fn set_display_scale(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let scale = round2(num_arg(args, "scale")?);
    let (lo, hi) = settings::SCALE_RANGE;
    if !(lo..=hi).contains(&scale) {
        return Err(format!("The scale must be between {} and {}.", percent(lo), percent(hi)));
    }
    let before = ctx.load_settings()?.display.scale;
    let s = ctx.update_settings(|s| s.display.scale = scale)?;
    if (s.display.scale - scale).abs() > 0.001 {
        return Err("The display scale didn't change.".into());
    }
    Ok(Outcome::new(format!("Set display scale to {}", percent(scale)), display_state(ctx, &s))
        .undo("set_display_scale", json!({ "scale": before })))
}

fn set_text_size(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let scale = round2(num_arg(args, "scale")?);
    let (lo, hi) = settings::TEXT_SCALE_RANGE;
    if !(lo..=hi).contains(&scale) {
        return Err(format!("The text size must be between {} and {}.", percent(lo), percent(hi)));
    }
    let before = ctx.load_settings()?.display.text_scale;
    let s = ctx.update_settings(|s| s.display.text_scale = scale)?;
    if (s.display.text_scale - scale).abs() > 0.001 {
        return Err("The text size didn't change.".into());
    }
    let verb = if scale > before {
        "Made text larger"
    } else if scale < before {
        "Made text smaller"
    } else {
        "Text size is already"
    };
    Ok(Outcome::new(format!("{verb} ({})", percent(scale)), display_state(ctx, &s))
        .undo("set_text_size", json!({ "scale": before })))
}

fn valid_time(t: &str) -> bool {
    let Some((h, m)) = t.split_once(':') else { return false };
    h.len() <= 2 && m.len() == 2 && h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60)
}

fn set_night_light(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let enabled = opt_bool(args, "enabled")?;
    let temperature = opt_num(args, "temperature")?.map(|t| t.round() as u32);
    let schedule = match opt_str(args, "schedule")? {
        Some(s) => Some(Schedule::parse(&s).ok_or("\u{201c}schedule\u{201d} must be always, sunset or custom.")?),
        None => None,
    };
    let from = opt_str(args, "from")?;
    let to = opt_str(args, "to")?;
    if enabled.is_none() && temperature.is_none() && schedule.is_none() && from.is_none() && to.is_none() {
        return Err("Say what to change: enabled, temperature or schedule.".into());
    }
    let (lo, hi) = settings::TEMPERATURE_RANGE;
    if temperature.is_some_and(|t| !(lo..=hi).contains(&t)) {
        return Err(format!("The temperature must be {lo}–{hi} K."));
    }
    for t in [&from, &to].into_iter().flatten() {
        if !valid_time(t) {
            return Err(format!("\u{201c}{t}\u{201d} isn't a time like 22:00."));
        }
    }
    let before = ctx.load_settings()?.display.night_light;
    let s = ctx.update_settings(|s| {
        let n = &mut s.display.night_light;
        if let Some(v) = enabled {
            n.enabled = v;
        }
        if let Some(v) = temperature {
            n.temperature = v;
        }
        if let Some(v) = schedule {
            n.schedule = v;
        }
        if let Some(v) = &from {
            n.from = v.clone();
        }
        if let Some(v) = &to {
            n.to = v.clone();
        }
    })?;
    let n = &s.display.night_light;
    if enabled.is_some_and(|v| v != n.enabled) || temperature.is_some_and(|v| v != n.temperature) || schedule.is_some_and(|v| v != n.schedule) {
        return Err("Night Light didn't change.".into());
    }
    let summary = match enabled {
        Some(on) => format!("Turned Night Light {}", on_off(on)),
        None if temperature.is_some() => format!("Set Night Light to {} K", n.temperature),
        None => "Changed the Night Light schedule".into(),
    };
    Ok(Outcome::new(summary, display_state(ctx, &s)).undo(
        "set_night_light",
        json!({
            "enabled": before.enabled,
            "temperature": before.temperature,
            "schedule": before.schedule.name(),
            "from": before.from,
            "to": before.to,
        }),
    ))
}

fn set_brightness(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let pct = num_arg(args, "percent")?.round();
    if !(1.0..=100.0).contains(&pct) {
        return Err("Brightness must be 1–100%.".into());
    }
    let b = backlight(ctx).ok_or("This computer has no adjustable built-in screen.")?;
    let before = b.percent();
    let raw = ((pct / 100.0) * b.max as f64).round().max(1.0) as u64;
    let raw_s = raw.to_string();
    if ctx.run.has("brightnessctl") {
        ctx.cmd("brightnessctl", &["--quiet", "--device", &b.device, "set", &raw_s])?;
    } else if ctx.run.has("busctl") {
        // logind lets the logged-in user set the backlight without root.
        ctx.cmd(
            "busctl",
            &[
                "call",
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                "SetBrightness",
                "ssu",
                "backlight",
                &b.device,
                &raw_s,
            ],
        )?;
    } else {
        return Err("Changing brightness needs brightnessctl or systemd's busctl.".into());
    }
    let after = backlight(ctx).map(|b| b.percent()).unwrap_or(before);
    if (after as f64 - pct).abs() > 1.5 {
        return Err(format!("Asked for {pct}% but brightness reads {after}%."));
    }
    let s = ctx.load_settings()?;
    Ok(Outcome::new(format!("Set brightness to {pct}%"), display_state(ctx, &s)).undo("set_brightness", json!({ "percent": before })))
}
