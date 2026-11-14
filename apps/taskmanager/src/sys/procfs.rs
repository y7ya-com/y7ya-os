//! Parsers for the /proc text formats we sample. They take file contents
//! rather than paths, so the tests can feed them fixture text.

use std::collections::HashMap;

/// The fields we use from `/proc/<pid>/stat`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcStat {
    pub pid: i32,
    pub comm: String,
    pub state: char,
    pub ppid: i32,
    /// User + system time, in clock ticks.
    pub cpu_ticks: u64,
    pub threads: u32,
    /// Start time after boot, in clock ticks; with the pid it identifies a process.
    pub start_ticks: u64,
}

pub fn parse_proc_stat(s: &str) -> Option<ProcStat> {
    // `comm` may hold spaces and parentheses, so split around the last ')'.
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let pid = s[..open].trim().parse().ok()?;
    let comm = s[open + 1..close].to_string();
    let f: Vec<&str> = s[close + 1..].split_whitespace().collect();
    // f[0] is field 3 (state) of proc(5).
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok());
    Some(ProcStat {
        pid,
        comm,
        state: f.first()?.chars().next()?,
        ppid: f.get(1)?.parse().ok()?,
        cpu_ticks: num(11)? + num(12)?,
        threads: num(17)? as u32,
        start_ticks: num(19)?,
    })
}

/// `/proc/<pid>/statm`: (resident, shared) in pages. Resident minus shared is
/// the process's private memory, the closest match to Windows' "Memory".
pub fn parse_statm(s: &str) -> Option<(u64, u64)> {
    let mut it = s.split_whitespace().skip(1).map(|v| v.parse::<u64>().ok());
    Some((it.next()??, it.next()??))
}

/// `/proc/<pid>/io`: bytes actually read from and written to storage.
pub fn parse_io(s: &str) -> Option<(u64, u64)> {
    let (mut r, mut w) = (None, None);
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("read_bytes:") {
            r = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("write_bytes:") {
            w = v.trim().parse().ok();
        }
    }
    Some((r?, w?))
}

/// `/proc/<pid>/cmdline`: NUL-separated arguments joined with spaces.
pub fn parse_cmdline(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    s.split('\0').filter(|a| !a.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Aggregate CPU time from a `cpu` line of `/proc/stat`, in clock ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl CpuTimes {
    /// Guest time is already counted in user time, so it's left out.
    pub fn total(&self) -> u64 {
        self.user + self.nice + self.system + self.idle + self.iowait + self.irq + self.softirq + self.steal
    }

    pub fn idle_all(&self) -> u64 {
        self.idle + self.iowait
    }
}

fn parse_cpu_line(line: &str) -> Option<CpuTimes> {
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let g = |i: usize| v.get(i).copied().unwrap_or(0);
    (v.len() >= 4).then(|| CpuTimes {
        user: g(0),
        nice: g(1),
        system: g(2),
        idle: g(3),
        iowait: g(4),
        irq: g(5),
        softirq: g(6),
        steal: g(7),
    })
}

/// `/proc/stat`: the machine total and one entry per logical CPU.
pub fn parse_stat(s: &str) -> (CpuTimes, Vec<CpuTimes>) {
    let mut total = CpuTimes::default();
    let mut cores = Vec::new();
    for line in s.lines() {
        if line.starts_with("cpu ") {
            total = parse_cpu_line(line).unwrap_or_default();
        } else if line.starts_with("cpu") {
            cores.extend(parse_cpu_line(line));
        }
    }
    (total, cores)
}

/// Busy share of `cur - prev`, in percent.
pub fn utilization(prev: &CpuTimes, cur: &CpuTimes) -> f32 {
    let total = cur.total().saturating_sub(prev.total());
    if total == 0 {
        return 0.0;
    }
    let idle = cur.idle_all().saturating_sub(prev.idle_all());
    (100.0 * (total - idle.min(total)) as f64 / total as f64) as f32
}

/// A process's CPU use as a share of the whole machine (all logical CPUs),
/// like Windows Task Manager: `proc_ticks` of the process over `machine_ticks`
/// summed across every CPU in the same interval.
pub fn cpu_percent(proc_ticks: u64, machine_ticks: u64) -> f32 {
    if machine_ticks == 0 {
        return 0.0;
    }
    ((100.0 * proc_ticks as f64 / machine_ticks as f64) as f32).clamp(0.0, 100.0)
}

/// `/proc/meminfo`, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MemInfo {
    pub total: u64,
    pub free: u64,
    pub available: u64,
    pub buffers: u64,
    pub cached: u64,
    pub shmem: u64,
    pub sreclaimable: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub committed: u64,
    pub commit_limit: u64,
    pub dirty: u64,
}

impl MemInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    /// Page cache that could be dropped (Windows calls this "Cached").
    pub fn cache(&self) -> u64 {
        (self.cached + self.buffers + self.sreclaimable).saturating_sub(self.shmem)
    }
}

pub fn parse_meminfo(s: &str) -> MemInfo {
    let mut m = MemInfo::default();
    for line in s.lines() {
        let Some((key, rest)) = line.split_once(':') else { continue };
        let kb: u64 = rest.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let b = kb * 1024;
        match key {
            "MemTotal" => m.total = b,
            "MemFree" => m.free = b,
            "MemAvailable" => m.available = b,
            "Buffers" => m.buffers = b,
            "Cached" => m.cached = b,
            "Shmem" => m.shmem = b,
            "SReclaimable" => m.sreclaimable = b,
            "SwapTotal" => m.swap_total = b,
            "SwapFree" => m.swap_free = b,
            "Committed_AS" => m.committed = b,
            "CommitLimit" => m.commit_limit = b,
            "Dirty" => m.dirty = b,
            _ => {}
        }
    }
    m
}

/// Cumulative counters of one block device from `/proc/diskstats`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskCounters {
    pub name: String,
    pub reads: u64,
    pub sectors_read: u64,
    pub read_ms: u64,
    pub writes: u64,
    pub sectors_written: u64,
    pub write_ms: u64,
    /// Milliseconds spent with I/O in flight ("active time").
    pub io_ms: u64,
}

pub fn parse_diskstats(s: &str) -> Vec<DiskCounters> {
    s.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let n = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok());
            Some(DiskCounters {
                name: f.get(2)?.to_string(),
                reads: n(3)?,
                sectors_read: n(5)?,
                read_ms: n(6)?,
                writes: n(7)?,
                sectors_written: n(9)?,
                write_ms: n(10)?,
                io_ms: n(12)?,
            })
        })
        .collect()
}

/// Cumulative byte counters of one interface from `/proc/net/dev`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetCounters {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

pub fn parse_net_dev(s: &str) -> Vec<NetCounters> {
    s.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            (f.len() >= 9).then(|| NetCounters { name: name.trim().to_string(), rx_bytes: f[0], tx_bytes: f[8] })
        })
        .collect()
}

/// uid → user name from `/etc/passwd`.
pub fn parse_passwd(s: &str) -> HashMap<u32, String> {
    s.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            Some((f.get(2)?.parse().ok()?, f.first()?.to_string()))
        })
        .collect()
}

/// Seconds since boot, from `/proc/uptime`.
pub fn parse_uptime(s: &str) -> f64 {
    s.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

/// Open file handles system-wide, from `/proc/sys/fs/file-nr`.
pub fn parse_file_nr(s: &str) -> u64 {
    s.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// The first `app-…` systemd unit in a `/proc/<pid>/cgroup` path, reduced to
/// the application id: desktops launch apps as `app[-<launcher>]-<id>@<rand>.service`
/// or `app[-<launcher>]-<id>-<rand>.scope` (systemd's XDG convention).
pub fn cgroup_app_id(s: &str) -> Option<String> {
    let path = s.lines().find_map(|l| l.strip_prefix("0::"))?;
    let unit = path.split('/').find(|c| c.starts_with("app-") && (c.ends_with(".service") || c.ends_with(".scope")))?;
    let unit = unit.strip_prefix("app-")?;
    let id = if let Some(base) = unit.strip_suffix(".service") {
        base.split('@').next()?
    } else {
        let base = unit.strip_suffix(".scope")?;
        // Drop the trailing "-<random or pid>".
        base.rsplit_once('-').map_or(base, |(id, _)| id)
    };
    // Launcher prefixes some desktops add (e.g. "app-gnome-org.gnome.Nautilus-123.scope").
    let id = ["gnome-", "kde-", "flatpak-"].iter().find_map(|p| id.strip_prefix(p)).unwrap_or(id);
    Some(id.replace("\\x2d", "-")).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_stat_with_awkward_comm() {
        let s = "1743 (Web Content (x)) S 1702 1702 1702 0 -1 4194560 26391 0 0 0 1530 220 0 0 20 0 27 0 2319086 6123520 997 18446744073709551615";
        let p = parse_proc_stat(s).unwrap();
        assert_eq!(p.pid, 1743);
        assert_eq!(p.comm, "Web Content (x)");
        assert_eq!(p.state, 'S');
        assert_eq!(p.ppid, 1702);
        assert_eq!(p.cpu_ticks, 1750);
        assert_eq!(p.threads, 27);
        assert_eq!(p.start_ticks, 2319086);
        assert!(parse_proc_stat("garbage").is_none());
    }

    #[test]
    fn statm_io_cmdline() {
        assert_eq!(parse_statm("1560 985 958 7 0 150 0\n"), Some((985, 958)));
        let io = "rchar: 2884\nwchar: 0\nsyscr: 7\nsyscw: 0\nread_bytes: 4096\nwrite_bytes: 8192\ncancelled_write_bytes: 0\n";
        assert_eq!(parse_io(io), Some((4096, 8192)));
        assert_eq!(parse_cmdline(b"/usr/bin/sleep\x00100\x00"), "/usr/bin/sleep 100");
    }

    #[test]
    fn stat_and_utilization() {
        let a = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\ncpu1 50 0 25 400 25 0 0 0 0 0\nintr 1\n";
        let b = "cpu  160 0 90 900 50 0 0 0 0 0\ncpu0 80 0 45 450 25 0 0 0 0 0\ncpu1 80 0 45 450 25 0 0 0 0 0\nintr 1\n";
        let (ta, ca) = parse_stat(a);
        let (tb, _) = parse_stat(b);
        assert_eq!(ca.len(), 2);
        assert_eq!(ta.total(), 1000);
        // 200 ticks passed, 100 of them idle.
        assert_eq!(tb.total() - ta.total(), 200);
        assert!((utilization(&ta, &tb) - 50.0).abs() < 1e-4);
        assert_eq!(utilization(&ta, &ta), 0.0);
    }

    #[test]
    fn cpu_percent_is_share_of_the_machine() {
        // One process busy on one of 8 CPUs for the whole interval: 12.5%.
        let interval_ticks_per_cpu = 100;
        assert!((cpu_percent(100, 8 * interval_ticks_per_cpu) - 12.5).abs() < 1e-4);
        assert_eq!(cpu_percent(0, 800), 0.0);
        assert_eq!(cpu_percent(5, 0), 0.0);
        // Never above 100%, even with sampling skew.
        assert_eq!(cpu_percent(900, 800), 100.0);
    }

    #[test]
    fn meminfo() {
        let s = "MemTotal:       32387048 kB\nMemFree:         1417260 kB\nMemAvailable:   20871952 kB\nBuffers:          122772 kB\nCached:         19480460 kB\nShmem:            417080 kB\nSReclaimable:     739088 kB\nSwapTotal:      106954740 kB\nSwapFree:       103966668 kB\nCommitLimit:    123148264 kB\nCommitted_AS:   21254484 kB\n";
        let m = parse_meminfo(s);
        assert_eq!(m.total, 32387048 * 1024);
        assert_eq!(m.used(), (32387048 - 20871952) * 1024);
        assert_eq!(m.cache(), (122772 + 19480460 + 739088 - 417080) * 1024);
        assert_eq!(m.committed, 21254484 * 1024);
    }

    #[test]
    fn diskstats_and_net_dev() {
        let d = parse_diskstats(" 259       0 nvme0n1 1237708 379292 104633920 243261 2450278 1345720 235245299 16273324 0 400790 16561117 0 0 0 0 114440 44531\n   7 0 loop0 1 2 3\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "nvme0n1");
        assert_eq!(d[0].sectors_read, 104633920);
        assert_eq!(d[0].sectors_written, 235245299);
        assert_eq!(d[0].io_ms, 400790);
        let n = parse_net_dev(
            "Inter-|   Receive |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes\n    lo: 171 78 0 0 0 0 0 0 171 78 0 0 0 0 0 0\nwlp0s20f3: 17498983993 20889188 0 1 0 0 0 0 24781332541 3400823 0 15 0 0 0 0\n",
        );
        assert_eq!(n.len(), 2);
        assert_eq!(n[1], NetCounters { name: "wlp0s20f3".into(), rx_bytes: 17498983993, tx_bytes: 24781332541 });
    }

    #[test]
    fn passwd_uptime_file_nr() {
        let p = parse_passwd("root:x:0:0::/root:/bin/bash\ny7ya:x:1000:1000:Y:/home/y7ya:/bin/zsh\nbroken\n");
        assert_eq!(p[&0], "root");
        assert_eq!(p[&1000], "y7ya");
        assert_eq!(p.len(), 2);
        assert_eq!(parse_uptime("23190.87 123264.81\n"), 23190.87);
        assert_eq!(parse_file_nr("22071\t0\t9223372036854775807\n"), 22071);
    }

    #[test]
    fn app_ids_from_cgroups() {
        let svc = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-dolphin@26b49a08bb0346b599df959eb06688da.service\n";
        assert_eq!(cgroup_app_id(svc).as_deref(), Some("dolphin"));
        let scope = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.kde.konsole-70400.scope/tab(70418).scope\n";
        assert_eq!(cgroup_app_id(scope).as_deref(), Some("org.kde.konsole"));
        let gnome = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-gnome-org.gnome.Nautilus-4242.scope\n";
        assert_eq!(cgroup_app_id(gnome).as_deref(), Some("org.gnome.Nautilus"));
        let escaped = "0::/user.slice/user@1000.service/app.slice/app-com.github.wwmm.easyeffects\\x2dservice@autostart.service\n";
        assert_eq!(cgroup_app_id(escaped).as_deref(), Some("com.github.wwmm.easyeffects-service"));
        let session = "0::/user.slice/user-1000.slice/user@1000.service/session.slice/plasma-plasmashell.service\n";
        assert_eq!(cgroup_app_id(session), None);
    }
}
