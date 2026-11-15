//! Battery, Wi-Fi, Bluetooth, sound, brightness, media and session actions.
//!
//! [`System`] is the small interface to the machine; `RealSystem` talks to
//! sysfs, NetworkManager/UPower/MPRIS over DBus and `nmcli`, `bluetoothctl`,
//! `wpctl`, `brightnessctl`, and `FakeSystem` keeps everything in memory for
//! tests and screenshots (`Y7_SHELL_FAKE=1`).
//!
//! A worker thread owns the `System`: the UI sends it requests and reads the
//! latest [`Status`]. Changes are event-driven where the system tells us
//! (DBus signals, `pactl subscribe`); the rest is polled slowly. Slow jobs
//! (scanning, joining a network, connecting a device) run on threads of
//! their own and report back to the worker, so sliders never wait on them.

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

/// A Wi-Fi network in range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Network {
    pub ssid: String,
    /// 0–100.
    pub strength: u8,
    /// NetworkManager's summary ("WPA2 WPA3", "WPA2 802.1X"); empty when open.
    pub security: String,
    /// There's a saved profile, so joining needs no password.
    pub known: bool,
    pub active: bool,
}

impl Network {
    pub fn secured(&self) -> bool {
        !self.security.is_empty()
    }

    /// Enterprise (802.1X) networks need more than a password: they're set
    /// up in Settings.
    pub fn enterprise(&self) -> bool {
        self.security.contains("802.1X")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceKind {
    Headphones,
    Speaker,
    Mouse,
    Keyboard,
    Phone,
    Computer,
    Gamepad,
    #[default]
    Other,
}

impl DeviceKind {
    /// From BlueZ's `Icon` property (freedesktop icon names).
    pub fn from_icon(icon: &str) -> DeviceKind {
        match icon {
            i if i.starts_with("audio-head") => DeviceKind::Headphones,
            i if i.starts_with("audio") => DeviceKind::Speaker,
            "input-mouse" | "input-tablet" => DeviceKind::Mouse,
            "input-keyboard" => DeviceKind::Keyboard,
            "input-gaming" => DeviceKind::Gamepad,
            "phone" => DeviceKind::Phone,
            "computer" => DeviceKind::Computer,
            _ => DeviceKind::Other,
        }
    }
}

/// A paired Bluetooth device.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtDevice {
    /// BlueZ's object path, which names it for connecting.
    pub path: String,
    pub name: String,
    pub kind: DeviceKind,
    pub connected: bool,
    /// Percent, when the device reports it.
    pub battery: Option<u8>,
}

/// A sound output (a PipeWire/PulseAudio sink).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    /// The sink's name, for switching to it.
    pub name: String,
    pub description: String,
    pub default: bool,
    pub kind: DeviceKind,
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
    /// Networks in range (read with `wifi`), the connected one first.
    pub networks: Vec<Network>,
    /// Paired Bluetooth devices (read with `bluetooth`).
    pub devices: Vec<BtDevice>,
    pub outputs: Vec<Output>,
    /// A scan for networks is running.
    pub scanning: bool,
    /// The network being joined.
    pub joining: Option<String>,
    /// The last join that failed: the network and why.
    pub join_error: Option<(String, String)>,
    /// The device being connected or disconnected.
    pub device_busy: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Battery,
    Wifi,
    Bluetooth,
    Volume,
    Brightness,
    Media,
    Outputs,
}

pub const ALL_PARTS: [Part; 7] =
    [Part::Battery, Part::Wifi, Part::Bluetooth, Part::Volume, Part::Brightness, Part::Media, Part::Outputs];

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
    /// Networks in range, from the last scan.
    fn networks(&self) -> Vec<Network>;
    /// Scans for networks; returns when the results are in (seconds).
    fn scan_wifi(&self);
    /// Joins a network, with a password for a secured one that isn't saved;
    /// returns when it's up or has failed.
    fn join_wifi(&self, network: &Network, password: Option<&str>) -> Result<(), String>;
    fn bluetooth(&self) -> Option<bool>;
    fn set_bluetooth(&self, on: bool);
    fn bt_devices(&self) -> Vec<BtDevice>;
    /// Connects or disconnects a paired device; returns when it's done.
    fn connect_bt(&self, path: &str, connect: bool) -> Result<(), String>;
    fn volume(&self) -> Option<Volume>;
    fn set_volume(&self, level: f32);
    fn outputs(&self) -> Vec<Output>;
    fn set_output(&self, name: &str);
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
            Part::Wifi => {
                self.wifi = sys.wifi();
                self.networks = if self.wifi.enabled { sys.networks() } else { Vec::new() };
            }
            Part::Bluetooth => {
                self.bluetooth = sys.bluetooth();
                self.devices = if self.bluetooth == Some(true) { sys.bt_devices() } else { Vec::new() };
            }
            Part::Outputs => self.outputs = sys.outputs(),
            Part::Volume => self.volume = sys.volume(),
            Part::Brightness => self.brightness = sys.brightness(),
            Part::Media => self.media = sys.media(),
        }
    }
}

// -- worker -------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
pub enum Request {
    Refresh(Part),
    SetWifi(bool),
    SetBluetooth(bool),
    SetVolume(f32),
    SetBrightness(f32),
    Media(String, MediaAction),
    SetDark(bool),
    Session(SessionAction),
    ScanWifi,
    /// A network and the password typed for it, if one was asked for.
    JoinWifi(Network, Option<String>),
    /// A device's object path; connect (true) or disconnect.
    ConnectBt(String, bool),
    SetOutput(String),
    /// From the slow jobs' own threads: they're done.
    Scanned,
    Joined(String, Result<(), String>),
    BtDone,
}

// Not derived: it would print passwords.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Request::Refresh(p) => write!(f, "Refresh({p:?})"),
            Request::SetWifi(on) => write!(f, "SetWifi({on})"),
            Request::SetBluetooth(on) => write!(f, "SetBluetooth({on})"),
            Request::SetVolume(v) => write!(f, "SetVolume({v})"),
            Request::SetBrightness(v) => write!(f, "SetBrightness({v})"),
            Request::Media(p, a) => write!(f, "Media({p:?}, {a:?})"),
            Request::SetDark(d) => write!(f, "SetDark({d})"),
            Request::Session(a) => write!(f, "Session({a:?})"),
            Request::ScanWifi => write!(f, "ScanWifi"),
            Request::JoinWifi(n, _) => write!(f, "JoinWifi({:?})", n.ssid),
            Request::ConnectBt(p, on) => write!(f, "ConnectBt({p:?}, {on})"),
            Request::SetOutput(o) => write!(f, "SetOutput({o:?})"),
            Request::Scanned => write!(f, "Scanned"),
            Request::Joined(ssid, r) => write!(f, "Joined({ssid:?}, {r:?})"),
            Request::BtDone => write!(f, "BtDone"),
        }
    }
}

/// A request too slow for the worker thread.
#[derive(Clone, PartialEq)]
pub enum Job {
    Scan,
    Join(Network, Option<String>),
    Connect(String, bool),
}

/// Runs a slow job and returns what to tell the worker.
pub fn run_job(sys: &dyn System, job: Job) -> Request {
    match job {
        Job::Scan => {
            sys.scan_wifi();
            Request::Scanned
        }
        Job::Join(n, password) => {
            let r = sys.join_wifi(&n, password.as_deref());
            Request::Joined(n.ssid, r)
        }
        Job::Connect(path, on) => {
            if let Err(e) = sys.connect_bt(&path, on) {
                eprintln!("y7-shell: couldn't {} {path}: {e}", if on { "connect" } else { "disconnect" });
            }
            Request::BtDone
        }
    }
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
        let job_tx = tx.clone();
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
                    for job in handle(sys.as_ref(), batch, &mut status) {
                        let (sys, tx) = (sys.clone(), job_tx.clone());
                        std::thread::spawn(move || {
                            let _ = tx.send(run_job(sys.as_ref(), job));
                        });
                    }
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

/// Applies a batch of requests, then re-reads what they touched. Returns
/// the slow jobs to start.
pub fn handle(sys: &dyn System, batch: Vec<Request>, status: &mut Status) -> Vec<Job> {
    let mut jobs = Vec::new();
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
            Request::ScanWifi if !status.scanning => {
                status.scanning = true;
                jobs.push(Job::Scan);
            }
            Request::JoinWifi(n, password) if status.joining.is_none() => {
                status.joining = Some(n.ssid.clone());
                status.join_error = None;
                jobs.push(Job::Join(n, password));
            }
            Request::ConnectBt(path, on) if status.device_busy.is_none() => {
                status.device_busy = Some(path.clone());
                jobs.push(Job::Connect(path, on));
            }
            Request::SetOutput(name) => {
                sys.set_output(&name);
                add(Part::Outputs);
                add(Part::Volume);
            }
            Request::Scanned => {
                status.scanning = false;
                add(Part::Wifi);
            }
            Request::Joined(ssid, r) => {
                status.joining = None;
                status.join_error = r.err().map(|e| (ssid, e));
                add(Part::Wifi);
            }
            Request::BtDone => {
                status.device_busy = None;
                add(Part::Bluetooth);
            }
            _ => {}
        }
    }
    for p in refresh {
        status.read(sys, p);
    }
    jobs
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
                networks: vec![
                    net("Home", 80, "WPA2 WPA3", true, true),
                    net("Studio", 64, "WPA2", true, false),
                    net("Bakery Guest", 58, "", false, false),
                    net("Rivers-5G", 46, "WPA2 WPA3", false, false),
                    net("UNI-Staff", 30, "WPA2 802.1X", false, false),
                    net("DIRECT-4F-Printer", 18, "WPA2", false, false),
                ],
                devices: vec![
                    dev("AirPods Pro", DeviceKind::Headphones, true, Some(82)),
                    dev("MX Master 3S", DeviceKind::Mouse, true, Some(55)),
                    dev("Magic Keyboard", DeviceKind::Keyboard, false, None),
                    dev("Pixel 8", DeviceKind::Phone, false, None),
                ],
                outputs: vec![
                    Output { name: "speakers".into(), description: "Built-in Speakers".into(), default: true, kind: DeviceKind::Speaker },
                    Output { name: "airpods".into(), description: "AirPods Pro".into(), default: false, kind: DeviceKind::Headphones },
                    Output { name: "hdmi".into(), description: "LG UltraFine (HDMI)".into(), default: false, kind: DeviceKind::Speaker },
                ],
                ..Default::default()
            }),
            log: Mutex::new(Vec::new()),
        }
    }

    /// A fake's password for every secured network it doesn't know.
    pub const PASSWORD: &str = "password";

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
    fn networks(&self) -> Vec<Network> {
        self.with(|s| s.networks.clone())
    }
    fn scan_wifi(&self) {}
    fn join_wifi(&self, network: &Network, password: Option<&str>) -> Result<(), String> {
        self.note(format!("join {}", network.ssid));
        let ok = !network.secured() || network.known || password == Some(Self::PASSWORD);
        if !ok {
            return Err("The password is incorrect.".into());
        }
        self.with(|s| {
            for n in &mut s.networks {
                n.active = n.ssid == network.ssid;
                n.known |= n.active;
            }
            s.wifi.ssid = Some(network.ssid.clone());
            s.wifi.strength = network.strength;
        });
        Ok(())
    }
    fn bluetooth(&self) -> Option<bool> {
        self.with(|s| s.bluetooth)
    }
    fn set_bluetooth(&self, on: bool) {
        self.with(|s| s.bluetooth = Some(on));
    }
    fn bt_devices(&self) -> Vec<BtDevice> {
        self.with(|s| s.devices.clone())
    }
    fn connect_bt(&self, path: &str, connect: bool) -> Result<(), String> {
        self.with(|s| s.devices.iter_mut().filter(|d| d.path == path).for_each(|d| d.connected = connect));
        Ok(())
    }
    fn volume(&self) -> Option<Volume> {
        self.with(|s| s.volume)
    }
    fn set_volume(&self, level: f32) {
        self.with(|s| s.volume = Some(Volume { level, muted: false }));
    }
    fn outputs(&self) -> Vec<Output> {
        self.with(|s| s.outputs.clone())
    }
    fn set_output(&self, name: &str) {
        self.with(|s| s.outputs.iter_mut().for_each(|o| o.default = o.name == name));
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

fn net(ssid: &str, strength: u8, security: &str, known: bool, active: bool) -> Network {
    Network { ssid: ssid.into(), strength, security: security.into(), known, active }
}

fn dev(name: &str, kind: DeviceKind, connected: bool, battery: Option<u8>) -> BtDevice {
    let path = format!("/org/bluez/hci0/dev_{}", name.replace(' ', "_"));
    BtDevice { path, name: name.into(), kind, connected, battery }
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

/// Runs a program and returns why it failed: the gist of its stderr.
fn run_checked(program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("couldn't run {program}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").trim();
    Err(line.trim_start_matches("Error: ").to_string())
}

/// Like `output`, in the C locale (for tools whose labels get translated).
fn output_c(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub(crate) fn has(program: &str) -> bool {
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

/// Splits a line of `nmcli -t` output: fields are separated by ':', and
/// ':' and '\' inside them are escaped with '\'.
pub fn nmcli_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => fields.last_mut().unwrap().extend(chars.next()),
            ':' => fields.push(String::new()),
            c => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// Parses `nmcli -t -f IN-USE,SSID,SECURITY,SIGNAL device wifi list`: one
/// network per name (its strongest access point; hidden ones left out),
/// the connected one first, then saved ones, then by signal.
pub fn parse_networks(list: &str, known: &[String]) -> Vec<Network> {
    let mut out: Vec<Network> = Vec::new();
    for line in list.lines() {
        let fields = nmcli_fields(line);
        let [in_use, ssid, security, signal] = fields.as_slice() else { continue };
        if ssid.is_empty() {
            continue;
        }
        let security = if security.trim() == "--" { "" } else { security.trim() };
        let n = Network {
            ssid: ssid.clone(),
            strength: signal.trim().parse().unwrap_or(0),
            security: security.to_string(),
            known: known.contains(ssid),
            active: in_use.trim() == "*",
        };
        match out.iter_mut().find(|o| o.ssid == n.ssid) {
            Some(o) => {
                o.active |= n.active;
                if n.strength > o.strength {
                    o.strength = n.strength;
                    o.security = n.security;
                }
            }
            None => out.push(n),
        }
    }
    out.sort_by(|a, b| b.active.cmp(&a.active).then(b.known.cmp(&a.known)).then(b.strength.cmp(&a.strength)));
    out
}

/// Saved Wi-Fi profiles' names, from `nmcli -t -f NAME,TYPE connection show`.
pub fn parse_known(list: &str) -> Vec<String> {
    list.lines()
        .filter_map(|l| match nmcli_fields(l).as_slice() {
            [name, kind] if kind == "802-11-wireless" => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Parses `pactl list sinks` (C locale).
pub fn parse_sinks(list: &str, default: &str) -> Vec<Output> {
    let mut out: Vec<Output> = Vec::new();
    for line in list.lines() {
        let l = line.trim();
        if l.starts_with("Sink #") {
            out.push(Output { kind: DeviceKind::Speaker, ..Default::default() });
            continue;
        }
        let Some(o) = out.last_mut() else { continue };
        if let Some(v) = l.strip_prefix("Name:") {
            o.name = v.trim().to_string();
            o.default = o.name == default;
        } else if let Some(v) = l.strip_prefix("Description:") {
            o.description = v.trim().to_string();
        } else if (l.starts_with("Active Port:") || l.starts_with("device.icon_name")) && (l.contains("head")) {
            o.kind = DeviceKind::Headphones;
        }
    }
    out.retain(|o| !o.name.is_empty());
    for o in &mut out {
        if o.description.is_empty() {
            o.description = o.name.clone();
        }
    }
    out
}

/// The Wi-Fi interface's name, like "wlp0s20f3".
fn wifi_iface() -> Option<String> {
    let list = output("nmcli", &["-t", "-f", "DEVICE,TYPE", "device"])?;
    list.lines().find_map(|l| match nmcli_fields(l).as_slice() {
        [dev, kind] if kind == "wifi" => Some(dev.clone()),
        _ => None,
    })
}

const NM: &str = "org.freedesktop.NetworkManager";

/// Joins a new secured network over DBus (so the password never shows up
/// in a process list), waits for it to come up, and forgets it if it
/// doesn't (so a mistyped password isn't saved).
fn nm_join(network: &Network, password: &str) -> Result<(), String> {
    use std::collections::HashMap;
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};
    let failed = || format!("Couldn\u{2019}t join \u{201c}{}\u{201d}.", network.ssid);
    let conn = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
    let iface = wifi_iface().ok_or("There's no Wi-Fi device.")?;
    let device: OwnedObjectPath = conn
        .call_method(Some(NM), "/org/freedesktop/NetworkManager", Some(NM), "GetDeviceByIpIface", &(iface.as_str(),))
        .and_then(|r| r.body().deserialize())
        .map_err(|e| e.to_string())?;

    let mut wireless: HashMap<&str, Value> = HashMap::new();
    wireless.insert("ssid", Value::from(network.ssid.as_bytes().to_vec()));
    wireless.insert("mode", Value::from("infrastructure"));
    let mut security: HashMap<&str, Value> = HashMap::new();
    if network.security.contains("WEP") {
        security.insert("key-mgmt", Value::from("none"));
        security.insert("wep-key0", Value::from(password));
        // A passphrase rather than a hex key.
        security.insert("wep-key-type", Value::from(2u32));
    } else {
        // WPA3-only networks take SAE; transition networks take either.
        let sae = network.security.contains("WPA3") && !network.security.contains("WPA2") && !network.security.contains("WPA1");
        security.insert("key-mgmt", Value::from(if sae { "sae" } else { "wpa-psk" }));
        security.insert("psk", Value::from(password));
    }
    let mut connection: HashMap<&str, Value> = HashMap::new();
    connection.insert("id", Value::from(network.ssid.as_str()));
    connection.insert("type", Value::from("802-11-wireless"));
    let mut settings: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
    settings.insert("connection", connection);
    settings.insert("802-11-wireless", wireless);
    settings.insert("802-11-wireless-security", security);

    let any = ObjectPath::try_from("/").map_err(|e| e.to_string())?;
    let (profile, active): (OwnedObjectPath, OwnedObjectPath) = conn
        .call_method(Some(NM), "/org/freedesktop/NetworkManager", Some(NM), "AddAndActivateConnection", &(settings, &device, any))
        .and_then(|r| r.body().deserialize())
        .map_err(|e| e.to_string())?;

    // Activating (1) until activated (2), or gone or deactivated when it failed.
    let state = || -> Option<u32> {
        let props = zbus::blocking::fdo::PropertiesProxy::builder(&conn).destination(NM).ok()?.path(active.as_str()).ok()?.build().ok()?;
        let iface = zbus::names::InterfaceName::try_from("org.freedesktop.NetworkManager.Connection.Active").ok()?;
        props.get(iface, "State").ok()?.try_into().ok()
    };
    for _ in 0..180 {
        std::thread::sleep(Duration::from_millis(250));
        match state() {
            Some(2) => return Ok(()),
            Some(0 | 1) => continue,
            _ => break,
        }
    }
    let _ = conn.call_method(Some(NM), profile.as_str(), Some("org.freedesktop.NetworkManager.Settings.Connection"), "Delete", &());
    Err(format!("{} Check the password and try again.", failed()))
}

/// A DBus property from a GetAll-style map.
fn prop<T: TryFrom<zbus::zvariant::OwnedValue>>(m: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>, key: &str) -> Option<T> {
    m.get(key)?.try_clone().ok()?.try_into().ok()
}

/// Paired and connected devices, from BlueZ's object tree.
fn bluez_devices() -> Option<Vec<BtDevice>> {
    let conn = zbus::blocking::Connection::system().ok()?;
    let objects: zbus::fdo::ManagedObjects = conn
        .call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let mut out = Vec::new();
    for (path, ifaces) in &objects {
        let iface = |name: &str| ifaces.iter().find(|(k, _)| k.as_str() == name).map(|(_, v)| v);
        let Some(d) = iface("org.bluez.Device1") else { continue };
        let connected = prop::<bool>(d, "Connected").unwrap_or(false);
        if !prop::<bool>(d, "Paired").unwrap_or(false) && !connected {
            continue;
        }
        let name = prop::<String>(d, "Alias").or_else(|| prop(d, "Name")).or_else(|| prop(d, "Address")).unwrap_or_default();
        out.push(BtDevice {
            path: path.to_string(),
            name,
            kind: DeviceKind::from_icon(&prop::<String>(d, "Icon").unwrap_or_default()),
            connected,
            battery: iface("org.bluez.Battery1").and_then(|b| prop::<u8>(b, "Percentage")),
        });
    }
    out.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())).then(a.path.cmp(&b.path)));
    Some(out)
}

/// Calls `changed` when a Bluetooth adapter or device changes in a way we
/// show (BlueZ also reports signal strength constantly while discovering).
fn watch_bluez(conn: zbus::blocking::Connection, changed: Arc<dyn Fn(Part) + Send + Sync>) {
    const SHOWN: [&str; 6] = ["Powered", "Connected", "Paired", "Alias", "Name", "Percentage"];
    // Only BlueZ has objects under /org/bluez. (Not matched by sender: the
    // signals carry BlueZ's unique name, not "org.bluez".)
    let props = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .member("PropertiesChanged")
        .and_then(|b| b.path_namespace("/org/bluez"))
        .map(|b| b.build());
    if let Ok(rule) = props {
        let (conn, changed) = (conn.clone(), changed.clone());
        std::thread::spawn(move || {
            let Ok(iter) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64)) else { return };
            for msg in iter.flatten() {
                type Body = (String, std::collections::HashMap<String, zbus::zvariant::OwnedValue>, Vec<String>);
                let Ok((_, props, gone)) = msg.body().deserialize::<Body>() else { continue };
                if props.keys().chain(gone.iter()).any(|k| SHOWN.contains(&k.as_str())) {
                    changed(Part::Bluetooth);
                }
            }
        });
    }
    // Devices pairing and being removed: BlueZ's object manager reports
    // objects coming and going (other services' too, told apart by path).
    for member in ["InterfacesAdded", "InterfacesRemoved"] {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.DBus.ObjectManager")
            .and_then(|b| b.member(member))
            .map(|b| b.build());
        let Ok(rule) = rule else { continue };
        let (conn, changed) = (conn.clone(), changed.clone());
        std::thread::spawn(move || {
            let Ok(iter) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64)) else { return };
            for msg in iter.flatten() {
                let body = msg.body();
                let Ok(path) = body.deserialize::<(zbus::zvariant::OwnedObjectPath, zbus::zvariant::OwnedValue)>().map(|b| b.0) else {
                    continue;
                };
                if path.as_str().starts_with("/org/bluez/") {
                    changed(Part::Bluetooth);
                }
            }
        });
    }
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

    fn networks(&self) -> Vec<Network> {
        let Some(list) = output("nmcli", &["-t", "-f", "IN-USE,SSID,SECURITY,SIGNAL", "device", "wifi", "list", "--rescan", "no"]) else {
            return Vec::new();
        };
        let known = output("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"]).map(|s| parse_known(&s)).unwrap_or_default();
        parse_networks(&list, &known)
    }

    fn scan_wifi(&self) {
        // Asking for a rescan makes nmcli wait for its results.
        let _ = output("nmcli", &["-t", "-f", "SSID", "device", "wifi", "list", "--rescan", "yes"]);
    }

    fn join_wifi(&self, network: &Network, password: Option<&str>) -> Result<(), String> {
        let r = match password {
            _ if network.known => run_checked("nmcli", &["--wait", "45", "connection", "up", "id", &network.ssid]),
            Some(pw) if network.secured() => return nm_join(network, pw),
            _ => run_checked("nmcli", &["--wait", "45", "device", "wifi", "connect", &network.ssid]),
        };
        r.map_err(|e| {
            eprintln!("y7-shell: couldn't join {}: {e}", network.ssid);
            if e.contains("Secrets were required") {
                "The password is incorrect.".to_string()
            } else {
                format!("Couldn\u{2019}t join \u{201c}{}\u{201d}.", network.ssid)
            }
        })
    }

    fn bluetooth(&self) -> Option<bool> {
        let out = output("bluetoothctl", &["show"])?;
        let powered = out.lines().find_map(|l| l.trim().strip_prefix("Powered:"))?;
        Some(powered.trim() == "yes")
    }

    fn set_bluetooth(&self, on: bool) {
        run("bluetoothctl", &["power", if on { "on" } else { "off" }]);
    }

    fn bt_devices(&self) -> Vec<BtDevice> {
        bluez_devices().unwrap_or_default()
    }

    fn connect_bt(&self, path: &str, connect: bool) -> Result<(), String> {
        let conn = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
        let method = if connect { "Connect" } else { "Disconnect" };
        conn.call_method(Some("org.bluez"), path, Some("org.bluez.Device1"), method, &()).map(drop).map_err(|e| e.to_string())
    }

    fn volume(&self) -> Option<Volume> {
        parse_wpctl(&output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?)
    }

    fn set_volume(&self, level: f32) {
        run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{level:.3}")]);
        run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
    }

    fn outputs(&self) -> Vec<Output> {
        let Some(list) = output_c("pactl", &["list", "sinks"]) else { return Vec::new() };
        let default = output("pactl", &["get-default-sink"]).unwrap_or_default();
        parse_sinks(&list, default.trim())
    }

    fn set_output(&self, name: &str) {
        run("pactl", &["set-default-sink", name]);
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
            for (path, part) in
                [("/org/freedesktop/UPower/devices/DisplayDevice", Part::Battery), ("/org/freedesktop/NetworkManager", Part::Wifi)]
            {
                if let Some(rule) = signal_rule(path, "PropertiesChanged") {
                    let c = changed.clone();
                    on_signals(conn.clone(), rule, move || c(part));
                }
            }
            watch_bluez(conn.clone(), changed.clone());
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
                        // Outputs plugged in or out, or another one chosen.
                        if line.contains("'new' on sink #") || line.contains("'remove' on sink #") || line.contains("on server") {
                            c(Part::Outputs);
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
    fn slow_jobs_report_back() {
        let sys = FakeSystem::demo();
        let mut status = Status::default();
        handle(&sys, vec![Request::Refresh(Part::Wifi), Request::Refresh(Part::Bluetooth)], &mut status);
        let rivers = status.networks.iter().find(|n| n.ssid == "Rivers-5G").unwrap().clone();

        // A wrong password: the job runs off the worker, then reports.
        let jobs = handle(&sys, vec![Request::JoinWifi(rivers.clone(), Some("nope".into()))], &mut status);
        assert_eq!(status.joining.as_deref(), Some("Rivers-5G"));
        assert_eq!(jobs.len(), 1);
        // A second join waits for the first.
        assert!(handle(&sys, vec![Request::JoinWifi(rivers.clone(), None)], &mut status).is_empty());
        let done = run_job(&sys, jobs.into_iter().next().unwrap());
        handle(&sys, vec![done], &mut status);
        assert_eq!(status.joining, None);
        assert_eq!(status.join_error.as_ref().map(|e| e.0.as_str()), Some("Rivers-5G"));

        let jobs = handle(&sys, vec![Request::JoinWifi(rivers, Some(FakeSystem::PASSWORD.into()))], &mut status);
        assert_eq!(status.join_error, None);
        let done = run_job(&sys, jobs.into_iter().next().unwrap());
        handle(&sys, vec![done], &mut status);
        assert_eq!(status.wifi.ssid.as_deref(), Some("Rivers-5G"));
        assert!(status.networks.iter().any(|n| n.ssid == "Rivers-5G" && n.active && n.known));

        let keyboard = status.devices.iter().find(|d| d.name == "Magic Keyboard").unwrap().path.clone();
        let jobs = handle(&sys, vec![Request::ConnectBt(keyboard.clone(), true)], &mut status);
        assert_eq!(status.device_busy.as_ref(), Some(&keyboard));
        let done = run_job(&sys, jobs.into_iter().next().unwrap());
        handle(&sys, vec![done], &mut status);
        assert_eq!(status.device_busy, None);
        assert!(status.devices.iter().any(|d| d.path == keyboard && d.connected));

        handle(&sys, vec![Request::SetOutput("airpods".into())], &mut status);
        assert_eq!(status.outputs.iter().find(|o| o.default).map(|o| o.name.as_str()), Some("airpods"));
    }

    #[test]
    fn parses_networks_and_sinks() {
        assert_eq!(nmcli_fields(r"a\:b:c\\d::"), ["a:b", r"c\d", "", ""]);
        let list = "*:Home\\:5G:WPA2:70\n :Cafe::40\n :Home\\:5G:WPA2:80\n ::WPA2:90\n :Old:--:20\n :Work:WPA2 802.1X:50\n";
        let known = parse_known("Home\\:5G:802-11-wireless\nlo:loopback\nWork:802-11-wireless\n");
        assert_eq!(known, ["Home:5G", "Work"]);
        let nets = parse_networks(list, &known);
        let names: Vec<&str> = nets.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["Home:5G", "Work", "Cafe", "Old"]);
        assert!(nets[0].active && nets[0].strength == 80);
        assert!(nets[1].enterprise() && nets[1].known);
        assert!(!nets[2].secured() && !nets[3].secured());

        let sinks = "Sink #1\n\tName: alsa_output.pci\n\tDescription: Built-in Audio\n\tActive Port: analog-output-speaker\n\
Sink #2\n\tName: bluez_output.AA\n\tDescription: AirPods\n\t\tdevice.icon_name = \"audio-headset-bluetooth\"\n";
        let outs = parse_sinks(sinks, "bluez_output.AA");
        assert_eq!(outs.len(), 2);
        assert_eq!((outs[0].description.as_str(), outs[0].default, outs[0].kind), ("Built-in Audio", false, DeviceKind::Speaker));
        assert_eq!((outs[1].name.as_str(), outs[1].default, outs[1].kind), ("bluez_output.AA", true, DeviceKind::Headphones));
        assert_eq!(DeviceKind::from_icon("input-mouse"), DeviceKind::Mouse);
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
