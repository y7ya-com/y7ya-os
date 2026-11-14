//! Commands from other processes over a Unix socket at
//! `$XDG_RUNTIME_DIR/y7-shell.sock` (`Y7_SHELL_SOCKET` overrides it).
//!
//! The protocol is one line per connection: the client writes a command and
//! reads back `ok` or `error: <why>`. `y7-shell msg <command>` does this.
//!
//! | Command | Effect |
//! |---|---|
//! | `spotlight` | Toggle Spotlight |
//! | `control-center` | Toggle Control Center |
//! | `notification-center` | Toggle Notification Center |
//! | `lock` | Lock the screen |
//! | `dismiss` | Close whatever panel or menu is open |
//! | `dnd on\|off\|toggle` | Do Not Disturb |
//! | `ping` | Check that the shell is running |

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Spotlight,
    ControlCenter,
    NotificationCenter,
    Lock,
    Dismiss,
    /// `None` toggles.
    Dnd(Option<bool>),
    Ping,
}

pub const USAGE: &str = "commands: spotlight, control-center, notification-center, lock, dismiss, dnd on|off|toggle, ping";

impl std::str::FromStr for Command {
    type Err = String;

    fn from_str(s: &str) -> Result<Command, String> {
        let words: Vec<&str> = s.split_whitespace().collect();
        Ok(match words.as_slice() {
            ["spotlight"] => Command::Spotlight,
            ["control-center"] => Command::ControlCenter,
            ["notification-center"] => Command::NotificationCenter,
            ["lock"] => Command::Lock,
            ["dismiss"] => Command::Dismiss,
            ["dnd", "on"] => Command::Dnd(Some(true)),
            ["dnd", "off"] => Command::Dnd(Some(false)),
            ["dnd"] | ["dnd", "toggle"] => Command::Dnd(None),
            ["ping"] => Command::Ping,
            _ => return Err(format!("unknown command {s:?}; {USAGE}")),
        })
    }
}

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("Y7_SHELL_SOCKET") {
        return p.into();
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("y7-shell.sock")
}

/// Sends one command and returns the reply.
pub fn send(path: &Path, command: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    writeln!(stream, "{command}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply.trim().to_string())
}

/// Listens on `path` and calls `handle` for each command (on a background
/// thread). Fails if another shell is already listening there.
pub fn serve(path: &Path, handle: impl Fn(Command) + Send + 'static) -> std::io::Result<()> {
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "y7-shell is already running"));
        }
        // Left over from a shell that died.
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::thread::Builder::new().name("ipc".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            let Ok(mut writer) = stream.try_clone() else { continue };
            if BufReader::new(stream).read_line(&mut line).is_err() {
                continue;
            }
            let reply = match line.trim().parse::<Command>() {
                Ok(cmd) => {
                    handle(cmd);
                    "ok".to_string()
                }
                Err(e) => format!("error: {e}"),
            };
            let _ = writeln!(writer, "{reply}");
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn parses_commands() {
        assert_eq!("spotlight".parse(), Ok(Command::Spotlight));
        assert_eq!(" control-center \n".parse(), Ok(Command::ControlCenter));
        assert_eq!("notification-center".parse(), Ok(Command::NotificationCenter));
        assert_eq!("dnd on".parse(), Ok(Command::Dnd(Some(true))));
        assert_eq!("dnd".parse(), Ok(Command::Dnd(None)));
        assert!("launch rockets".parse::<Command>().is_err());
        assert!("".parse::<Command>().is_err());
    }

    #[test]
    fn socket_round_trip() {
        let path = std::env::temp_dir().join(format!("y7-shell-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let (tx, rx) = mpsc::channel();
        serve(&path, move |cmd| tx.send(cmd).unwrap()).unwrap();
        assert_eq!(send(&path, "spotlight").unwrap(), "ok");
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), Command::Spotlight);
        assert!(send(&path, "bogus").unwrap().starts_with("error: unknown command"));
        assert_eq!(send(&path, "lock").unwrap(), "ok");
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), Command::Lock);
        // A second shell can't take over a live socket.
        assert_eq!(serve(&path, |_| {}).unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
        std::fs::remove_file(&path).unwrap();
    }
}
