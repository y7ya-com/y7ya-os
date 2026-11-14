//! Running system programs, behind a trait so tests can swap in fakes.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// How long any one system command may take (connecting to Wi-Fi is the slow one).
const TIMEOUT_SECS: &str = "40";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[cfg(test)]
impl Output {
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self { success: true, stdout: stdout.into(), stderr: String::new() }
    }

    pub fn fail(stderr: impl Into<String>) -> Self {
        Self { success: false, stdout: String::new(), stderr: stderr.into() }
    }
}

pub trait Runner: Send + Sync {
    /// Runs `program` with `args` (no shell involved) and waits for it.
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output>;
    /// Whether `program` is installed.
    fn has(&self, program: &str) -> bool;
}

/// The real thing: spawns processes, each under `timeout(1)`, in the C locale
/// so their output parses the same everywhere.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output> {
        let path = which(program).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not installed"))?;
        let mut cmd = if which("timeout").is_some() {
            let mut c = Command::new("timeout");
            c.args(["--kill-after=2", TIMEOUT_SECS]).arg(&path);
            c
        } else {
            Command::new(&path)
        };
        let out = cmd.args(args).env("LC_ALL", "C").stdin(Stdio::null()).output()?;
        Ok(Output {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn has(&self, program: &str) -> bool {
        which(program).is_some()
    }
}

/// Finds a program on `PATH`, plus the per-user install directories that
/// apps launched from a desktop session often don't have on their `PATH`.
pub fn which(program: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        for d in [".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin", ".cargo/bin", ".volta/bin"] {
            dirs.push(home.join(d));
        }
    }
    dirs.extend(["/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(program)).find(|p| is_executable(p))
}

fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
