//! The y7ya-os settings file, shared by the desktop, the Assistant and apps.
//!
//! It lives at `~/.config/y7ya-os/settings.toml` (`$XDG_CONFIG_HOME` is
//! honored, and `Y7YA_CONFIG_DIR` overrides the directory). Every key is
//! optional; missing keys take the defaults shown here, and keys this version
//! doesn't know are kept when the file is saved again.
//!
//! ```toml
//! [appearance]
//! mode = "auto"            # "light" | "dark" | "auto" (dark from sunset to sunrise)
//! accent = "blue"          # blue purple pink red orange yellow green graphite
//!
//! [appearance.app_accents] # optional per-app accent overrides: app id = "#rrggbb"
//! notes = "#e2a800"
//!
//! [wallpaper]
//! path = ""                # image file; empty means the desktop's default
//! fit = "fill"             # "fill" | "fit" | "center"
//!
//! [display]
//! scale = 1.0              # UI scale, 1.0–3.0
//! text_scale = 1.0         # extra text size on top of the UI scale, 0.8–1.6
//!
//! [display.night_light]
//! enabled = false
//! temperature = 4000       # Kelvin, 2500 (warm) – 6500 (neutral)
//! schedule = "sunset"      # "always" | "sunset" (sunset to sunrise) | "custom"
//! from = "22:00"           # used with "custom"
//! to = "07:00"
//!
//! [notifications]
//! do_not_disturb = false
//!
//! [keyboard]
//! repeat_rate = 30         # repeats per second, 1–100
//! repeat_delay = 500       # milliseconds before repeating starts, 100–2000
//! ```
//!
//! Writers save atomically (write a temporary file, then rename), so readers
//! never see half a file. [`watch`] lets the desktop and apps react live.

mod watch;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use watch::{Watcher, watch};

pub const FILE_NAME: &str = "settings.toml";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub wallpaper: Wallpaper,
    pub display: Display,
    pub notifications: Notifications,
    pub keyboard: Keyboard,
    /// Sections this version doesn't know about, kept as-is.
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Light,
    Dark,
    #[default]
    Auto,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_lowercase().as_str() {
            "light" => Some(Mode::Light),
            "dark" => Some(Mode::Dark),
            "auto" | "automatic" => Some(Mode::Auto),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
            Mode::Auto => "auto",
        }
    }

    /// Whether this mode is dark at the given local hour (0–23). `Auto` uses
    /// a fixed evening window until the desktop knows the real sunset.
    pub fn is_dark_at(self, hour: u32) -> bool {
        match self {
            Mode::Light => false,
            Mode::Dark => true,
            Mode::Auto => !(7..19).contains(&hour),
        }
    }
}

/// The macOS accent colors.
pub const ACCENTS: [(&str, u32); 8] = [
    ("blue", 0x007aff),
    ("purple", 0xa550a7),
    ("pink", 0xf74f9e),
    ("red", 0xff5257),
    ("orange", 0xf7821b),
    ("yellow", 0xffc600),
    ("green", 0x62ba46),
    ("graphite", 0x8c8c8c),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub mode: Mode,
    pub accent: String,
    /// App id → `#rrggbb`.
    pub app_accents: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { mode: Mode::Auto, accent: "blue".into(), app_accents: BTreeMap::new(), extra: toml::Table::new() }
    }
}

impl Appearance {
    /// The accent as `0xrrggbb`, falling back to blue for unknown names.
    pub fn accent_rgb(&self) -> u32 {
        ACCENTS.iter().find(|(n, _)| *n == self.accent).map_or(ACCENTS[0].1, |(_, c)| *c)
    }

    /// An app's own accent override, if it has a valid one.
    pub fn app_accent(&self, app: &str) -> Option<u32> {
        let hex = self.app_accents.get(app)?.trim_start_matches('#');
        (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    #[default]
    Fill,
    Fit,
    Center,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wallpaper {
    pub path: String,
    pub fit: Fit,
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub scale: f64,
    pub text_scale: f64,
    pub night_light: NightLight,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Display {
    fn default() -> Self {
        Self { scale: 1.0, text_scale: 1.0, night_light: NightLight::default(), extra: toml::Table::new() }
    }
}

pub const SCALE_RANGE: (f64, f64) = (1.0, 3.0);
pub const TEXT_SCALE_RANGE: (f64, f64) = (0.8, 1.6);
pub const TEMPERATURE_RANGE: (u32, u32) = (2500, 6500);
pub const REPEAT_RATE_RANGE: (u32, u32) = (1, 100);
pub const REPEAT_DELAY_RANGE: (u32, u32) = (100, 2000);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Schedule {
    Always,
    #[default]
    Sunset,
    Custom,
}

impl Schedule {
    pub fn parse(s: &str) -> Option<Schedule> {
        match s.trim().to_lowercase().as_str() {
            "always" | "manual" | "on" => Some(Schedule::Always),
            "sunset" | "sunset-to-sunrise" => Some(Schedule::Sunset),
            "custom" => Some(Schedule::Custom),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Schedule::Always => "always",
            Schedule::Sunset => "sunset",
            Schedule::Custom => "custom",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NightLight {
    pub enabled: bool,
    pub temperature: u32,
    pub schedule: Schedule,
    pub from: String,
    pub to: String,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for NightLight {
    fn default() -> Self {
        Self {
            enabled: false,
            temperature: 4000,
            schedule: Schedule::Sunset,
            from: "22:00".into(),
            to: "07:00".into(),
            extra: toml::Table::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub do_not_disturb: bool,
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keyboard {
    pub repeat_rate: u32,
    pub repeat_delay: u32,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self { repeat_rate: 30, repeat_delay: 500, extra: toml::Table::new() }
    }
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Parse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Parse(e) => write!(f, "the settings file isn't valid TOML: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// `~/.config/y7ya-os`, or `Y7YA_CONFIG_DIR`.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("Y7YA_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config")).join("y7ya-os")
}

/// The settings file's path.
pub fn path() -> PathBuf {
    config_dir().join(FILE_NAME)
}

impl Settings {
    /// Reads the settings file; a missing file gives the defaults.
    pub fn load() -> Result<Settings, Error> {
        Self::load_from(&path())
    }

    /// Like [`Settings::load`] but never fails: errors give the defaults.
    pub fn load_or_default() -> Settings {
        Self::load().unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Result<Settings, Error> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn parse(s: &str) -> Result<Settings, Error> {
        let mut settings: Settings = toml::from_str(s).map_err(|e| Error::Parse(e.message().to_string()))?;
        settings.clamp();
        Ok(settings)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), Error> {
        self.save_to(&path())
    }

    /// Writes atomically: a temporary file next to `path`, then a rename.
    pub fn save_to(&self, path: &Path) -> Result<(), Error> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
        std::fs::write(&tmp, self.to_toml())?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(())
    }

    /// Loads, applies `f`, saves, and returns the saved settings.
    pub fn update_at(path: &Path, f: impl FnOnce(&mut Settings)) -> Result<Settings, Error> {
        let mut s = Self::load_from(path)?;
        f(&mut s);
        s.clamp();
        s.save_to(path)?;
        Ok(s)
    }

    /// Keeps every value inside its documented range.
    pub fn clamp(&mut self) {
        let d = &mut self.display;
        d.scale = clamp_f(d.scale, SCALE_RANGE, 1.0);
        d.text_scale = clamp_f(d.text_scale, TEXT_SCALE_RANGE, 1.0);
        let n = &mut d.night_light;
        n.temperature = n.temperature.clamp(TEMPERATURE_RANGE.0, TEMPERATURE_RANGE.1);
        let k = &mut self.keyboard;
        k.repeat_rate = k.repeat_rate.clamp(REPEAT_RATE_RANGE.0, REPEAT_RATE_RANGE.1);
        k.repeat_delay = k.repeat_delay.clamp(REPEAT_DELAY_RANGE.0, REPEAT_DELAY_RANGE.1);
    }
}

fn clamp_f(v: f64, (lo, hi): (f64, f64), fallback: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("y7ya-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_from_empty_file() {
        let s = Settings::parse("").unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!(s.appearance.mode, Mode::Auto);
        assert_eq!(s.display.scale, 1.0);
        assert_eq!(s.keyboard.repeat_delay, 500);
    }

    #[test]
    fn parses_documented_example_and_clamps() {
        let s = Settings::parse(
            r##"
            [appearance]
            mode = "dark"
            accent = "purple"
            [appearance.app_accents]
            notes = "#e2a800"
            [display]
            scale = 9.0
            [display.night_light]
            enabled = true
            temperature = 1000
            schedule = "custom"
            [keyboard]
            repeat_rate = 0
            "##,
        )
        .unwrap();
        assert_eq!(s.appearance.mode, Mode::Dark);
        assert_eq!(s.appearance.accent_rgb(), 0xa550a7);
        assert_eq!(s.appearance.app_accent("notes"), Some(0xe2a800));
        assert_eq!(s.display.scale, 3.0);
        assert!(s.display.night_light.enabled);
        assert_eq!(s.display.night_light.temperature, 2500);
        assert_eq!(s.display.night_light.schedule, Schedule::Custom);
        assert_eq!(s.keyboard.repeat_rate, 1);
    }

    #[test]
    fn keeps_unknown_keys_and_round_trips() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, "[dock]\nsize = 48\n[display]\nscale = 2.0\nhdr = true\n").unwrap();
        let s = Settings::update_at(&path, |s| s.notifications.do_not_disturb = true).unwrap();
        assert!(s.notifications.do_not_disturb);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[dock]"), "{text}");
        assert!(text.contains("hdr = true"), "{text}");
        let again = Settings::load_from(&path).unwrap();
        assert_eq!(again, s);
        assert_eq!(again.display.scale, 2.0);
    }

    #[test]
    fn invalid_file_is_an_error() {
        assert!(matches!(Settings::parse("[appearance]\nmode = \"purple\""), Err(Error::Parse(_))));
        assert!(Settings::parse("not toml at all [").is_err());
    }

    #[test]
    fn auto_mode_follows_the_clock() {
        assert!(Mode::Auto.is_dark_at(22));
        assert!(Mode::Auto.is_dark_at(3));
        assert!(!Mode::Auto.is_dark_at(12));
        assert!(Mode::Dark.is_dark_at(12));
        assert_eq!(Mode::parse(" Dark "), Some(Mode::Dark));
    }

    #[test]
    fn watcher_sees_saves() {
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = temp_dir("watch");
        let path = dir.join(FILE_NAME);
        let (tx, rx) = mpsc::channel();
        let watcher = watch(&path, move |s: &Settings| {
            let _ = tx.send(s.appearance.mode);
        })
        .unwrap();
        // Unrelated files in the directory are ignored.
        std::fs::write(dir.join("other.txt"), "x").unwrap();
        Settings::update_at(&path, |s| s.appearance.mode = Mode::Dark).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Mode::Dark);
        Settings::update_at(&path, |s| s.appearance.mode = Mode::Light).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Mode::Light);
        drop(watcher);
        Settings::update_at(&path, |s| s.appearance.mode = Mode::Dark).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
