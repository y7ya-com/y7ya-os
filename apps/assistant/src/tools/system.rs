//! Date & time (`timedatectl`) and About This Computer (read-only).

use serde_json::{Value, json};

use super::*;

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "get_date_time",
        category: Category::DateTime,
        doing: "Checking date & time",
        description: "Read the local date and time, the time zone and whether the clock syncs automatically.",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_date_time,
    },
    Tool {
        name: "set_time_zone",
        category: Category::DateTime,
        doing: "Changing time zone",
        description: "Set the system time zone, as an IANA name like Europe/Paris or America/New_York. Needs the user's confirmation, and may need administrator rights.",
        effect: Effect::Change,
        schema: || object(&[("time_zone", json!({ "type": "string" }))], &["time_zone"]),
        confirm: |args| {
            let tz = args["time_zone"].as_str().unwrap_or("this time zone");
            Some(Confirm {
                title: format!("Change the time zone to {}?", tz.replace('_', " ")),
                detail: "This changes the clock for everyone who uses this computer.".into(),
                button: "Change".into(),
                destructive: false,
            })
        },
        run: set_time_zone,
    },
    Tool {
        name: "about_this_computer",
        category: Category::About,
        doing: "Looking up this computer",
        description: "Read system information: computer model, OS, kernel, processor, memory, disk space, uptime.",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: about,
    },
];

/// `Key=Value` lines (from `timedatectl show`, `/etc/os-release` and the like).
pub fn parse_kv(s: &str, key: &str) -> Option<String> {
    s.lines()
        .find_map(|l| l.trim().strip_prefix(key).and_then(|r| r.strip_prefix('=')))
        .map(|v| v.trim().trim_matches('"').to_string())
}

fn time_zone(ctx: &Ctx) -> Option<String> {
    if ctx.run.has("timedatectl") {
        if let Some(tz) = ctx.cmd("timedatectl", &["show"]).ok().and_then(|s| parse_kv(&s, "Timezone")) {
            return Some(tz);
        }
    }
    let link = std::fs::read_link(ctx.sys("/etc/localtime")).ok()?;
    let s = link.to_string_lossy();
    s.split_once("zoneinfo/").map(|(_, tz)| tz.to_string())
}

fn get_date_time(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let now = chrono::Local::now();
    let show = if ctx.run.has("timedatectl") { ctx.cmd("timedatectl", &["show"]).unwrap_or_default() } else { String::new() };
    Ok(Outcome::new(
        "Checked date & time",
        json!({
            "local_time": now.format("%A %-d %B %Y, %H:%M").to_string(),
            "time_zone": time_zone(ctx),
            "utc_offset": now.format("%:z").to_string(),
            "automatic_time": parse_kv(&show, "NTP").map(|v| v == "yes"),
            "synchronized": parse_kv(&show, "NTPSynchronized").map(|v| v == "yes"),
        }),
    ))
}

fn set_time_zone(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let tz = str_arg(args, "time_zone")?.replace(' ', "_");
    let valid = !tz.contains("..") && !tz.starts_with('/') && ctx.sys(&format!("/usr/share/zoneinfo/{tz}")).is_file();
    if !valid {
        return Err(format!("\u{201c}{tz}\u{201d} isn't a known time zone. Use a name like Europe/Paris."));
    }
    let before = time_zone(ctx);
    if before.as_deref() == Some(tz.as_str()) {
        return Ok(Outcome::new(format!("The time zone is already {tz}"), json!({ "time_zone": tz })));
    }
    if let Err(e) = ctx.cmd("timedatectl", &["--no-ask-password", "set-timezone", &tz]) {
        let lower = e.to_lowercase();
        if lower.contains("authentication") || lower.contains("access denied") || lower.contains("not authorized") {
            return Err(format!(
                "Changing the time zone needs an administrator. In a terminal, run: sudo timedatectl set-timezone {tz}"
            ));
        }
        return Err(e);
    }
    let after = time_zone(ctx);
    if after.as_deref() != Some(tz.as_str()) {
        return Err(format!("The time zone is still {}.", after.unwrap_or_else(|| "unchanged".into())));
    }
    let mut o = Outcome::new(format!("Changed time zone to {}", tz.replace('_', " ")), json!({ "time_zone": tz }));
    if let Some(b) = before {
        o = o.undo("set_time_zone", json!({ "time_zone": b }));
    }
    Ok(o)
}

fn gib(bytes: f64) -> String {
    let g = bytes / (1u64 << 30) as f64;
    if g >= 100.0 { format!("{g:.0} GB") } else { format!("{g:.1} GB") }
}

fn about(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let os = ctx.read("/etc/os-release").and_then(|s| parse_kv(&s, "PRETTY_NAME"));
    let host = ctx.read("/etc/hostname").or_else(|| ctx.read("/proc/sys/kernel/hostname"));
    let vendor = ctx.read("/sys/class/dmi/id/sys_vendor");
    let product = ctx.read("/sys/class/dmi/id/product_name");
    let version = ctx.read("/sys/class/dmi/id/product_version").filter(|v| !v.is_empty() && !v.contains("Default") && !v.contains("Not"));
    let model = [vendor, product, version].into_iter().flatten().collect::<Vec<_>>().join(" ");
    let cpuinfo = ctx.read("/proc/cpuinfo").unwrap_or_default();
    let cpu = cpuinfo.lines().find_map(|l| l.strip_prefix("model name").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim().to_string()));
    let threads = cpuinfo.lines().filter(|l| l.starts_with("processor")).count();
    let mem = ctx
        .read("/proc/meminfo")
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("MemTotal:").map(|v| v.trim().trim_end_matches("kB").trim().to_string())))
        .and_then(|kb| kb.parse::<f64>().ok())
        .map(|kb| gib(kb * 1024.0));
    let disk = ctx.cmd("df", &["-B1", "--output=size,avail", "/"]).ok().and_then(|s| {
        let line = s.lines().nth(1)?;
        let mut it = line.split_whitespace().map(|v| v.parse::<f64>().ok());
        Some(json!({ "total": gib(it.next()??), "available": gib(it.next()??) }))
    });
    let uptime = ctx.read("/proc/uptime").and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok()).map(|secs| {
        let m = (secs / 60.0) as u64;
        match (m / 1440, m / 60 % 24, m % 60) {
            (0, 0, m) => format!("{m} min"),
            (0, h, m) => format!("{h} h {m} min"),
            (1, h, _) => format!("1 day {h} h"),
            (d, h, _) => format!("{d} days {h} h"),
        }
    });
    Ok(Outcome::new(
        "Looked up this computer",
        json!({
            "name": host,
            "model": Some(model).filter(|m| !m.is_empty()),
            "os": os,
            "desktop": std::env::var("XDG_CURRENT_DESKTOP").ok(),
            "kernel": ctx.read("/proc/sys/kernel/osrelease"),
            "processor": cpu,
            "threads": (threads > 0).then_some(threads),
            "memory": mem,
            "disk": disk,
            "uptime": uptime,
        }),
    ))
}
