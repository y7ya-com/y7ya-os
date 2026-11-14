//! Per-process sampling from `/proc/<pid>`. Each tick reads three small files
//! per process (`stat`, `statm`, `io`); things that rarely change (command
//! line, executable, owner, environment) are read once per process.

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::procfs::{cgroup_app_id, cpu_percent, parse_cmdline, parse_io, parse_proc_stat, parse_statm};

#[derive(Clone, Debug)]
pub struct Process {
    pub pid: i32,
    pub ppid: i32,
    pub name: String,
    pub cmdline: String,
    pub exe: Option<PathBuf>,
    pub uid: u32,
    pub state: char,
    pub threads: u32,
    pub start_ticks: u64,
    /// Percent of the whole machine.
    pub cpu: f32,
    /// Total CPU time used so far, in clock ticks.
    pub cpu_ticks: u64,
    /// Private (non-shared) resident memory, in bytes.
    pub mem: u64,
    /// Storage reads + writes, bytes per second.
    pub disk: f64,
    pub kernel: bool,
    /// Has a display in its environment (`WAYLAND_DISPLAY` or `DISPLAY`).
    pub gui: bool,
    /// Application id from the systemd unit the desktop launched it in.
    pub app_id: Option<String>,
}

impl Process {
    pub fn status(&self) -> &'static str {
        match self.state {
            'T' | 't' => "Suspended",
            'Z' => "Zombie",
            'D' => "Waiting",
            _ => "",
        }
    }
}

/// What we remember about a live process between ticks.
struct Known {
    start_ticks: u64,
    name: String,
    cmdline: String,
    exe: Option<PathBuf>,
    uid: u32,
    gui: bool,
    kernel: bool,
    app_id: Option<String>,
    cpu_ticks: u64,
    io_bytes: Option<u64>,
}

pub struct ProcessSampler {
    root: PathBuf,
    known: HashMap<i32, Known>,
    page_size: u64,
    last: Option<(Instant, u64)>,
    buf: String,
}

impl ProcessSampler {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64;
        Self { root: root.into(), known: HashMap::new(), page_size, last: None, buf: String::with_capacity(1024) }
    }

    /// Reads every process. `machine_ticks` is the CPU time of all CPUs
    /// together from `/proc/stat`, used to turn tick deltas into percentages.
    pub fn sample(&mut self, machine_ticks: u64, now: Instant) -> Vec<Process> {
        let (dt, dmachine) = match self.last {
            Some((t, m)) => (now.duration_since(t).as_secs_f64(), machine_ticks.saturating_sub(m)),
            None => (0.0, 0),
        };
        self.last = Some((now, machine_ticks));

        let mut out = Vec::with_capacity(self.known.len() + 16);
        let Ok(rd) = std::fs::read_dir(&self.root) else { return out };
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
            let dir = e.path();
            if !read_into(&dir.join("stat"), &mut self.buf) {
                continue;
            }
            let Some(st) = parse_proc_stat(&self.buf) else { continue };

            let fresh = self.known.get(&pid).is_none_or(|k| k.start_ticks != st.start_ticks);
            if fresh {
                self.known.insert(pid, discover(&dir, &st.comm, st.ppid, pid, st.start_ticks));
            }
            let mem = if read_into(&dir.join("statm"), &mut self.buf) {
                parse_statm(&self.buf).map_or(0, |(res, shared)| res.saturating_sub(shared) * self.page_size)
            } else {
                0
            };
            // Only readable for our own processes (or as root).
            let io = if read_into(&dir.join("io"), &mut self.buf) { parse_io(&self.buf).map(|(r, w)| r + w) } else { None };

            let k = self.known.get_mut(&pid).unwrap();
            let cpu = if fresh || dmachine == 0 { 0.0 } else { cpu_percent(st.cpu_ticks.saturating_sub(k.cpu_ticks), dmachine) };
            let disk = match (k.io_bytes, io) {
                (Some(a), Some(b)) if !fresh && dt > 0.0 => b.saturating_sub(a) as f64 / dt,
                _ => 0.0,
            };
            k.cpu_ticks = st.cpu_ticks;
            k.io_bytes = io;
            out.push(Process {
                pid,
                ppid: st.ppid,
                name: k.name.clone(),
                cmdline: k.cmdline.clone(),
                exe: k.exe.clone(),
                uid: k.uid,
                state: st.state,
                threads: st.threads,
                start_ticks: st.start_ticks,
                cpu,
                cpu_ticks: st.cpu_ticks,
                mem,
                disk,
                kernel: k.kernel,
                gui: k.gui,
                app_id: k.app_id.clone(),
            });
        }
        let alive: std::collections::HashSet<i32> = out.iter().map(|p| p.pid).collect();
        self.known.retain(|pid, _| alive.contains(pid));
        out
    }
}

fn read_into(path: &Path, buf: &mut String) -> bool {
    buf.clear();
    std::fs::File::open(path).and_then(|mut f| f.read_to_string(buf)).is_ok()
}

/// The once-per-process facts.
fn discover(dir: &Path, comm: &str, ppid: i32, pid: i32, start_ticks: u64) -> Known {
    let kernel = pid == 2 || ppid == 2;
    let cmdline = if kernel { String::new() } else { std::fs::read(dir.join("cmdline")).map(|b| parse_cmdline(&b)).unwrap_or_default() };
    let exe = std::fs::read_link(dir.join("exe")).ok().map(|p| {
        let s = p.to_string_lossy();
        s.strip_suffix(" (deleted)").map(PathBuf::from).unwrap_or(p)
    });
    let uid = std::fs::metadata(dir).map(|m| m.uid()).unwrap_or(0);
    let gui = !kernel && std::fs::read(dir.join("environ")).is_ok_and(|env| has_display(&env));
    let app_id = if kernel { None } else { std::fs::read_to_string(dir.join("cgroup")).ok().and_then(|s| cgroup_app_id(&s)) };
    Known {
        start_ticks,
        name: display_name(comm, exe.as_deref(), &cmdline),
        cmdline,
        exe,
        uid,
        gui,
        kernel,
        app_id,
        cpu_ticks: 0,
        io_bytes: None,
    }
}

fn has_display(environ: &[u8]) -> bool {
    environ.split(|&b| b == 0).any(|v| v.starts_with(b"WAYLAND_DISPLAY=") || v.starts_with(b"DISPLAY="))
}

/// `comm` is cut at 15 bytes; prefer the executable's (or argv[0]'s) full
/// name when `comm` is a prefix of it.
pub fn display_name(comm: &str, exe: Option<&Path>, cmdline: &str) -> String {
    let argv0 = cmdline.split_whitespace().next().map(Path::new);
    for p in [exe, argv0].into_iter().flatten() {
        if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
            if n.len() > comm.len() && n.starts_with(comm) {
                return n.to_string();
            }
        }
    }
    comm.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_undo_comm_truncation() {
        let exe = Path::new("/usr/lib/xdg-desktop-portal-kde");
        assert_eq!(display_name("xdg-desktop-por", Some(exe), ""), "xdg-desktop-portal-kde");
        assert_eq!(display_name("kworker/0:1", None, ""), "kworker/0:1");
        assert_eq!(display_name("Isolated Web Co", Some(Path::new("/usr/lib/firefox/firefox")), ""), "Isolated Web Co");
        assert_eq!(display_name("plasma-browser-", None, "/usr/bin/plasma-browser-integration-host x"), "plasma-browser-integration-host");
    }

    #[test]
    fn display_detection() {
        assert!(has_display(b"HOME=/h\0WAYLAND_DISPLAY=wayland-1\0"));
        assert!(has_display(b"DISPLAY=:0\0"));
        assert!(!has_display(b"HOME=/h\0XDISPLAY=x\0"));
    }

    /// Samples a fake `/proc` with one process twice and checks the deltas.
    #[test]
    fn samples_fixture_proc() {
        let root = std::env::temp_dir().join(format!("tm-proc-{}", std::process::id()));
        let dir = root.join("4242");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |ticks: u64, io: u64| {
            let stat = format!("4242 (sleep) S 1 4242 4242 0 -1 0 0 0 0 0 {ticks} 0 0 0 20 0 1 0 5000 0 0");
            std::fs::write(dir.join("stat"), stat).unwrap();
            std::fs::write(dir.join("statm"), "500 300 100 1 0 50 0").unwrap();
            std::fs::write(dir.join("io"), format!("read_bytes: {io}\nwrite_bytes: 0\n")).unwrap();
        };
        std::fs::write(dir.join("cmdline"), b"sleep\x00100\x00").unwrap();
        write(100, 0);
        let mut s = ProcessSampler::new(&root);
        let t0 = Instant::now();
        let first = s.sample(10_000, t0);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].cpu, 0.0);
        assert_eq!(first[0].cmdline, "sleep 100");
        assert_eq!(first[0].mem, 200 * s.page_size);

        write(150, 1_000_000);
        let second = s.sample(10_200, t0 + std::time::Duration::from_secs(2));
        // 50 of the machine's 200 ticks.
        assert!((second[0].cpu - 25.0).abs() < 1e-3);
        assert!((second[0].disk - 500_000.0).abs() < 1.0);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
