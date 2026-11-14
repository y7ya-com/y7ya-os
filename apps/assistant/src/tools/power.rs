//! Power: the power profile (`powerprofilesctl`) and battery status (sysfs).

use serde_json::{Value, json};

use super::*;

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "get_power",
        category: Category::Power,
        doing: "Checking battery",
        description: "Read battery charge, whether it's charging or on power, time remaining, and the power profile (power-saver, balanced, performance).",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_power,
    },
    Tool {
        name: "set_power_profile",
        category: Category::Power,
        doing: "Changing power mode",
        description: "Set the power profile: power-saver (longer battery life), balanced, or performance.",
        effect: Effect::Change,
        schema: || object(&[("profile", json!({ "type": "string", "enum": ["power-saver", "balanced", "performance"] }))], &["profile"]),
        confirm: never,
        run: set_power_profile,
    },
];

/// Profile names from `powerprofilesctl list`; the active one is starred.
pub fn parse_profiles(s: &str) -> (Vec<String>, Option<String>) {
    let mut all = Vec::new();
    let mut active = None;
    for line in s.lines() {
        let Some(name) = line.trim_end().strip_suffix(':') else { continue };
        let (star, name) = match name.strip_prefix('*') {
            Some(n) => (true, n.trim()),
            None => (false, name.trim()),
        };
        // Profile lines are flush left (after the star column); details are indented further.
        if line.starts_with("    ") || name.contains(char::is_whitespace) || name.is_empty() {
            continue;
        }
        if star {
            active = Some(name.to_string());
        }
        all.push(name.to_string());
    }
    (all, active)
}

#[derive(Debug, PartialEq)]
pub struct Battery {
    pub percent: u32,
    pub status: String,
    pub minutes_left: Option<u32>,
}

fn battery(ctx: &Ctx) -> (Option<Battery>, Option<bool>) {
    let dir = ctx.sys("/sys/class/power_supply");
    let Ok(entries) = std::fs::read_dir(dir) else { return (None, None) };
    let mut names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let mut bat = None;
    let mut ac = None;
    for n in names {
        let get = |f: &str| ctx.read(&format!("/sys/class/power_supply/{n}/{f}"));
        match get("type").as_deref() {
            Some("Battery") if bat.is_none() && get("scope").as_deref() != Some("Device") => {
                let Some(percent) = get("capacity").and_then(|c| c.parse().ok()) else { continue };
                let status = get("status").unwrap_or_else(|| "Unknown".into());
                let num = |a: &str, b: &str| get(a).or_else(|| get(b)).and_then(|v| v.parse::<f64>().ok());
                let now = num("energy_now", "charge_now");
                let rate = num("power_now", "current_now").filter(|r| *r > 0.0);
                let minutes_left = match (status.as_str(), now, rate) {
                    ("Discharging", Some(now), Some(rate)) => Some((now / rate * 60.0) as u32),
                    _ => None,
                };
                bat = Some(Battery { percent, status, minutes_left });
            }
            Some("Mains") => ac = Some(ac.unwrap_or(false) || get("online").as_deref() == Some("1")),
            _ => {}
        }
    }
    (bat, ac)
}

fn profiles(ctx: &Ctx) -> Option<(Vec<String>, Option<String>)> {
    ctx.run.has("powerprofilesctl").then(|| ctx.cmd("powerprofilesctl", &["list"]).ok().map(|s| parse_profiles(&s))).flatten()
}

fn state(ctx: &Ctx) -> Value {
    let (bat, ac) = battery(ctx);
    let profiles = profiles(ctx);
    json!({
        "battery": bat.map(|b| json!({ "percent": b.percent, "status": b.status, "minutes_left": b.minutes_left })),
        "on_power_adapter": ac,
        "power_profile": profiles.as_ref().and_then(|p| p.1.clone()),
        "available_profiles": profiles.map(|p| p.0),
    })
}

fn get_power(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    Ok(Outcome::new("Checked battery", state(ctx)))
}

fn set_power_profile(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let want = str_arg(args, "profile")?.to_lowercase().replace(' ', "-");
    let (all, before) = profiles(ctx).ok_or("Power modes aren't available here (they need power-profiles-daemon).")?;
    if !all.contains(&want) {
        return Err(format!("This computer's power modes are: {}.", all.join(", ")));
    }
    ctx.cmd("powerprofilesctl", &["set", &want])?;
    let after = profiles(ctx).and_then(|p| p.1);
    if after.as_deref() != Some(want.as_str()) {
        return Err(format!("The power mode is still {}.", after.unwrap_or_else(|| "unchanged".into())));
    }
    let label = match want.as_str() {
        "power-saver" => "Low Power",
        "performance" => "High Performance",
        _ => "Balanced",
    };
    let mut o = Outcome::new(format!("Switched to {label} mode"), state(ctx));
    if let Some(b) = before {
        o = o.undo("set_power_profile", json!({ "profile": b }));
    }
    Ok(o)
}
