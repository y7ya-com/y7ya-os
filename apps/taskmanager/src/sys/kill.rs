//! Ending processes: SIGTERM first, then SIGKILL for whatever is still
//! running after a grace period. A process is identified by its pid *and*
//! start time, so a recycled pid is never signalled by mistake.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::procfs::parse_proc_stat;

pub const GRACE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub pid: i32,
    pub start_ticks: u64,
}

pub struct Reaper {
    root: PathBuf,
    grace: Duration,
    pending: Vec<(Target, Instant)>,
}

impl Reaper {
    pub fn new(grace: Duration) -> Self {
        Self { root: "/proc".into(), grace, pending: Vec::new() }
    }

    /// Whether the process is still running (zombies count as gone).
    pub fn alive(&self, t: Target) -> bool {
        let stat = std::fs::read_to_string(self.root.join(t.pid.to_string()).join("stat")).ok();
        stat.and_then(|s| parse_proc_stat(&s)).is_some_and(|s| s.start_ticks == t.start_ticks && s.state != 'Z')
    }

    /// Sends SIGTERM to each target and schedules SIGKILL for stragglers.
    pub fn terminate(&mut self, targets: &[Target], now: Instant) -> Result<(), String> {
        let mut err = None;
        for &t in targets {
            if !self.alive(t) {
                continue;
            }
            if unsafe { libc::kill(t.pid, libc::SIGTERM) } != 0 {
                let e = std::io::Error::last_os_error();
                err = Some(match e.raw_os_error() {
                    Some(libc::EPERM) => "You don't have permission to end this process.".to_string(),
                    _ => e.to_string(),
                });
                continue;
            }
            if !self.pending.iter().any(|(p, _)| *p == t) {
                self.pending.push((t, now + self.grace));
            }
        }
        err.map_or(Ok(()), Err)
    }

    pub fn is_pending(&self, pid: i32) -> bool {
        self.pending.iter().any(|(t, _)| t.pid == pid)
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.iter().map(|(_, d)| *d).min()
    }

    /// SIGKILLs targets whose grace period ran out; forgets the ones that exited.
    pub fn tick(&mut self, now: Instant) {
        let pending = std::mem::take(&mut self.pending);
        for (t, deadline) in pending {
            if !self.alive(t) {
                continue;
            }
            if now >= deadline {
                unsafe { libc::kill(t.pid, libc::SIGKILL) };
            } else {
                self.pending.push((t, deadline));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};

    fn target(child: &Child) -> Target {
        let s = std::fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap();
        Target { pid: child.id() as i32, start_ticks: parse_proc_stat(&s).unwrap().start_ticks }
    }

    fn wait_exit(child: &mut Child, within: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < within {
            if child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn sigterm_ends_a_cooperative_process() {
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        let t = target(&child);
        let mut r = Reaper::new(Duration::from_millis(200));
        r.terminate(&[t], Instant::now()).unwrap();
        assert!(wait_exit(&mut child, Duration::from_secs(5)));
        r.tick(Instant::now());
        assert!(r.next_deadline().is_none());
    }

    #[test]
    fn escalates_to_sigkill() {
        // Ignored signals stay ignored across exec, so this sleep shrugs off SIGTERM.
        let mut child = Command::new("sh").args(["-c", "trap '' TERM; exec sleep 60"]).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let t = target(&child);
        let mut r = Reaper::new(Duration::from_millis(200));
        let start = Instant::now();
        r.terminate(&[t], start).unwrap();
        assert!(!wait_exit(&mut child, Duration::from_millis(150)), "SIGTERM should be ignored");
        assert!(r.is_pending(t.pid));
        r.tick(start + Duration::from_millis(250));
        assert!(wait_exit(&mut child, Duration::from_secs(5)));
    }

    #[test]
    fn recycled_pids_are_left_alone() {
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        let mut t = target(&child);
        t.start_ticks += 1;
        let mut r = Reaper::new(GRACE);
        r.terminate(&[t], Instant::now()).unwrap();
        assert!(!wait_exit(&mut child, Duration::from_millis(200)));
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
