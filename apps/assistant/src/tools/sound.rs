//! Sound through PipeWire's `wpctl`: output volume, mute and output device.

use serde_json::{Value, json};

use super::*;

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "get_sound",
        category: Category::Sound,
        doing: "Checking sound",
        description: "Read output volume (percent), mute, the current output device and the other outputs.",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_sound,
    },
    Tool {
        name: "set_volume",
        category: Category::Sound,
        doing: "Changing volume",
        description: "Set the output volume in percent (0-100). Unmutes if it was muted. For \"louder\"/\"quieter\", read the volume first and change it by about 10-20 points.",
        effect: Effect::Change,
        schema: || object(&[("percent", json!({ "type": "number", "minimum": 0, "maximum": 100 }))], &["percent"]),
        confirm: never,
        run: set_volume,
    },
    Tool {
        name: "set_muted",
        category: Category::Sound,
        doing: "Changing mute",
        description: "Mute or unmute the sound output.",
        effect: Effect::Change,
        schema: || object(&[("muted", json!({ "type": "boolean" }))], &["muted"]),
        confirm: never,
        run: set_muted,
    },
    Tool {
        name: "set_output_device",
        category: Category::Sound,
        doing: "Switching sound output",
        description: "Play sound through another output (speakers, headphones, HDMI...). Pass the device id or (part of) its name from get_sound.",
        effect: Effect::Change,
        schema: || object(&[("device", json!({ "type": "string" }))], &["device"]),
        confirm: never,
        run: set_output_device,
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct Sink {
    pub id: u32,
    pub name: String,
    pub default: bool,
}

/// `Volume: 0.28` or `Volume: 0.28 [MUTED]` → (0.28, muted).
pub fn parse_volume(s: &str) -> Option<(f64, bool)> {
    let rest = s.trim().strip_prefix("Volume:")?.trim();
    let num = rest.split_whitespace().next()?.parse().ok()?;
    Some((num, rest.contains("[MUTED]")))
}

/// The sinks listed under "Audio → Sinks" in `wpctl status`.
pub fn parse_sinks(status: &str) -> Vec<Sink> {
    let mut sinks = Vec::new();
    let mut in_audio = false;
    let mut in_sinks = false;
    for line in status.lines() {
        if !line.starts_with(' ') && !line.trim().is_empty() {
            in_audio = line.trim() == "Audio";
            in_sinks = false;
            continue;
        }
        if !in_audio {
            continue;
        }
        let t = line.trim_start_matches([' ', '│']).trim_end();
        if t.starts_with("├─") || t.starts_with("└─") {
            in_sinks = t.ends_with("Sinks:");
            continue;
        }
        if !in_sinks {
            continue;
        }
        let (default, t) = match t.strip_prefix('*') {
            Some(r) => (true, r.trim_start()),
            None => (false, t),
        };
        let Some((id, rest)) = t.split_once(". ") else { continue };
        let Ok(id) = id.trim().parse() else { continue };
        let name = rest.split(" [vol:").next().unwrap_or(rest).trim().to_string();
        sinks.push(Sink { id, name, default });
    }
    sinks
}

struct Sound {
    volume: f64,
    muted: bool,
    sinks: Vec<Sink>,
}

fn read(ctx: &Ctx) -> Result<Sound, String> {
    let v = ctx.cmd("wpctl", &["get-volume", SINK])?;
    let (volume, muted) = parse_volume(&v).ok_or("There's no sound output.")?;
    let sinks = parse_sinks(&ctx.cmd("wpctl", &["status"])?);
    Ok(Sound { volume, muted, sinks })
}

fn pct(v: f64) -> u32 {
    (v * 100.0).round() as u32
}

fn state(s: &Sound) -> Value {
    let output = s.sinks.iter().find(|k| k.default);
    json!({
        "volume_percent": pct(s.volume),
        "muted": s.muted,
        "output": output.map(|k| json!({ "id": k.id, "name": k.name })),
        "outputs": s.sinks.iter().map(|k| json!({ "id": k.id, "name": k.name })).collect::<Vec<_>>(),
    })
}

fn get_sound(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let s = read(ctx)?;
    Ok(Outcome::new("Checked sound", state(&s)))
}

fn set_volume(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let target = num_arg(args, "percent")?.round();
    if !(0.0..=100.0).contains(&target) {
        return Err("Volume must be 0–100%.".into());
    }
    let before = read(ctx)?;
    ctx.cmd("wpctl", &["set-volume", "--limit", "1.0", SINK, &format!("{:.2}", target / 100.0)])?;
    if before.muted && target > 0.0 {
        ctx.cmd("wpctl", &["set-mute", SINK, "0"])?;
    }
    let after = read(ctx)?;
    if pct(after.volume) as f64 != target {
        return Err(format!("Asked for {target}% but the volume reads {}%.", pct(after.volume)));
    }
    Ok(Outcome::new(format!("Set volume to {target}%"), state(&after)).undo("set_volume", json!({ "percent": pct(before.volume) })))
}

fn set_muted(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let muted = bool_arg(args, "muted")?;
    let before = read(ctx)?;
    ctx.cmd("wpctl", &["set-mute", SINK, if muted { "1" } else { "0" }])?;
    let after = read(ctx)?;
    if after.muted != muted {
        return Err("Mute didn't change.".into());
    }
    let summary = if muted { "Muted sound" } else { "Unmuted sound" };
    Ok(Outcome::new(summary, state(&after)).undo("set_muted", json!({ "muted": before.muted })))
}

fn set_output_device(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let want = str_arg(args, "device")?;
    let before = read(ctx)?;
    let q = want.to_lowercase();
    let sink = before
        .sinks
        .iter()
        .find(|k| k.id.to_string() == want)
        .or_else(|| before.sinks.iter().find(|k| k.name.to_lowercase() == q))
        .or_else(|| before.sinks.iter().find(|k| k.name.to_lowercase().contains(&q)))
        .ok_or_else(|| {
            let names: Vec<&str> = before.sinks.iter().map(|k| k.name.as_str()).collect();
            format!("No output matches \u{201c}{want}\u{201d}. Outputs: {}.", names.join(", "))
        })?
        .clone();
    let prev = before.sinks.iter().find(|k| k.default).map(|k| k.id);
    ctx.cmd("wpctl", &["set-default", &sink.id.to_string()])?;
    let after = read(ctx)?;
    if !after.sinks.iter().any(|k| k.default && k.id == sink.id) {
        return Err(format!("Sound is still not playing through {}.", sink.name));
    }
    let mut o = Outcome::new(format!("Switched sound output to {}", sink.name), state(&after));
    if let Some(p) = prev.filter(|&p| p != sink.id) {
        o = o.undo("set_output_device", json!({ "device": p.to_string() }));
    }
    Ok(o)
}
