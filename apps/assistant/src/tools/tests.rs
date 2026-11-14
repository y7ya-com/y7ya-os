//! Tools against a fake system: no real setting is touched. The fake keeps
//! state, so each tool's read-back verification is exercised for real.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::runner::Output;
use super::*;

const WPCTL_STATUS: &str = include_str!("fixtures/wpctl-status.txt");
const NMCLI_LIST: &str = include_str!("fixtures/nmcli-wifi-list.txt");
const BT_SHOW: &str = include_str!("fixtures/bluetoothctl-show.txt");
const PROFILES: &str = include_str!("fixtures/powerprofilesctl-list.txt");
const TIMEDATECTL: &str = include_str!("fixtures/timedatectl-show.txt");

struct State {
    volume: f64,
    muted: bool,
    sink: u32,
    /// Ignore volume changes (to test verification).
    stuck_volume: bool,
    wifi: bool,
    ssid: Option<String>,
    saved: Vec<String>,
    bt_powered: bool,
    bt_connected: Vec<&'static str>,
    profile: String,
    defaults: HashMap<String, String>,
    tz: String,
    tz_needs_admin: bool,
    calls: Vec<String>,
}

struct Fake {
    state: Mutex<State>,
    root: PathBuf,
    missing: Vec<&'static str>,
}

const NETWORKS: [(&str, u32, &str); 4] =
    [("Vodafone75DB8D_19", 65, "WPA2 WPA3"), ("Pixel 9 Hotspot", 80, "WPA2"), ("Café: Guest", 58, ""), ("SKYUNHVS", 12, "WPA2")];
const BT_DEVICES: [(&str, &str); 3] = [("78:87:12:59:A5:B7", "R1 Mouse"), ("AC:80:0A:11:22:33", "WH-1000XM4"), ("F4:73:35:00:11:22", "Magic Keyboard")];

impl Fake {
    fn new(root: PathBuf) -> Fake {
        Fake {
            state: Mutex::new(State {
                volume: 0.28,
                muted: false,
                sink: 50,
                stuck_volume: false,
                wifi: true,
                ssid: Some("Vodafone75DB8D_19".into()),
                saved: vec!["Vodafone75DB8D_19".into(), "Pixel 9 Hotspot".into()],
                bt_powered: true,
                bt_connected: vec!["78:87:12:59:A5:B7"],
                profile: "balanced".into(),
                defaults: HashMap::from([("x-scheme-handler/https".to_string(), "firefox.desktop".to_string())]),
                tz: "Europe/London".into(),
                tz_needs_admin: false,
                calls: Vec::new(),
            }),
            root,
            missing: Vec::new(),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    fn wpctl_status(s: &State) -> String {
        let mut out = String::new();
        let mut in_sinks = false;
        for line in WPCTL_STATUS.lines() {
            if line.contains("├─") || line.contains("└─") {
                in_sinks = line.contains("Sinks:");
            }
            let mut l = line.to_string();
            if in_sinks && l.contains(". ") {
                l = l.replacen("│  *   ", "│      ", 1);
                if l.contains(&format!(" {}. ", s.sink)) {
                    l = l.replacen("│      ", "│  *   ", 1);
                }
            }
            out.push_str(&l);
            out.push('\n');
        }
        out
    }

    fn handle(&self, s: &mut State, program: &str, a: &[&str]) -> Output {
        fn ok(s: impl Into<String>) -> Output {
            Output::ok(s)
        }
        match (program, a) {
            ("wpctl", ["get-volume", _]) => ok(format!("Volume: {:.2}{}\n", s.volume, if s.muted { " [MUTED]" } else { "" })),
            ("wpctl", ["set-volume", "--limit", "1.0", _, v]) => {
                if !s.stuck_volume {
                    s.volume = v.parse::<f64>().unwrap().min(1.0);
                }
                ok("")
            }
            ("wpctl", ["set-mute", _, m]) => {
                s.muted = *m == "1";
                ok("")
            }
            ("wpctl", ["status"]) => ok(Self::wpctl_status(s)),
            ("wpctl", ["set-default", id]) => {
                s.sink = id.parse().unwrap();
                ok("")
            }
            ("nmcli", ["radio", "wifi"]) => ok(if s.wifi { "enabled\n" } else { "disabled\n" }),
            ("nmcli", ["radio", "wifi", v]) => {
                s.wifi = *v == "on";
                if !s.wifi {
                    s.ssid = None;
                }
                ok("")
            }
            ("nmcli", ["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", _]) => {
                let mut out = String::new();
                for (ssid, signal, sec) in NETWORKS {
                    let star = if s.ssid.as_deref() == Some(ssid) { "*" } else { " " };
                    out.push_str(&format!("{star}:{}:{signal}:{sec}\n", ssid.replace(':', "\\:")));
                }
                ok(out)
            }
            ("nmcli", ["-t", "-f", "NAME,TYPE", "connection", "show"]) => {
                let mut out = String::from("lo:loopback\n");
                for n in &s.saved {
                    out.push_str(&format!("{}:802-11-wireless\n", n.replace(':', "\\:")));
                }
                ok(out)
            }
            ("nmcli", ["--wait", "30", "connection", "up", "id", ssid]) if s.saved.iter().any(|n| n == ssid) => {
                s.ssid = Some(ssid.to_string());
                ok("Connection successfully activated\n")
            }
            ("nmcli", ["--wait", "30", "device", "wifi", "connect", ssid, rest @ ..]) => {
                let Some((_, _, sec)) = NETWORKS.iter().find(|n| n.0 == *ssid) else {
                    return Output::fail(format!("Error: No network with SSID '{ssid}' found.\n"));
                };
                if !sec.is_empty() && !matches!(rest, ["password", _]) {
                    return Output::fail("Error: Connection activation failed: Secrets were required, but not provided.\n");
                }
                s.ssid = Some(ssid.to_string());
                s.saved.push(ssid.to_string());
                ok("Device 'wlp0s20f3' successfully activated\n")
            }
            ("bluetoothctl", ["show"]) => ok(BT_SHOW.replace("Powered: yes", if s.bt_powered { "Powered: yes" } else { "Powered: no" })),
            ("bluetoothctl", ["devices", filter]) => {
                let mut out = String::new();
                for (addr, name) in BT_DEVICES {
                    if *filter == "Paired" || (s.bt_powered && s.bt_connected.contains(&addr)) {
                        out.push_str(&format!("Device {addr} {name}\n"));
                    }
                }
                ok(out)
            }
            ("bluetoothctl", ["power", v]) => {
                s.bt_powered = *v == "on";
                ok(format!("Changing power {v} succeeded\n"))
            }
            ("bluetoothctl", ["connect", addr]) => {
                let a = BT_DEVICES.iter().find(|d| d.0 == *addr).unwrap().0;
                s.bt_connected.push(a);
                ok("Connection successful\n")
            }
            ("bluetoothctl", ["disconnect", addr]) => {
                s.bt_connected.retain(|d| d != addr);
                ok("Successful disconnected\n")
            }
            ("powerprofilesctl", ["list"]) => ok(PROFILES.replace("* balanced", "  balanced").replace(&format!("  {}:", s.profile), &format!("* {}:", s.profile))),
            ("powerprofilesctl", ["set", p]) => {
                s.profile = p.to_string();
                ok("")
            }
            ("plasma-apply-colorscheme", [scheme]) => {
                std::fs::write(self.root.join("config/kdeglobals"), format!("[General]\nColorScheme={scheme}\n")).unwrap();
                ok(format!("Successfully applied the color scheme {scheme} to your current Plasma session\n"))
            }
            ("xdg-mime", ["query", "default", mime]) => ok(s.defaults.get(*mime).cloned().unwrap_or_default() + "\n"),
            ("xdg-mime", ["default", id, mimes @ ..]) => {
                for m in mimes {
                    s.defaults.insert(m.to_string(), id.to_string());
                }
                ok("")
            }
            ("timedatectl", ["show"]) => ok(TIMEDATECTL.replace("Europe/London", &s.tz)),
            ("timedatectl", ["--no-ask-password", "set-timezone", tz]) => {
                if s.tz_needs_admin {
                    return Output::fail("Failed to set time zone: Access denied\n");
                }
                s.tz = tz.to_string();
                ok("")
            }
            ("busctl", ["call", _, _, _, "SetBrightness", "ssu", "backlight", dev, v]) => {
                std::fs::write(self.root.join(format!("sys/class/backlight/{dev}/brightness")), v).unwrap();
                ok("")
            }
            ("df", _) => ok("     1B-blocks        Avail\n1000204886016 400081954406\n"),
            _ => Output::fail(format!("fake: unexpected {program} {a:?}")),
        }
    }
}

impl Runner for Fake {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("{program} {}", args.join(" ")));
        Ok(self.handle(&mut s, program, args))
    }

    fn has(&self, program: &str) -> bool {
        program != "brightnessctl" && !self.missing.contains(&program)
    }
}

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("assistant-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["config", "sys/class/backlight/intel_backlight", "sys/class/power_supply/BAT1", "sys/class/power_supply/ACAD", "apps", "usr/share/zoneinfo/Europe", "etc", "proc/sys/kernel"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let w = |p: &str, s: &str| std::fs::write(dir.join(p), s).unwrap();
    w("config/kdeglobals", "[General]\nColorScheme=BreezeLight\n");
    w("sys/class/backlight/intel_backlight/brightness", "48000");
    w("sys/class/backlight/intel_backlight/max_brightness", "96000");
    w("sys/class/power_supply/BAT1/type", "Battery");
    w("sys/class/power_supply/BAT1/capacity", "76");
    w("sys/class/power_supply/BAT1/status", "Discharging");
    w("sys/class/power_supply/BAT1/energy_now", "38000000");
    w("sys/class/power_supply/BAT1/power_now", "9500000");
    w("sys/class/power_supply/ACAD/type", "Mains");
    w("sys/class/power_supply/ACAD/online", "0");
    w("usr/share/zoneinfo/Europe/Paris", "TZif");
    w("etc/os-release", "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\n");
    w("etc/hostname", "laptop\n");
    w("proc/sys/kernel/osrelease", "7.2.11-arch1-1");
    w("proc/meminfo", "MemTotal:       32599864 kB\nMemFree:  1 kB\n");
    w("proc/uptime", "93784.12 100.0");
    w("proc/cpuinfo", "processor\t: 0\nmodel name\t: Intel(R) Core(TM) Ultra 7 155H\nprocessor\t: 1\nmodel name\t: Intel(R) Core(TM) Ultra 7 155H\n");
    w("apps/firefox.desktop", "[Desktop Entry]\nType=Application\nName=Firefox\nMimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n");
    w("apps/chromium.desktop", "[Desktop Entry]\nType=Application\nName=Chromium\nMimeType=text/html;x-scheme-handler/https;\n[Desktop Action new]\nName=New Window\n");
    w("apps/hidden.desktop", "[Desktop Entry]\nType=Application\nName=Hidden Browser\nNoDisplay=true\nMimeType=x-scheme-handler/https;\n");
    dir
}

fn setup(name: &str) -> (Ctx, Arc<Fake>) {
    let root = temp_root(name);
    let fake = Arc::new(Fake::new(root.clone()));
    let ctx = Ctx {
        run: fake.clone(),
        settings: root.join("config/y7ya-os/settings.toml"),
        root: root.clone(),
        config_home: root.join("config"),
        app_dirs: vec![root.join("apps")],
        read_only: false,
    };
    (ctx, fake)
}

fn done(r: CallResult) -> Outcome {
    match r {
        CallResult::Done(o) => o,
        other => panic!("expected Done, got {other:?}"),
    }
}

fn failed(r: CallResult) -> String {
    match r {
        CallResult::Failed(e) => e,
        other => panic!("expected Failed, got {other:?}"),
    }
}

fn undo(ctx: &Ctx, o: &Outcome) -> Outcome {
    let u = o.undo.as_ref().expect("undo");
    done(call(ctx, &u.tool, &u.args, true))
}

// -- parsing ------------------------------------------------------------------

#[test]
fn parses_wpctl() {
    let sinks = sound::parse_sinks(WPCTL_STATUS);
    let names: Vec<&str> = sinks.iter().map(|s| s.name.as_str()).collect();
    // Filters (easyeffects) and video sinks aren't outputs.
    assert_eq!(names, ["Built-in Audio Analog Stereo", "HDMI / DisplayPort 1 Output", "WH-1000XM4"]);
    assert_eq!(sinks.iter().find(|s| s.default).map(|s| s.id), Some(50));
    assert_eq!(sound::parse_volume("Volume: 0.28\n"), Some((0.28, false)));
    assert_eq!(sound::parse_volume("Volume: 0.40 [MUTED]"), Some((0.40, true)));
    assert_eq!(sound::parse_volume("Translate ID error"), None);
}

#[test]
fn parses_nmcli() {
    let nets = network::parse_wifi_list(NMCLI_LIST);
    let ssids: Vec<&str> = nets.iter().map(|n| n.ssid.as_str()).collect();
    // In use first, then by signal; one entry per SSID; hidden ones dropped; `\:` unescaped.
    assert_eq!(ssids, ["Vodafone75DB8D_19", "Pixel 9 Hotspot", "Café: Guest", "Vodafone-9C30", "SKYUNHVS"]);
    assert!(nets[0].in_use);
    assert_eq!(nets[0].signal, 65);
    assert_eq!(nets[2].security, "");
    assert_eq!(network::parse_saved("lo:loopback\nHome\\:5G:802-11-wireless\n"), ["Home:5G"]);
}

#[test]
fn parses_bluetoothctl_powerprofiles_and_kv() {
    assert_eq!(bluetooth::parse_powered(BT_SHOW), Some(true));
    assert_eq!(bluetooth::parse_powered("No default controller available"), None);
    let devs = bluetooth::parse_devices("Device 78:87:12:59:A5:B7 R1mouse2\nDevice AC:80:0A:11:22:33 Sony WH-1000XM4\ngarbage\n");
    assert_eq!(devs.len(), 2);
    assert_eq!(devs[1].name, "Sony WH-1000XM4");
    let (all, active) = power::parse_profiles(PROFILES);
    assert_eq!(all, ["performance", "balanced", "power-saver"]);
    assert_eq!(active.as_deref(), Some("balanced"));
    assert_eq!(system::parse_kv(TIMEDATECTL, "Timezone").as_deref(), Some("Europe/London"));
    assert_eq!(system::parse_kv(TIMEDATECTL, "NTP").as_deref(), Some("yes"));
}

#[test]
fn parses_desktop_files() {
    let a = apps::parse_desktop_file("x.desktop", "[Desktop Entry]\nName=X\nMimeType=a/b;c/d;\n[Desktop Action y]\nName=Y\n").unwrap();
    assert_eq!(a.name, "X");
    assert_eq!(a.mimes, ["a/b", "c/d"]);
    assert!(apps::parse_desktop_file("h.desktop", "[Desktop Entry]\nName=H\nNoDisplay=true\n").is_none());
}

// -- the registry ---------------------------------------------------------------

#[test]
fn every_tool_is_well_formed() {
    let mut names = std::collections::HashSet::new();
    for t in all() {
        assert!(names.insert(t.name), "duplicate {}", t.name);
        let s = (t.schema)();
        assert_eq!(s["type"], "object", "{}", t.name);
        assert!(s["properties"].is_object(), "{}", t.name);
        assert!(!t.description.is_empty());
    }
    // Things that can cut you off ask first.
    assert!((find("set_wifi_enabled").unwrap().confirm)(&json!({ "enabled": false })).is_some());
    assert!((find("set_wifi_enabled").unwrap().confirm)(&json!({ "enabled": true })).is_none());
    assert!((find("connect_wifi").unwrap().confirm)(&json!({ "ssid": "x" })).is_some());
    assert!((find("set_bluetooth_enabled").unwrap().confirm)(&json!({ "enabled": false })).is_some());
    assert!((find("set_time_zone").unwrap().confirm)(&json!({ "time_zone": "Europe/Paris" })).is_some());
    for t in all().filter(|t| t.effect == Effect::Read) {
        assert!((t.confirm)(&json!({})).is_none());
    }
}

#[test]
fn call_result_json_round_trips() {
    let o = Outcome::new("Set volume to 30%", json!({ "volume_percent": 30 })).undo("set_volume", json!({ "percent": 28 }));
    for r in [
        CallResult::Done(o),
        CallResult::NeedsConfirmation(Confirm { title: "Turn off Wi-Fi?".into(), detail: "d".into(), button: "Turn Off".into(), destructive: true }),
        CallResult::Failed("nope".into()),
    ] {
        assert_eq!(CallResult::from_json(&r.to_json()), r);
    }
}

#[test]
fn unknown_tools_and_bad_args_fail_cleanly() {
    let (ctx, fake) = setup("bad-args");
    assert!(failed(call(&ctx, "rm_rf", &json!({}), false)).contains("no tool"));
    assert!(failed(call(&ctx, "set_volume", &json!({ "percent": "loud" }), false)).contains("number"));
    assert!(failed(call(&ctx, "set_volume", &json!({ "percent": 150 }), false)).contains("0–100"));
    assert!(failed(call(&ctx, "set_appearance", &json!({ "mode": "purple" }), false)).contains("light, dark or auto"));
    assert!(failed(call(&ctx, "set_volume", &json!([1, 2]), false)).contains("object"));
    assert!(fake.calls().iter().all(|c| !c.contains("set-volume")));
}

#[test]
fn read_only_mode_refuses_changes() {
    let (mut ctx, _) = setup("read-only");
    ctx.read_only = true;
    assert!(failed(call(&ctx, "set_appearance", &json!({ "mode": "dark" }), false)).contains("read-only"));
    done(call(&ctx, "get_sound", &json!({}), false));
}

// -- changes, verification and undo ---------------------------------------------

#[test]
fn appearance_writes_settings_and_kde() {
    let (ctx, fake) = setup("appearance");
    let o = done(call(&ctx, "set_appearance", &json!({ "mode": "dark" }), false));
    assert_eq!(o.summary, "Changed appearance to Dark");
    assert_eq!(o.state["mode"], "dark");
    assert_eq!(o.state["kde_color_scheme"], "BreezeDark");
    assert_eq!(settings::Settings::load_from(&ctx.settings).unwrap().appearance.mode, settings::Mode::Dark);
    assert!(fake.calls().contains(&"plasma-apply-colorscheme BreezeDark".to_string()));

    let back = undo(&ctx, &o);
    assert_eq!(back.state["mode"], "auto");
    // Accent colors are validated and undoable too.
    let a = done(call(&ctx, "set_accent_color", &json!({ "color": "purple" }), false));
    assert_eq!(a.summary, "Changed accent color to Purple");
    assert_eq!(undo(&ctx, &a).state["accent"], "blue");
}

#[test]
fn appearance_without_kde_still_works() {
    let (ctx, _) = setup("appearance-nokde");
    let fake = Arc::new(Fake { missing: vec!["plasma-apply-colorscheme"], ..Fake::new(ctx.root.clone()) });
    let ctx = Ctx { run: fake.clone(), ..ctx };
    done(call(&ctx, "set_appearance", &json!({ "mode": "light" }), false));
    assert!(fake.calls().is_empty());
}

#[test]
fn volume_is_verified_and_undoable() {
    let (ctx, fake) = setup("volume");
    let o = done(call(&ctx, "set_volume", &json!({ "percent": 30 }), false));
    assert_eq!(o.summary, "Set volume to 30%");
    assert_eq!(o.state["volume_percent"], 30);
    assert_eq!(o.undo, Some(Action { tool: "set_volume".into(), args: json!({ "percent": 28 }) }));
    assert!(fake.calls().contains(&"wpctl set-volume --limit 1.0 @DEFAULT_AUDIO_SINK@ 0.30".to_string()));
    assert_eq!(undo(&ctx, &o).state["volume_percent"], 28);

    // Setting the volume unmutes, like on a Mac.
    done(call(&ctx, "set_muted", &json!({ "muted": true }), false));
    let o = done(call(&ctx, "set_volume", &json!({ "percent": "45%" }), false));
    assert_eq!(o.state["muted"], false);

    fake.state.lock().unwrap().stuck_volume = true;
    let e = failed(call(&ctx, "set_volume", &json!({ "percent": 80 }), false));
    assert!(e.contains("reads 45%"), "{e}");
}

#[test]
fn output_device_by_name() {
    let (ctx, _) = setup("output");
    let o = done(call(&ctx, "set_output_device", &json!({ "device": "hdmi" }), false));
    assert_eq!(o.summary, "Switched sound output to HDMI / DisplayPort 1 Output");
    assert_eq!(o.state["output"]["id"], 58);
    assert_eq!(undo(&ctx, &o).state["output"]["id"], 50);
    assert!(failed(call(&ctx, "set_output_device", &json!({ "device": "toaster" }), false)).contains("WH-1000XM4"));
}

#[test]
fn turning_wifi_off_needs_confirmation() {
    let (ctx, fake) = setup("wifi-off");
    let r = call(&ctx, "set_wifi_enabled", &json!({ "enabled": false }), false);
    let CallResult::NeedsConfirmation(c) = r else { panic!("{r:?}") };
    assert_eq!(c.button, "Turn Off");
    assert!(c.destructive);
    assert!(fake.calls().is_empty(), "nothing may run before the user confirms");

    let o = done(call(&ctx, "set_wifi_enabled", &json!({ "enabled": false }), true));
    assert_eq!(o.summary, "Turned Wi-Fi off");
    // Turning it back on is harmless, so undo needs no confirmation.
    assert_eq!(undo(&ctx, &o).state["enabled"], true);
}

#[test]
fn wifi_status_and_joining_a_hotspot() {
    let (ctx, _) = setup("wifi-join");
    let o = done(call(&ctx, "get_wifi", &json!({}), false));
    assert_eq!(o.state["connected_to"]["ssid"], "Vodafone75DB8D_19");
    assert_eq!(o.state["connected_to"]["signal"], "good");
    let list = done(call(&ctx, "list_wifi_networks", &json!({ "rescan": true }), false));
    let hotspot = list.state["networks"].as_array().unwrap().iter().find(|n| n["ssid"] == "Pixel 9 Hotspot").unwrap().clone();
    assert_eq!(hotspot["saved"], true);

    assert!(matches!(call(&ctx, "connect_wifi", &json!({ "ssid": "Pixel 9 Hotspot" }), false), CallResult::NeedsConfirmation(_)));
    let o = done(call(&ctx, "connect_wifi", &json!({ "ssid": "Pixel 9 Hotspot" }), true));
    assert_eq!(o.summary, "Joined \u{201c}Pixel 9 Hotspot\u{201d}");
    assert_eq!(o.undo.as_ref().unwrap().args, json!({ "ssid": "Vodafone75DB8D_19" }));
    assert_eq!(undo(&ctx, &o).state["connected_to"]["ssid"], "Vodafone75DB8D_19");

    // A secured network that isn't saved needs a password.
    let e = failed(call(&ctx, "connect_wifi", &json!({ "ssid": "SKYUNHVS" }), true));
    assert!(e.contains("password"), "{e}");
    done(call(&ctx, "connect_wifi", &json!({ "ssid": "SKYUNHVS", "password": "hunter22" }), true));
}

#[test]
fn bluetooth_devices() {
    let (ctx, _) = setup("bluetooth");
    let o = done(call(&ctx, "get_bluetooth", &json!({}), false));
    assert_eq!(o.state["enabled"], true);
    assert_eq!(o.state["devices"].as_array().unwrap().len(), 3);
    let e = failed(call(&ctx, "connect_bluetooth_device", &json!({ "device": "headphones" }), false));
    assert!(e.contains("paired"));
    let o = done(call(&ctx, "connect_bluetooth_device", &json!({ "device": "wh-1000" }), false));
    assert_eq!(o.summary, "Connected WH-1000XM4");
    assert!(matches!(call(&ctx, "disconnect_bluetooth_device", &json!({ "device": "WH-1000XM4" }), false), CallResult::NeedsConfirmation(_)));
    let back = undo(&ctx, &o);
    assert_eq!(back.summary, "Disconnected WH-1000XM4");
}

#[test]
fn power_and_battery() {
    let (ctx, _) = setup("power");
    let o = done(call(&ctx, "get_power", &json!({}), false));
    assert_eq!(o.state["battery"]["percent"], 76);
    assert_eq!(o.state["battery"]["minutes_left"], 240);
    assert_eq!(o.state["on_power_adapter"], false);
    assert_eq!(o.state["power_profile"], "balanced");
    let o = done(call(&ctx, "set_power_profile", &json!({ "profile": "power-saver" }), false));
    assert_eq!(o.summary, "Switched to Low Power mode");
    assert_eq!(undo(&ctx, &o).state["power_profile"], "balanced");
}

#[test]
fn brightness_through_logind() {
    let (ctx, fake) = setup("brightness");
    assert_eq!(done(call(&ctx, "get_display", &json!({}), false)).state["brightness_percent"], 50);
    let o = done(call(&ctx, "set_brightness", &json!({ "percent": 75 }), false));
    assert_eq!(o.state["brightness_percent"], 75);
    assert!(fake.calls().iter().any(|c| c.contains("SetBrightness ssu backlight intel_backlight 72000")));
    assert_eq!(undo(&ctx, &o).state["brightness_percent"], 50);
}

#[test]
fn display_night_light_and_text() {
    let (ctx, _) = setup("display");
    let o = done(call(&ctx, "set_text_size", &json!({ "scale": 1.2 }), false));
    assert_eq!(o.summary, "Made text larger (120%)");
    let o = done(call(&ctx, "set_night_light", &json!({ "enabled": true, "temperature": 3400 }), false));
    assert_eq!(o.summary, "Turned Night Light on");
    assert_eq!(o.state["night_light"]["temperature"], 3400);
    assert_eq!(undo(&ctx, &o).state["night_light"]["enabled"], false);
    assert!(failed(call(&ctx, "set_night_light", &json!({ "schedule": "custom", "from": "25:00" }), false)).contains("25:00"));
    let o = done(call(&ctx, "set_display_scale", &json!({ "scale": 1.5 }), false));
    assert_eq!(o.summary, "Set display scale to 150%");
    let s = settings::Settings::load_from(&ctx.settings).unwrap();
    assert_eq!(s.display.scale, 1.5);
    assert_eq!(s.display.text_scale, 1.2);
}

#[test]
fn settings_file_tools() {
    let (ctx, _) = setup("prefs");
    let o = done(call(&ctx, "set_do_not_disturb", &json!({ "enabled": true }), false));
    assert_eq!(o.summary, "Turned Do Not Disturb on");
    let o = done(call(&ctx, "set_key_repeat", &json!({ "rate": 50 }), false));
    assert_eq!(o.state["repeat_rate"], 50);
    assert_eq!(o.state["repeat_delay_ms"], 500);
    assert!(failed(call(&ctx, "set_key_repeat", &json!({}), false)).contains("rate"));

    let img = ctx.root.join("beach.jpg");
    std::fs::write(&img, "jpeg").unwrap();
    let o = done(call(&ctx, "set_wallpaper", &json!({ "path": img.to_string_lossy() }), false));
    assert_eq!(o.summary, "Set wallpaper to beach.jpg");
    assert_eq!(undo(&ctx, &o).summary, "Restored the default wallpaper");
    assert!(failed(call(&ctx, "set_wallpaper", &json!({ "path": "/nope/x.png" }), false)).contains("no image"));
    let txt = ctx.root.join("notes.txt");
    std::fs::write(&txt, "x").unwrap();
    assert!(failed(call(&ctx, "set_wallpaper", &json!({ "path": txt.to_string_lossy() }), false)).contains("isn't an image"));
}

#[test]
fn a_broken_settings_file_is_not_overwritten() {
    let (ctx, _) = setup("broken");
    std::fs::create_dir_all(ctx.settings.parent().unwrap()).unwrap();
    std::fs::write(&ctx.settings, "[appearance\nmode=").unwrap();
    assert!(failed(call(&ctx, "set_do_not_disturb", &json!({ "enabled": true }), false)).contains("TOML"));
    assert_eq!(std::fs::read_to_string(&ctx.settings).unwrap(), "[appearance\nmode=");
}

#[test]
fn default_apps() {
    let (ctx, _) = setup("apps");
    let o = done(call(&ctx, "get_default_apps", &json!({}), false));
    assert_eq!(o.state["web_browser"]["name"], "Firefox");
    let o = done(call(&ctx, "list_apps_for", &json!({ "kind": "web_browser" }), false));
    let names: Vec<&str> = o.state["apps"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Chromium", "Firefox"]);
    let o = done(call(&ctx, "set_default_app", &json!({ "kind": "web_browser", "app": "chromium" }), false));
    assert_eq!(o.summary, "Made Chromium the default web browser");
    assert_eq!(undo(&ctx, &o).summary, "Made Firefox the default web browser");
    assert!(failed(call(&ctx, "set_default_app", &json!({ "kind": "web_browser", "app": "Hidden Browser" }), false)).contains("Chromium, Firefox"));
}

#[test]
fn time_zone_needs_confirmation_and_maybe_admin() {
    let (ctx, fake) = setup("tz");
    assert_eq!(done(call(&ctx, "get_date_time", &json!({}), false)).state["time_zone"], "Europe/London");
    assert!(matches!(call(&ctx, "set_time_zone", &json!({ "time_zone": "Europe/Paris" }), false), CallResult::NeedsConfirmation(_)));
    assert!(failed(call(&ctx, "set_time_zone", &json!({ "time_zone": "Mars/Olympus" }), true)).contains("isn't a known"));
    assert!(failed(call(&ctx, "set_time_zone", &json!({ "time_zone": "../../etc/passwd" }), true)).contains("isn't a known"));
    fake.state.lock().unwrap().tz_needs_admin = true;
    assert!(failed(call(&ctx, "set_time_zone", &json!({ "time_zone": "Europe/Paris" }), true)).contains("sudo timedatectl"));
    fake.state.lock().unwrap().tz_needs_admin = false;
    let o = done(call(&ctx, "set_time_zone", &json!({ "time_zone": "Europe/Paris" }), true));
    assert_eq!(o.undo.unwrap().args, json!({ "time_zone": "Europe/London" }));
}

#[test]
fn about_this_computer() {
    let (ctx, _) = setup("about");
    let o = done(call(&ctx, "about_this_computer", &json!({}), false));
    assert_eq!(o.state["os"], "Arch Linux");
    assert_eq!(o.state["name"], "laptop");
    assert_eq!(o.state["processor"], "Intel(R) Core(TM) Ultra 7 155H");
    assert_eq!(o.state["threads"], 2);
    assert_eq!(o.state["memory"], "31.1 GB");
    assert_eq!(o.state["disk"]["total"], "932 GB");
    assert_eq!(o.state["uptime"], "1 day 2 h");
}
