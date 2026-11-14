//! Battery, Wi-Fi, Bluetooth, sound, brightness, media and session actions.
//!
//! [`System`] is the small interface to the machine; `RealSystem` talks to
//! sysfs, NetworkManager/UPower/MPRIS over DBus and `nmcli`, `bluetoothctl`,
//! `wpctl`, `brightnessctl`, and `FakeSystem` keeps everything in memory for
//! tests and screenshots (`Y7_SHELL_FAKE=1`).
//!
//! A worker thread owns the `System`: the UI sends it requests and reads the
//! latest [`Status`]. Changes are event-driven where the system tells us
//! (DBus signals, `pactl subscribe`); the rest is polled slowly.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wifi {
    /// There's a Wi-Fi device at all.
    pub available: bool,
    pub enabled: bool,
    pub ssid: Option<String>,
    /// 0–100.
    pub strength: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Volume {
    /// 0–1.
    pub level: f32,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// The MPRIS bus name, for controls.
    pub player: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub battery: Option<Battery>,
    pub wifi: Wifi,
    /// `None` without an adapter.
    pub bluetooth: Option<bool>,
    pub volume: Option<Volume>,
    pub brightness: Option<f32>,
    pub media: Option<NowPlaying>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Battery,
    Wifi,
    Bluetooth,
    Volume,
    Brightness,
    Media,
}

pub const ALL_PARTS: [Part; 6] = [Part::Battery, Part::Wifi, Part::Bluetooth, Part::Volume, Part::Brightness, Part::Media];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaAction {
    PlayPause,
    Next,
    Previous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionAction {
    Lock,
    Sleep,
    LogOut,
    Restart,
    ShutDown,
}

pub trait System: Send + Sync {
    fn battery(&self) -> Option<Battery>;
    fn wifi(&self) -> Wifi;
    fn set_wifi(&self, on: bool);
    fn bluetooth(&self) -> Option<bool>;
    fn set_bluetooth(&self, on: bool);
    fn volume(&self) -> Option<Volume>;
    fn set_volume(&self, level: f32);
    fn brightness(&self) -> Option<f32>;
    fn set_brightness(&self, level: f32);
    fn media(&self) -> Option<NowPlaying>;
    fn media_control(&self, player: &str, action: MediaAction);
    fn set_dark(&self, dark: bool);
    fn session(&self, action: SessionAction);
    /// Starts watching for changes; `changed` is called (from any thread)
    /// when a part may have changed.
    fn watch(&self, changed: Arc<dyn Fn(Part) + Send + Sync>);
}

impl Status {
    fn read(&mut self, sys: &dyn System, part: Part) {
        match part {
            Part::Battery => self.battery = sys.battery(),
            Part::Wifi => self.wifi = sys.wifi(),
            Part::Bluetooth => self.bluetooth = sys.bluetooth(),
            Part::Volume => self.volume = sys.volume(),
            Part::Brightness => self.brightness = sys.brightness(),
            Part::Media => self.media = sys.media(),
        }
    }
}

// -- worker -------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Refresh(Part),
    SetWifi(bool),
    SetBluetooth(bool),
    SetVolume(f32),
    SetBrightness(f32),
    Media(String, MediaAction),
    SetDark(bool),
    Session(SessionAction),
}

/// The UI's handle on the worker.
#[derive(Clone)]
pub struct Services {
    tx: Sender<Request>,
    pub status: Arc<Mutex<Status>>,
}

impl Services {
    pub fn start(sys: Arc<dyn System>, wake: Arc<dyn Fn() + Send + Sync>) -> Services {
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let s = status.clone();
        let watch_tx = tx.clone();
        sys.watch(Arc::new(move |part| {
            let _ = watch_tx.send(Request::Refresh(part));
        }));
        for part in ALL_PARTS {
            let _ = tx.send(Request::Refresh(part));
        }
        std::thread::Builder::new()
            .name("system".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    // Batch whatever piled up (slider drags, signal bursts).
                    std::thread::sleep(Duration::from_millis(30));
                    let mut batch = vec![first];
                    batch.extend(rx.try_iter());
                    let mut status = s.lock().unwrap().clone();
                    handle(sys.as_ref(), batch, &mut status);
                    let changed = *s.lock().unwrap() != status;
                    *s.lock().unwrap() = status;
                    if changed {
                        wake();
                    }
                }
            })
            .expect("system thread");
        Services { tx, status }
    }

    pub fn send(&self, r: Request) {
        let _ = self.tx.send(r);
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Updates the cached status right away, so the UI doesn't wait for the
    /// system to catch up (toggles, sliders).
    pub fn optimistic(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.status.lock().unwrap());
    }
}

/// Applies a batch of requests, then re-reads what they touched.
pub fn handle(sys: &dyn System, batch: Vec<Request>, status: &mut Status) {
    let mut refresh: Vec<Part> = Vec::new();
    let mut add = |p: Part| {
        if !refresh.contains(&p) {
            refresh.push(p);
        }
    };
    // Only the last of each setter matters.
    let last_volume = batch.iter().rposition(|r| matches!(r, Request::SetVolume(_)));
    let last_brightness = batch.iter().rposition(|r| matches!(r, Request::SetBrightness(_)));
    for (i, r) in batch.into_iter().enumerate() {
        match r {
            Request::Refresh(p) => add(p),
            Request::SetWifi(on) => {
                sys.set_wifi(on);
                add(Part::Wifi);
            }
            Request::SetBluetooth(on) => {
                sys.set_bluetooth(on);
                add(Part::Bluetooth);
            }
            Request::SetVolume(v) if Some(i) == last_volume => {
                sys.set_volume(v.clamp(0.0, 1.0));
                add(Part::Volume);
            }
            Request::SetBrightness(v) if Some(i) == last_brightness => {
                sys.set_brightness(v.clamp(0.01, 1.0));
                add(Part::Brightness);
            }
            Request::Media(player, a) => {
                sys.media_control(&player, a);
                add(Part::Media);
            }
            Request::SetDark(d) => sys.set_dark(d),
            Request::Session(a) => sys.session(a),
            _ => {}
        }
    }
    for p in refresh {
        status.read(sys, p);
    }
}

// -- fake -----------------------------------------------------------------------------

/// Everything in memory: tests, screenshots, and never touching the real machine.
pub struct FakeSystem {
    pub state: Mutex<Status>,
    pub log: Mutex<Vec<String>>,
}

impl FakeSystem {
    pub fn demo() -> FakeSystem {
        FakeSystem {
            state: Mutex::new(Status {
                battery: Some(Battery { percent: 82, charging: false }),
                wifi: Wifi { available: true, enabled: true, ssid: Some("Home".into()), strength: 80 },
                bluetooth: Some(true),
                volume: Some(Volume { level: 0.6, muted: false }),
                brightness: Some(0.7),
                media: Some(NowPlaying {
                    title: "Holocene".into(),
                    artist: "Bon Iver".into(),
                    playing: true,
                    player: "org.mpris.MediaPlayer2.demo".into(),
                }),
            }),
            log: Mutex::new(Vec::new()),
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Status) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    fn note(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }
}

impl System for FakeSystem {
    fn battery(&self) -> Option<Battery> {
        self.with(|s| s.battery)
    }
    fn wifi(&self) -> Wifi {
        self.with(|s| s.wifi.clone())
    }
    fn set_wifi(&self, on: bool) {
        self.with(|s| {
            s.wifi.enabled = on;
            s.wifi.ssid = on.then(|| "Home".into());
        });
    }
    fn bluetooth(&self) -> Option<bool> {
        self.with(|s| s.bluetooth)
    }
    fn set_bluetooth(&self, on: bool) {
        self.with(|s| s.bluetooth = Some(on));
    }
    fn volume(&self) -> Option<Volume> {
        self.with(|s| s.volume)
    }
    fn set_volume(&self, level: f32) {
        self.with(|s| s.volume = Some(Volume { level, muted: false }));
    }
    fn brightness(&self) -> Option<f32> {
        self.with(|s| s.brightness)
    }
    fn set_brightness(&self, level: f32) {
        self.with(|s| s.brightness = Some(level));
    }
    fn media(&self) -> Option<NowPlaying> {
        self.with(|s| s.media.clone())
    }
    fn media_control(&self, _player: &str, action: MediaAction) {
        self.with(|s| {
            if let (Some(m), MediaAction::PlayPause) = (s.media.as_mut(), action) {
                m.playing = !m.playing;
            }
        });
    }
    fn set_dark(&self, dark: bool) {
        self.note(format!("dark {dark}"));
    }
    fn session(&self, action: SessionAction) {
        self.note(format!("session {action:?}"));
        eprintln!("y7-shell (fake system): would {action:?}");
    }
    fn watch(&self, _changed: Arc<dyn Fn(Part) + Send + Sync>) {}
}

// -- real -----------------------------------------------------------------------------

pub struct RealSystem;

/// Runs a program and returns its stdout, or `None` if it's missing or failed.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run(program: &str, args: &[&str]) {
    let status = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    if let Err(e) = status {
        eprintln!("y7-shell: couldn't run {program}: {e}");
    }
}

fn has(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// The first laptop battery under /sys/class/power_supply.
fn sysfs_battery(root: &Path) -> Option<Battery> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).collect();
    dirs.sort();
    let bat = dirs.iter().find(|d| {
        read_trim(&d.join("type")).as_deref() == Some("Battery") && read_trim(&d.join("scope")).as_deref() != Some("Device")
    })?;
    let percent = read_trim(&bat.join("capacity"))?.parse::<u8>().ok()?.min(100);
    let status = read_trim(&bat.join("status")).unwrap_or_default();
    let plugged = dirs.iter().any(|d| {
        read_trim(&d.join("type")).as_deref() == Some("Mains") && read_trim(&d.join("online")).as_deref() == Some("1")
    });
    let charging = status == "Charging" || (plugged && status != "Discharging");
    Some(Battery { percent, charging })
}

/// Parses `wpctl get-volume`: "Volume: 0.45 [MUTED]".
pub fn parse_wpctl(s: &str) -> Option<Volume> {
    let rest = s.trim().strip_prefix("Volume:")?.trim();
    let level = rest.split_whitespace().next()?.parse::<f32>().ok()?;
    Some(Volume { level: level.min(1.0), muted: rest.contains("[MUTED]") })
}

/// Parses `nmcli -t -f ACTIVE,SSID,SIGNAL dev wifi`: the active network.
pub fn parse_nmcli_wifi(s: &str) -> Option<(String, u8)> {
    s.lines().find_map(|l| {
        let rest = l.strip_prefix("yes:")?;
        let (ssid, signal) = rest.rsplit_once(':')?;
        Some((ssid.replace("\\:", ":"), signal.parse().unwrap_or(0)))
    })
}

fn backlight() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter().next()
}

fn mpris_players(conn: &zbus::blocking::Connection) -> Vec<String> {
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(conn) else { return Vec::new() };
    let Ok(names) = dbus.list_names() else { return Vec::new() };
    names.into_iter().map(|n| n.to_string()).filter(|n| n.starts_with("org.mpris.MediaPlayer2.")).collect()
}

fn mpris_get(conn: &zbus::blocking::Connection, player: &str) -> Option<NowPlaying> {
    let props = zbus::blocking::fdo::PropertiesProxy::builder(conn)
        .destination(player.to_string())
        .ok()?
        .path("/org/mpris/MediaPlayer2")
        .ok()?
        .build()
        .ok()?;
    let iface = zbus::names::InterfaceName::try_from("org.mpris.MediaPlayer2.Player").ok()?;
    let status: String = props.get(iface.clone(), "PlaybackStatus").ok()?.try_into().ok()?;
    let meta: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
        props.get(iface, "Metadata").ok()?.try_into().ok()?;
    let title: String = meta.get("xesam:title").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok()).unwrap_or_default();
    let artist: Vec<String> =
        meta.get("xesam:artist").and_then(|v| v.try_clone().ok()).and_then(|v| v.try_into().ok()).unwrap_or_default();
    if title.is_empty() || status == "Stopped" {
        return None;
    }
    Some(NowPlaying { title, artist: artist.join(", "), playing: status == "Playing", player: player.to_string() })
}

/// Calls `f` for every signal matching `rule` (on its own thread).
fn on_signals(conn: zbus::blocking::Connection, rule: zbus::MatchRule<'static>, f: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let Ok(iter) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64)) else { return };
        for _ in iter {
            f();
        }
    });
}

/// Signals named `member` from the object at `path` (only that object: the
/// ones below it, like access points and devices, chatter constantly).
fn signal_rule(path: &'static str, member: &'static str) -> Option<zbus::MatchRule<'static>> {
    let b = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).member(member).ok()?;
    Some(b.path(path).ok()?.build())
}

/// Polls `part` every `every` (for things with no change signal).
fn poll(changed: Arc<dyn Fn(Part) + Send + Sync>, part: Part, every: Duration) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(every);
            changed(part);
        }
    });
}

impl System for RealSystem {
    fn battery(&self) -> Option<Battery> {
        sysfs_battery(Path::new("/sys/class/power_supply"))
    }

    fn wifi(&self) -> Wifi {
        let Some(devices) = output("nmcli", &["-t", "-f", "TYPE", "device"]) else { return Wifi::default() };
        if !devices.lines().any(|l| l == "wifi") {
            return Wifi::default();
        }
        let enabled = output("nmcli", &["-t", "-f", "WIFI", "general"]).is_some_and(|s| s.trim() == "enabled");
        let active = enabled
            .then(|| output("nmcli", &["-t", "-f", "ACTIVE,SSID,SIGNAL", "device", "wifi", "list", "--rescan", "no"]))
            .flatten()
            .and_then(|s| parse_nmcli_wifi(&s));
        Wifi { available: true, enabled, strength: active.as_ref().map_or(0, |a| a.1), ssid: active.map(|a| a.0) }
    }

    fn set_wifi(&self, on: bool) {
        run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
    }

    fn bluetooth(&self) -> Option<bool> {
        let out = output("bluetoothctl", &["show"])?;
        let powered = out.lines().find_map(|l| l.trim().strip_prefix("Powered:"))?;
        Some(powered.trim() == "yes")
    }

    fn set_bluetooth(&self, on: bool) {
        run("bluetoothctl", &["power", if on { "on" } else { "off" }]);
    }

    fn volume(&self) -> Option<Volume> {
        parse_wpctl(&output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?)
    }

    fn set_volume(&self, level: f32) {
        run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{level:.3}")]);
        run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
    }

    fn brightness(&self) -> Option<f32> {
        let dir = backlight()?;
        let cur: f32 = read_trim(&dir.join("brightness"))?.parse().ok()?;
        let max: f32 = read_trim(&dir.join("max_brightness"))?.parse().ok()?;
        (max > 0.0).then(|| cur / max)
    }

    fn set_brightness(&self, level: f32) {
        if has("brightnessctl") {
            run("brightnessctl", &["--quiet", "set", &format!("{}%", (level * 100.0).round())]);
            return;
        }
        // logind lets the session's user set the backlight without root.
        let Some(dir) = backlight() else { return };
        let Some(max) = read_trim(&dir.join("max_brightness")).and_then(|s| s.parse::<f32>().ok()) else { return };
        let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if let Ok(conn) = zbus::blocking::Connection::system() {
            let _ = conn.call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1/session/auto",
                Some("org.freedesktop.login1.Session"),
                "SetBrightness",
                &("backlight", name, (level * max).round() as u32),
            );
        }
    }

    fn media(&self) -> Option<NowPlaying> {
        let conn = zbus::blocking::Connection::session().ok()?;
        let all: Vec<NowPlaying> = mpris_players(&conn).iter().filter_map(|p| mpris_get(&conn, p)).collect();
        all.iter().find(|m| m.playing).or(all.first()).cloned()
    }

    fn media_control(&self, player: &str, action: MediaAction) {
        let method = match action {
            MediaAction::PlayPause => "PlayPause",
            MediaAction::Next => "Next",
            MediaAction::Previous => "Previous",
        };
        if let Ok(conn) = zbus::blocking::Connection::session() {
            let _ = conn.call_method(Some(player), "/org/mpris/MediaPlayer2", Some("org.mpris.MediaPlayer2.Player"), method, &());
        }
    }

    fn set_dark(&self, dark: bool) {
        // Plasma's tool also updates GTK apps; otherwise edit kdeglobals,
        // which our apps and the desktop read.
        if has("plasma-apply-colorscheme") {
            run("plasma-apply-colorscheme", &[if dark { "BreezeDark" } else { "BreezeLight" }]);
            return;
        }
        let Some(path) = dirs::config_dir().map(|d| d.join("kdeglobals")) else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let scheme = format!("ColorScheme={}", if dark { "BreezeDark" } else { "BreezeLight" });
        let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with("ColorScheme=")).map(String::from).collect();
        match lines.iter().position(|l| l == "[General]") {
            Some(i) => lines.insert(i + 1, scheme),
            None => lines.splice(0..0, ["[General]".to_string(), scheme, String::new()]).for_each(drop),
        }
        let _ = std::fs::write(path, lines.join("\n") + "\n");
    }

    fn session(&self, action: SessionAction) {
        match action {
            SessionAction::Lock => run("loginctl", &["lock-session"]),
            SessionAction::Sleep => run("systemctl", &["suspend"]),
            SessionAction::LogOut => match std::env::var("XDG_SESSION_ID") {
                Ok(id) => run("loginctl", &["terminate-session", &id]),
                Err(_) => run("loginctl", &["terminate-user", &std::env::var("USER").unwrap_or_default()]),
            },
            SessionAction::Restart => run("systemctl", &["reboot"]),
            SessionAction::ShutDown => run("systemctl", &["poweroff"]),
        }
    }

    fn watch(&self, changed: Arc<dyn Fn(Part) + Send + Sync>) {
        let system = zbus::blocking::Connection::system().ok();
        let session = zbus::blocking::Connection::session().ok();
        let mut upower = false;
        if let Some(conn) = &system {
            for (path, part) in [
                ("/org/freedesktop/UPower/devices/DisplayDevice", Part::Battery),
                ("/org/freedesktop/NetworkManager", Part::Wifi),
                ("/org/bluez/hci0", Part::Bluetooth),
            ] {
                if let Some(rule) = signal_rule(path, "PropertiesChanged") {
                    let c = changed.clone();
                    on_signals(conn.clone(), rule, move || c(part));
                }
            }
            upower = zbus::blocking::fdo::DBusProxy::new(conn)
                .ok()
                .and_then(|d| d.name_has_owner("org.freedesktop.UPower".try_into().ok()?).ok())
                .unwrap_or(false);
        }
        if !upower && self.battery().is_some() {
            poll(changed.clone(), Part::Battery, Duration::from_secs(60));
        }
        // Signal strength drifts without NetworkManager saying so.
        if self.wifi().available {
            poll(changed.clone(), Part::Wifi, Duration::from_secs(60));
        }

        if let Some(conn) = &session {
            let c = changed.clone();
            if let Some(rule) = signal_rule("/org/mpris/MediaPlayer2", "PropertiesChanged") {
                on_signals(conn.clone(), rule, move || c(Part::Media));
            }
            // Players appearing and quitting.
            let c = changed.clone();
            let rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .member("NameOwnerChanged")
                .and_then(|b| b.arg0ns("org.mpris.MediaPlayer2"))
                .map(|b| b.build());
            if let Ok(rule) = rule {
                on_signals(conn.clone(), rule, move || c(Part::Media));
            }
        }

        // PipeWire (through its PulseAudio server) reports sink changes.
        let c = changed.clone();
        let subscribed = Command::new("pactl")
            .arg("subscribe")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()
            .and_then(|mut child| child.stdout.take());
        match subscribed {
            Some(out) => {
                std::thread::spawn(move || {
                    use std::io::BufRead;
                    for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                        // "sink #", not "sink-input #": streams come and go all the time.
                        if line.contains("on sink #") || line.contains("on server") {
                            c(Part::Volume);
                        }
                    }
                });
            }
            None if has("wpctl") => poll(c, Part::Volume, Duration::from_secs(30)),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_applies_and_refreshes() {
        let sys = FakeSystem::demo();
        let mut status = Status::default();
        handle(&sys, ALL_PARTS.iter().map(|&p| Request::Refresh(p)).collect(), &mut status);
        assert_eq!(status.wifi.ssid.as_deref(), Some("Home"));
        assert_eq!(status.battery, Some(Battery { percent: 82, charging: false }));

        // Slider drags collapse to the last value.
        handle(&sys, vec![Request::SetVolume(0.2), Request::SetWifi(false), Request::SetVolume(1.4)], &mut status);
        assert_eq!(status.volume, Some(Volume { level: 1.0, muted: false }));
        assert!(!status.wifi.enabled);
        assert_eq!(status.wifi.ssid, None);

        handle(&sys, vec![Request::Media("p".into(), MediaAction::PlayPause)], &mut status);
        assert!(!status.media.as_ref().unwrap().playing);
        handle(&sys, vec![Request::Session(SessionAction::Lock), Request::SetDark(true)], &mut status);
        assert_eq!(*sys.log.lock().unwrap(), ["session Lock", "dark true"]);
    }

    #[test]
    fn services_thread_wakes_ui() {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let services = Services::start(Arc::new(FakeSystem::demo()), Arc::new(move || {
            let _ = tx.lock().unwrap().send(());
        }));
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(services.status().bluetooth, Some(true));
        services.send(Request::SetBluetooth(false));
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(services.status().bluetooth, Some(false));
    }

    #[test]
    fn parsers() {
        assert_eq!(parse_wpctl("Volume: 0.45\n"), Some(Volume { level: 0.45, muted: false }));
        assert_eq!(parse_wpctl("Volume: 0.30 [MUTED]"), Some(Volume { level: 0.3, muted: true }));
        assert_eq!(parse_wpctl("nope"), None);
        assert_eq!(parse_nmcli_wifi("no:Other:40\nyes:Home\\:5G:72\n"), Some(("Home:5G".into(), 72)));
        assert_eq!(parse_nmcli_wifi("no:Other:40\n"), None);

        let root = std::env::temp_dir().join(format!("y7-power-test-{}", std::process::id()));
        for (dir, files) in [
            ("AC", vec![("type", "Mains"), ("online", "1")]),
            ("BAT0", vec![("type", "Battery"), ("capacity", "57"), ("status", "Not charging")]),
            ("hid-mouse", vec![("type", "Battery"), ("scope", "Device"), ("capacity", "10")]),
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            for (f, v) in files {
                std::fs::write(root.join(dir).join(f), format!("{v}\n")).unwrap();
            }
        }
        assert_eq!(sysfs_battery(&root), Some(Battery { percent: 57, charging: true }));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
