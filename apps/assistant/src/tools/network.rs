//! Wi-Fi through NetworkManager's `nmcli`.

use serde_json::{Value, json};

use super::*;

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "get_wifi",
        category: Category::Wifi,
        doing: "Checking Wi-Fi",
        description: "Read whether Wi-Fi is on and which network this computer is connected to (with signal strength).",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_wifi,
    },
    Tool {
        name: "list_wifi_networks",
        category: Category::Wifi,
        doing: "Looking for Wi-Fi networks",
        description: "List nearby Wi-Fi networks with signal strength, security and whether they're saved. Set rescan to look again (takes a few seconds), e.g. right after a phone hotspot was turned on.",
        effect: Effect::Read,
        schema: || object(&[("rescan", json!({ "type": "boolean" }))], &[]),
        confirm: never,
        run: list_wifi_networks,
    },
    Tool {
        name: "set_wifi_enabled",
        category: Category::Wifi,
        doing: "Changing Wi-Fi",
        description: "Turn Wi-Fi on or off. Turning it off needs the user's confirmation in the app.",
        effect: Effect::Change,
        schema: || object(&[("enabled", json!({ "type": "boolean" }))], &["enabled"]),
        confirm: |args| {
            (args["enabled"].as_bool() == Some(false)).then(|| Confirm {
                title: "Turn off Wi-Fi?".into(),
                detail: "You'll be disconnected from the internet, and the assistant can't reply until Wi-Fi is back on.".into(),
                button: "Turn Off".into(),
                destructive: true,
            })
        },
        run: set_wifi_enabled,
    },
    Tool {
        name: "connect_wifi",
        category: Category::Wifi,
        doing: "Joining Wi-Fi",
        description: "Join a Wi-Fi network by name (SSID), e.g. a phone's hotspot. Saved networks need no password. Needs the user's confirmation in the app. List networks first to get the exact name.",
        effect: Effect::Change,
        schema: || {
            object(
                &[
                    ("ssid", json!({ "type": "string" })),
                    ("password", json!({ "type": "string", "description": "Only for networks that aren't saved yet." })),
                ],
                &["ssid"],
            )
        },
        confirm: |args| {
            let ssid = args["ssid"].as_str().unwrap_or("this network");
            Some(Confirm {
                title: format!("Join \u{201c}{ssid}\u{201d}?"),
                detail: "Your computer will leave its current Wi-Fi network to join this one.".into(),
                button: "Join".into(),
                destructive: false,
            })
        },
        run: connect_wifi,
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    pub ssid: String,
    pub signal: u32,
    pub security: String,
    pub in_use: bool,
}

/// `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list`: one entry per
/// SSID (the strongest access point, or the one in use), hidden ones dropped,
/// strongest first.
pub fn parse_wifi_list(s: &str) -> Vec<Network> {
    let mut out: Vec<Network> = Vec::new();
    for line in s.lines().filter(|l| !l.trim().is_empty()) {
        let f = split_terse(line);
        if f.len() < 4 || f[1].is_empty() {
            continue;
        }
        let n = Network {
            in_use: f[0].trim() == "*",
            ssid: f[1].clone(),
            signal: f[2].trim().parse().unwrap_or(0),
            security: f[3].trim().to_string(),
        };
        match out.iter_mut().find(|o| o.ssid == n.ssid) {
            Some(o) => {
                o.in_use |= n.in_use;
                o.signal = o.signal.max(n.signal);
            }
            None => out.push(n),
        }
    }
    out.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    out
}

/// Names of saved Wi-Fi connections from `nmcli -t -f NAME,TYPE connection show`.
pub fn parse_saved(s: &str) -> Vec<String> {
    s.lines()
        .map(split_terse)
        .filter(|f| f.len() >= 2 && f[1] == "802-11-wireless")
        .map(|f| f[0].clone())
        .collect()
}

fn radio_on(ctx: &Ctx) -> Result<bool, String> {
    Ok(ctx.cmd("nmcli", &["radio", "wifi"])?.trim() == "enabled")
}

fn networks(ctx: &Ctx, rescan: bool) -> Result<Vec<Network>, String> {
    let out = ctx.cmd("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", if rescan { "yes" } else { "no" }])?;
    Ok(parse_wifi_list(&out))
}

fn bars(signal: u32) -> &'static str {
    match signal {
        70.. => "strong",
        45..70 => "good",
        25..45 => "weak",
        _ => "very weak",
    }
}

fn current_json(n: Option<&Network>) -> Value {
    n.map_or(Value::Null, |n| json!({ "ssid": n.ssid, "signal_percent": n.signal, "signal": bars(n.signal), "security": n.security }))
}

fn get_wifi(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    let on = radio_on(ctx)?;
    let nets = if on { networks(ctx, false)? } else { Vec::new() };
    let current = nets.iter().find(|n| n.in_use);
    Ok(Outcome::new("Checked Wi-Fi", json!({ "enabled": on, "connected_to": current_json(current) })))
}

fn list_wifi_networks(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    if !radio_on(ctx)? {
        return Ok(Outcome::new("Checked Wi-Fi", json!({ "enabled": false, "networks": [] })));
    }
    let nets = networks(ctx, opt_bool(args, "rescan")?.unwrap_or(false))?;
    let saved = parse_saved(&ctx.cmd("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"])?);
    let list: Vec<Value> = nets
        .iter()
        .take(25)
        .map(|n| {
            json!({
                "ssid": n.ssid,
                "signal_percent": n.signal,
                "secured": !n.security.is_empty() && n.security != "--",
                "saved": saved.contains(&n.ssid),
                "connected": n.in_use,
            })
        })
        .collect();
    Ok(Outcome::new(format!("Found {} Wi-Fi networks", list.len()), json!({ "enabled": true, "networks": list })))
}

fn set_wifi_enabled(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let on = bool_arg(args, "enabled")?;
    let before = radio_on(ctx)?;
    ctx.cmd("nmcli", &["radio", "wifi", on_off(on)])?;
    if radio_on(ctx)? != on {
        return Err(format!("Wi-Fi is still {}.", on_off(before)));
    }
    Ok(Outcome::new(format!("Turned Wi-Fi {}", on_off(on)), json!({ "enabled": on })).undo("set_wifi_enabled", json!({ "enabled": before })))
}

fn connect_wifi(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let ssid = str_arg(args, "ssid")?;
    let password = opt_str(args, "password")?;
    if !radio_on(ctx)? {
        ctx.cmd("nmcli", &["radio", "wifi", "on"])?;
    }
    let previous = networks(ctx, false)?.into_iter().find(|n| n.in_use).map(|n| n.ssid);
    if previous.as_deref() == Some(ssid.as_str()) {
        return Ok(Outcome::new(format!("Already on \u{201c}{ssid}\u{201d}"), json!({ "connected_to": ssid })));
    }
    let saved = parse_saved(&ctx.cmd("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"])?);
    let result = if saved.contains(&ssid) && password.is_none() {
        ctx.cmd("nmcli", &["--wait", "30", "connection", "up", "id", &ssid])
    } else {
        let mut a = vec!["--wait", "30", "device", "wifi", "connect", ssid.as_str()];
        if let Some(p) = &password {
            a.extend(["password", p.as_str()]);
        }
        ctx.cmd("nmcli", &a)
    };
    if let Err(e) = result {
        let hint = if e.to_lowercase().contains("secrets") || e.to_lowercase().contains("password") {
            " It needs the right password."
        } else {
            ""
        };
        return Err(format!("Couldn't join \u{201c}{ssid}\u{201d}: {e}.{hint}"));
    }
    let now = networks(ctx, false)?;
    let current = now.iter().find(|n| n.in_use);
    if current.map(|n| n.ssid.as_str()) != Some(ssid.as_str()) {
        return Err(format!("Tried to join \u{201c}{ssid}\u{201d}, but the computer isn't connected to it."));
    }
    let mut o = Outcome::new(format!("Joined \u{201c}{ssid}\u{201d}"), json!({ "enabled": true, "connected_to": current_json(current) }));
    if let Some(p) = previous {
        o = o.undo("connect_wifi", json!({ "ssid": p }));
    }
    Ok(o)
}
