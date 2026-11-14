//! The AI side: the user's own logged-in `claude` or `codex` CLI, driven the
//! way T3 Code does it. We keep no API keys. Each turn spawns the CLI, which
//! starts `assistant --mcp` for the settings tools, streams its answer back as
//! [`Event`]s, and exits; follow-ups resume the same session. Nothing runs
//! between turns.

pub mod claude;
pub mod codex;

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::tools::CallResult;
use crate::tools::runner::which;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Claude,
    Codex,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Claude, Kind::Codex];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Codex => "codex",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == s)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Claude => "Claude",
            Kind::Codex => "Codex",
        }
    }

    /// The CLI's own name, for messages.
    pub fn product(self) -> &'static str {
        match self {
            Kind::Claude => "Claude Code",
            Kind::Codex => "Codex CLI",
        }
    }

    pub fn install_command(self) -> &'static str {
        match self {
            Kind::Claude => "curl -fsSL https://claude.ai/install.sh | bash",
            Kind::Codex => "npm install -g @openai/codex",
        }
    }

    pub fn login_command(self) -> &'static str {
        match self {
            Kind::Claude => "claude auth login",
            Kind::Codex => "codex login",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Checking,
    Ready,
    NotInstalled,
    SignedOut,
}

/// Whether a backend can be used (blocking: run it off the UI thread).
pub fn check(kind: Kind) -> Status {
    let Some(bin) = which(kind.key()) else { return Status::NotInstalled };
    let args: &[&str] = match kind {
        Kind::Claude => &["auth", "status"],
        Kind::Codex => &["login", "status"],
    };
    let Ok(out) = Command::new(bin).args(args).stdin(Stdio::null()).output() else { return Status::NotInstalled };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let all = format!("{stdout}{}", String::from_utf8_lossy(&out.stderr)).to_lowercase();
    match kind {
        Kind::Claude => match serde_json::from_str::<Value>(&stdout).ok().and_then(|v| v["loggedIn"].as_bool()) {
            Some(true) => Status::Ready,
            Some(false) => Status::SignedOut,
            // An older CLI without `auth status`: let the turn itself report problems.
            None => Status::Ready,
        },
        Kind::Codex if out.status.success() && all.contains("logged in") && !all.contains("not logged in") => Status::Ready,
        Kind::Codex => Status::SignedOut,
    }
}

/// What a turn reports, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The session (Claude) or thread (Codex) id to resume next time.
    Session(String),
    /// More of the assistant's reply.
    Text(String),
    ToolStart { id: String, name: String, args: Value },
    ToolEnd { id: String, result: CallResult },
    Error(String),
    /// The turn is over (always the last event).
    Done,
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

pub const SYSTEM_PROMPT: &str = "You are Assistant, the settings helper built into y7ya-os, a Mac-inspired Linux desktop. There is no Settings app: people ask in plain language and you change their system with the y7ya settings tools, which are your only tools.

- Act, don't explain: call the tool that does what was asked. Read the current state first when the request is relative (\"louder\", \"bigger text\", \"a bit dimmer\") or a question.
- Every change is verified and shown to the user as a card (with Undo where possible), so afterwards just confirm in one short sentence. Don't repeat numbers the card already shows unless asked.
- Some changes (turning Wi-Fi or Bluetooth off, joining a Wi-Fi network, disconnecting a device, changing the time zone) need the user's go-ahead: the tool returns needs_confirmation and the app shows a button. Tell them to confirm on the card, and don't call the tool again.
- Messages starting with [The user ...] are notes from the app about what the user did on a card.
- Reply in one or two short sentences of plain text. No Markdown, lists, headings or code formatting.
- If a tool fails, say simply what went wrong and what they can do.
- You only handle this computer's settings. For anything else, say briefly that you're the settings assistant.";

/// How the CLIs start our MCP server: this same binary with `--mcp`.
pub fn mcp_command() -> (String, Vec<String>) {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "assistant".into());
    (exe, vec!["--mcp".into()])
}

/// An empty directory the CLIs run in, so no project files or instructions
/// get picked up.
pub fn workdir() -> PathBuf {
    let dir = crate::data_dir().join("session");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A running turn; dropping it doesn't stop it, [`Turn::stop`] does.
#[derive(Clone)]
pub struct Turn {
    child: Arc<Mutex<Option<Child>>>,
}

impl Turn {
    /// A turn with no process behind it (for previews).
    pub fn idle() -> Turn {
        Turn { child: Arc::new(Mutex::new(None)) }
    }

    pub fn stop(&self) {
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }
}

/// Starts one turn on a background thread.
pub fn start(kind: Kind, session: Option<String>, prompt: String, sink: Sink) -> Turn {
    let turn = Turn { child: Arc::new(Mutex::new(None)) };
    let slot = turn.child.clone();
    std::thread::spawn(move || {
        let result = match kind {
            Kind::Claude => claude::run(session, &prompt, &sink, &slot),
            Kind::Codex => codex::run(session, &prompt, &sink, &slot),
        };
        if let Err(e) = result {
            sink(Event::Error(e));
        }
        sink(Event::Done);
    });
    turn
}

/// A spawned CLI with line-by-line stdout and stderr collected for errors.
pub(crate) struct Proc {
    pub stdin: Option<ChildStdin>,
    lines: std::io::Lines<BufReader<std::process::ChildStdout>>,
    stderr: Arc<Mutex<String>>,
    slot: Arc<Mutex<Option<Child>>>,
}

impl Proc {
    pub fn spawn(kind: Kind, args: &[String], slot: &Arc<Mutex<Option<Child>>>) -> Result<Proc, String> {
        let bin = which(kind.key()).ok_or_else(|| format!("{} isn't installed.", kind.product()))?;
        let mut child = Command::new(bin)
            .args(args)
            .current_dir(workdir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Couldn't start {}: {e}", kind.product()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let mut err = child.stderr.take().unwrap();
        let stderr = Arc::new(Mutex::new(String::new()));
        let sink = stderr.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut s = sink.lock().unwrap();
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
                // Keep only the tail.
                if s.len() > 8192 {
                    let cut = s.len() - 4096;
                    let cut = (cut..s.len()).find(|&i| s.is_char_boundary(i)).unwrap_or(cut);
                    s.drain(..cut);
                }
            }
        });
        *slot.lock().unwrap() = Some(child);
        Ok(Proc { stdin, lines: BufReader::new(stdout).lines(), stderr, slot: slot.clone() })
    }

    /// The next line of output; `ASSISTANT_DEBUG=1` echoes the raw protocol to stderr.
    pub fn next_line(&mut self) -> Option<String> {
        let line = self.lines.next()?.ok()?;
        if std::env::var_os("ASSISTANT_DEBUG").is_some() {
            eprintln!("< {line}");
        }
        Some(line)
    }

    /// The last meaningful line the CLI printed to stderr.
    pub fn stderr_tail(&self) -> Option<String> {
        let s = self.stderr.lock().unwrap();
        s.lines().rev().map(str::trim).find(|l| !l.is_empty()).map(|l| l.chars().take(300).collect())
    }

    /// Closes stdin and waits for the CLI to exit (killing it if it lingers).
    pub fn finish(mut self) -> bool {
        self.stdin = None;
        // Lock per check so `Turn::stop` never waits on us.
        for _ in 0..60 {
            let mut slot = self.slot.lock().unwrap();
            let Some(child) = slot.as_mut() else { return false };
            if let Ok(Some(status)) = child.try_wait() {
                *slot = None;
                return status.success();
            }
            drop(slot);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if let Some(mut child) = self.slot.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        false
    }
}

/// A CLI that died before finishing the turn: say why as best we can.
pub(crate) fn died(kind: Kind, p: &Proc) -> String {
    let tail = p.stderr_tail().unwrap_or_default();
    let lower = tail.to_lowercase();
    if lower.contains("login") || lower.contains("logged in") || lower.contains("auth") || lower.contains("api key") {
        return format!("{} isn't signed in. Run \u{201c}{}\u{201d} in a terminal, then try again.", kind.product(), kind.login_command());
    }
    if tail.is_empty() { format!("{} stopped unexpectedly.", kind.product()) } else { format!("{} stopped: {tail}", kind.product()) }
}

/// Turns a tool result's text (our JSON) into a [`CallResult`].
pub(crate) fn tool_result(text: &str, is_error: bool) -> CallResult {
    match serde_json::from_str::<Value>(text) {
        Ok(v) if v.get("ok").is_some() => CallResult::from_json(&v),
        _ if is_error => CallResult::Failed(text.trim().to_string()),
        _ => CallResult::Done(crate::tools::Outcome::new(text.trim(), Value::Null)),
    }
}
