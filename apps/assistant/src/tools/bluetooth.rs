//! Bluetooth through BlueZ's `bluetoothctl`.

use serde_json::{Value, json};

use super::*;

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "get_bluetooth",
        category: Category::Bluetooth,
        doing: "Checking Bluetooth",
        description: "Read whether Bluetooth is on and list paired devices (name, address, connected).",
        effect: Effect::Read,
        schema: no_args,
        confirm: never,
        run: get_bluetooth,
    },
    Tool {
        name: "set_bluetooth_enabled",
        category: Category::Bluetooth,
        doing: "Changing Bluetooth",
        description: "Turn Bluetooth on or off. Turning it off needs the user's confirmation in the app.",
        effect: Effect::Change,
        schema: || object(&[("enabled", json!({ "type": "boolean" }))], &["enabled"]),
        confirm: |args| {
            (args["enabled"].as_bool() == Some(false)).then(|| Confirm {
                title: "Turn off Bluetooth?".into(),
                detail: "Bluetooth keyboards, mice and headphones will disconnect.".into(),
                button: "Turn Off".into(),
                destructive: true,
            })
        },
        run: set_bluetooth_enabled,
    },
    Tool {
        name: "connect_bluetooth_device",
        category: Category::Bluetooth,
        doing: "Connecting",
        description: "Connect a paired Bluetooth device, by name (or part of it) or address. Turns Bluetooth on if needed.",
        effect: Effect::Change,
        schema: || object(&[("device", json!({ "type": "string" }))], &["device"]),
        confirm: never,
        run: connect_device,
    },
    Tool {
        name: "disconnect_bluetooth_device",
        category: Category::Bluetooth,
        doing: "Disconnecting",
        description: "Disconnect a Bluetooth device, by name or address. Needs the user's confirmation in the app.",
        effect: Effect::Change,
        schema: || object(&[("device", json!({ "type": "string" }))], &["device"]),
        confirm: |args| {
            let name = args["device"].as_str().unwrap_or("this device");
            Some(Confirm {
                title: format!("Disconnect \u{201c}{name}\u{201d}?"),
                detail: "If it's your keyboard or mouse, it stops working until you reconnect it.".into(),
                button: "Disconnect".into(),
                destructive: true,
            })
        },
        run: disconnect_device,
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub address: String,
    pub name: String,
}

/// `Device 78:87:12:59:A5:B7 R1mouse2` lines from `bluetoothctl devices`.
pub fn parse_devices(s: &str) -> Vec<Device> {
    s.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (address, name) = rest.split_once(' ').unwrap_or((rest, ""));
            (address.len() == 17).then(|| Device { address: address.to_string(), name: name.trim().to_string() })
        })
        .collect()
}

/// `Powered: yes` from `bluetoothctl show`; `None` when there's no adapter.
pub fn parse_powered(s: &str) -> Option<bool> {
    s.lines().find_map(|l| l.trim().strip_prefix("Powered:")).map(|v| v.trim() == "yes")
}

struct State {
    powered: bool,
    paired: Vec<Device>,
    connected: Vec<Device>,
}

fn read(ctx: &Ctx) -> Result<State, String> {
    let show = ctx.cmd("bluetoothctl", &["show"])?;
    let powered = parse_powered(&show).ok_or("This computer has no Bluetooth adapter (or the Bluetooth service isn't running).")?;
    let paired = parse_devices(&ctx.cmd("bluetoothctl", &["devices", "Paired"])?);
    let connected = parse_devices(&ctx.cmd("bluetoothctl", &["devices", "Connected"])?);
    Ok(State { powered, paired, connected })
}

fn state_json(s: &State) -> Value {
    let mut devices: Vec<Value> = s
        .paired
        .iter()
        .map(|d| json!({ "name": d.name, "address": d.address, "connected": s.connected.contains(d) }))
        .collect();
    // Connected but unpaired devices (rare) are worth showing too.
    for d in s.connected.iter().filter(|d| !s.paired.contains(d)) {
        devices.push(json!({ "name": d.name, "address": d.address, "connected": true }));
    }
    json!({ "enabled": s.powered, "devices": devices })
}

fn get_bluetooth(ctx: &Ctx, _: &Value) -> Result<Outcome, String> {
    Ok(Outcome::new("Checked Bluetooth", state_json(&read(ctx)?)))
}

fn set_power(ctx: &Ctx, on: bool) -> Result<(), String> {
    ctx.cmd("bluetoothctl", &["power", on_off(on)])?;
    Ok(())
}

fn set_bluetooth_enabled(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let on = bool_arg(args, "enabled")?;
    let before = read(ctx)?.powered;
    set_power(ctx, on)?;
    let after = read(ctx)?;
    if after.powered != on {
        return Err(format!("Bluetooth is still {}.", on_off(before)));
    }
    Ok(Outcome::new(format!("Turned Bluetooth {}", on_off(on)), state_json(&after)).undo("set_bluetooth_enabled", json!({ "enabled": before })))
}

fn find<'a>(list: &'a [Device], want: &str) -> Option<&'a Device> {
    let q = want.to_lowercase();
    list.iter()
        .find(|d| d.address.eq_ignore_ascii_case(want))
        .or_else(|| list.iter().find(|d| d.name.to_lowercase() == q))
        .or_else(|| list.iter().find(|d| d.name.to_lowercase().contains(&q)))
}

fn connect_device(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let want = str_arg(args, "device")?;
    let mut s = read(ctx)?;
    let d = find(&s.paired, &want)
        .ok_or_else(|| format!("No paired device matches \u{201c}{want}\u{201d}. New devices have to be paired once first."))?
        .clone();
    if s.connected.contains(&d) {
        return Ok(Outcome::new(format!("{} is already connected", d.name), state_json(&s)));
    }
    if !s.powered {
        set_power(ctx, true)?;
    }
    ctx.cmd("bluetoothctl", &["connect", &d.address]).map_err(|e| format!("Couldn't connect {}: {e}. Is it on and nearby?", d.name))?;
    s = read(ctx)?;
    if !s.connected.contains(&d) {
        return Err(format!("{} didn't connect. Is it on and nearby?", d.name));
    }
    Ok(Outcome::new(format!("Connected {}", d.name), state_json(&s)).undo("disconnect_bluetooth_device", json!({ "device": d.address })))
}

fn disconnect_device(ctx: &Ctx, args: &Value) -> Result<Outcome, String> {
    let want = str_arg(args, "device")?;
    let s = read(ctx)?;
    let Some(d) = find(&s.connected, &want).cloned() else {
        return Ok(Outcome::new(format!("\u{201c}{want}\u{201d} isn't connected"), state_json(&s)));
    };
    ctx.cmd("bluetoothctl", &["disconnect", &d.address])?;
    let after = read(ctx)?;
    if after.connected.contains(&d) {
        return Err(format!("{} is still connected.", d.name));
    }
    Ok(Outcome::new(format!("Disconnected {}", d.name), state_json(&after)).undo("connect_bluetooth_device", json!({ "device": d.address })))
}
