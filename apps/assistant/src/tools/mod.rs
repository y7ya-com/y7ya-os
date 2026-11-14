//! The settings tools the model can call. Each is a small typed function that
//! reads the current state, makes one change, reads the state again to verify
//! it, and says how to undo it. There's deliberately no way to run an
//! arbitrary command: system programs are only reached through [`Runner`]
//! with fixed argument shapes.
//!
//! Risky calls (anything that could cut the network or change system-wide
//! state) aren't executed when the model asks. They come back as
//! [`CallResult::NeedsConfirmation`], and only the app runs them, after the
//! user clicks the confirm button on the card.

mod apps;
mod bluetooth;
mod desktop;
mod display;
mod network;
mod power;
pub mod runner;
mod sound;
mod system;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value, json};

pub use runner::{Runner, SystemRunner};

/// What a tool belongs to; picks the icon on its card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Appearance,
    Wallpaper,
    Display,
    NightLight,
    Sound,
    Wifi,
    Bluetooth,
    Power,
    Notifications,
    Keyboard,
    Apps,
    DateTime,
    About,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Only reads state.
    Read,
    /// Changes something.
    Change,
}

/// The question on a confirmation card.
#[derive(Clone, Debug, PartialEq)]
pub struct Confirm {
    pub title: String,
    pub detail: String,
    pub button: String,
    pub destructive: bool,
}

/// Another tool call, used for undo.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub tool: String,
    pub args: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// One line for the card, e.g. "Changed appearance to Dark".
    pub summary: String,
    /// The state after the call, for the model.
    pub state: Value,
    pub undo: Option<Action>,
}

impl Outcome {
    pub fn new(summary: impl Into<String>, state: Value) -> Self {
        Self { summary: summary.into(), state, undo: None }
    }

    pub fn undo(mut self, tool: &str, args: Value) -> Self {
        self.undo = Some(Action { tool: tool.into(), args });
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CallResult {
    Done(Outcome),
    NeedsConfirmation(Confirm),
    Failed(String),
}

impl CallResult {
    /// The JSON the model sees, and the app reads back to draw the card.
    pub fn to_json(&self) -> Value {
        match self {
            CallResult::Done(o) => {
                let mut v = json!({ "ok": true, "summary": o.summary, "state": o.state });
                if let Some(u) = &o.undo {
                    v["undo"] = json!({ "tool": u.tool, "args": u.args });
                }
                v
            }
            CallResult::NeedsConfirmation(c) => json!({
                "ok": false,
                "needs_confirmation": true,
                "confirm": { "title": c.title, "detail": c.detail, "button": c.button, "destructive": c.destructive },
                "message": format!(
                    "Not done yet: the app is showing the user a \u{201c}{}\u{201d} button. It runs only if they click it. Don't call this tool again for this request.",
                    c.button
                ),
            }),
            CallResult::Failed(e) => json!({ "ok": false, "error": e }),
        }
    }

    pub fn from_json(v: &Value) -> CallResult {
        if v["ok"].as_bool() == Some(true) {
            let undo = v.get("undo").and_then(|u| {
                Some(Action { tool: u["tool"].as_str()?.to_string(), args: u.get("args").cloned().unwrap_or(json!({})) })
            });
            return CallResult::Done(Outcome {
                summary: v["summary"].as_str().unwrap_or("Done").to_string(),
                state: v.get("state").cloned().unwrap_or(Value::Null),
                undo,
            });
        }
        if v["needs_confirmation"].as_bool() == Some(true) {
            let c = &v["confirm"];
            return CallResult::NeedsConfirmation(Confirm {
                title: c["title"].as_str().unwrap_or("Confirm this change?").to_string(),
                detail: c["detail"].as_str().unwrap_or_default().to_string(),
                button: c["button"].as_str().unwrap_or("Confirm").to_string(),
                destructive: c["destructive"].as_bool().unwrap_or(false),
            });
        }
        CallResult::Failed(v["error"].as_str().unwrap_or("Something went wrong").to_string())
    }
}

pub struct Tool {
    pub name: &'static str,
    pub category: Category,
    /// Shown on the card while it runs, e.g. "Changing appearance".
    pub doing: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    /// JSON Schema for the arguments.
    pub schema: fn() -> Value,
    /// Whether these arguments need the user's go-ahead first.
    pub confirm: fn(&Value) -> Option<Confirm>,
    pub run: fn(&Ctx, &Value) -> Result<Outcome, String>,
}

pub fn never(_: &Value) -> Option<Confirm> {
    None
}

/// Every tool, in the order the model sees them.
pub fn all() -> impl Iterator<Item = &'static Tool> {
    [desktop::TOOLS, display::TOOLS, sound::TOOLS, network::TOOLS, bluetooth::TOOLS, power::TOOLS, apps::TOOLS, system::TOOLS]
        .into_iter()
        .flatten()
}

pub fn find(name: &str) -> Option<&'static Tool> {
    all().find(|t| t.name == name)
}

/// Runs a tool. `confirmed` is only ever true when the user clicked the
/// button on the card in the app (never for calls coming from the model).
pub fn call(ctx: &Ctx, name: &str, args: &Value, confirmed: bool) -> CallResult {
    let Some(tool) = find(name) else { return CallResult::Failed(format!("There's no tool called {name}.")) };
    let empty = json!({});
    let args = if args.is_null() { &empty } else { args };
    if !args.is_object() {
        return CallResult::Failed("Arguments must be a JSON object.".into());
    }
    if ctx.read_only && tool.effect == Effect::Change {
        return CallResult::Failed("Changes are turned off (read-only mode).".into());
    }
    if !confirmed {
        if let Some(c) = (tool.confirm)(args) {
            return CallResult::NeedsConfirmation(c);
        }
    }
    match (tool.run)(ctx, args) {
        Ok(o) => CallResult::Done(o),
        Err(e) => CallResult::Failed(e),
    }
}

/// Where tools read and write. Tests point everything at fakes.
pub struct Ctx {
    pub run: Arc<dyn Runner>,
    /// The y7ya-os settings file.
    pub settings: PathBuf,
    /// Prefix for system files (`/sys`, `/proc`, `/etc`, `/usr/share`).
    pub root: PathBuf,
    /// `~/.config`, for `kdeglobals`.
    pub config_home: PathBuf,
    /// Directories holding `.desktop` files, most important first.
    pub app_dirs: Vec<PathBuf>,
    /// Refuse every change (used to try the backends safely).
    pub read_only: bool,
}

impl Ctx {
    pub fn system() -> Ctx {
        let home = dirs::home_dir().unwrap_or_default();
        let data_home = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
        let mut app_dirs = vec![data_home.join("applications"), data_home.join("flatpak/exports/share/applications")];
        let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
        let data_dirs = if data_dirs.is_empty() { "/usr/local/share:/usr/share".to_string() } else { data_dirs };
        app_dirs.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("applications")));
        app_dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
        app_dirs.dedup();
        Ctx {
            run: Arc::new(SystemRunner),
            settings: settings::path(),
            root: PathBuf::from("/"),
            config_home: dirs::config_dir().unwrap_or_else(|| home.join(".config")),
            app_dirs,
            read_only: std::env::var_os("ASSISTANT_READ_ONLY").is_some(),
        }
    }

    /// A system path under `root`.
    fn sys(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.sys(path)).ok().map(|s| s.trim().to_string())
    }

    /// Runs a program and returns its stdout, or a readable error.
    fn cmd(&self, program: &str, args: &[&str]) -> Result<String, String> {
        if !self.run.has(program) {
            return Err(format!("{program} isn't installed."));
        }
        let out = self.run.run(program, args).map_err(|e| format!("Couldn't run {program}: {e}"))?;
        if out.success {
            Ok(out.stdout)
        } else {
            let msg = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
            let msg = msg.lines().last().unwrap_or("").trim();
            Err(if msg.is_empty() { format!("{program} failed.") } else { format!("{program}: {msg}") })
        }
    }

    fn load_settings(&self) -> Result<settings::Settings, String> {
        settings::Settings::load_from(&self.settings).map_err(|e| format!("Couldn't read {}: {e}", self.settings.display()))
    }

    fn update_settings(&self, f: impl FnOnce(&mut settings::Settings)) -> Result<settings::Settings, String> {
        settings::Settings::update_at(&self.settings, f).map_err(|e| format!("Couldn't save {}: {e}", self.settings.display()))?;
        // Verify by reading back what's on disk.
        self.load_settings()
    }
}

// -- argument helpers ---------------------------------------------------------

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    args.get(key).filter(|v| !v.is_null())
}

fn str_arg(args: &Value, key: &str) -> Result<String, String> {
    opt_str(args, key)?.ok_or_else(|| format!("Missing \u{201c}{key}\u{201d}."))
}

fn opt_str(args: &Value, key: &str) -> Result<Option<String>, String> {
    match arg(args, key) {
        None => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("\u{201c}{key}\u{201d} must be a non-empty string.")),
    }
}

fn num_arg(args: &Value, key: &str) -> Result<f64, String> {
    opt_num(args, key)?.ok_or_else(|| format!("Missing \u{201c}{key}\u{201d}."))
}

fn opt_num(args: &Value, key: &str) -> Result<Option<f64>, String> {
    match arg(args, key) {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .or_else(|| v.as_str().and_then(|s| s.trim().trim_end_matches('%').parse().ok()))
            .filter(|n| n.is_finite())
            .map(Some)
            .ok_or_else(|| format!("\u{201c}{key}\u{201d} must be a number.")),
    }
}

fn bool_arg(args: &Value, key: &str) -> Result<bool, String> {
    opt_bool(args, key)?.ok_or_else(|| format!("Missing \u{201c}{key}\u{201d}."))
}

fn opt_bool(args: &Value, key: &str) -> Result<Option<bool>, String> {
    match arg(args, key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(Value::String(s)) if matches!(s.as_str(), "true" | "on") => Ok(Some(true)),
        Some(Value::String(s)) if matches!(s.as_str(), "false" | "off") => Ok(Some(false)),
        Some(_) => Err(format!("\u{201c}{key}\u{201d} must be true or false.")),
    }
}

/// A JSON Schema object from `(name, schema)` pairs.
fn object(props: &[(&str, Value)], required: &[&str]) -> Value {
    let mut p = Map::new();
    for (k, v) in props {
        p.insert(k.to_string(), v.clone());
    }
    json!({ "type": "object", "properties": p, "required": required, "additionalProperties": false })
}

fn no_args() -> Value {
    object(&[], &[])
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// Splits one line of `nmcli -t` output, honoring `\:` escapes.
fn split_terse(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.last_mut().unwrap().push(n);
                }
            }
            ':' => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests;
