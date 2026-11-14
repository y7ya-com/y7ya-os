//! Machine-wide performance counters (CPU, memory, disks, network, GPU) from
//! `/proc` and `/sys`, with a short history of each for the graphs.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::procfs::{
    CpuTimes, DiskCounters, MemInfo, NetCounters, parse_diskstats, parse_file_nr, parse_meminfo, parse_net_dev, parse_stat,
    parse_uptime, utilization,
};

/// Samples kept per graph: a minute at the 1 Hz sampling rate.
pub const HISTORY: usize = 60;

#[derive(Clone, Default)]
pub struct History(VecDeque<f32>);

impl History {
    pub fn push(&mut self, v: f32) {
        if self.0.len() == HISTORY {
            self.0.pop_front();
        }
        self.0.push_back(v);
    }

    pub fn values(&self) -> Vec<f32> {
        self.0.iter().copied().collect()
    }

    pub fn last(&self) -> f32 {
        self.0.back().copied().unwrap_or(0.0)
    }

    pub fn max(&self) -> f32 {
        self.0.iter().copied().fold(0.0, f32::max)
    }
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    read(path).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn read_num(path: impl AsRef<Path>) -> Option<u64> {
    read_trim(path)?.parse().ok()
}

pub struct CpuInfo {
    pub model: String,
    pub base_ghz: Option<f32>,
    pub sockets: usize,
    pub cores: usize,
    pub logical: usize,
    /// ("L1 cache", bytes), summed over all cache instances.
    pub caches: Vec<(String, u64)>,
    pub virtualization: bool,
}

pub struct Cpu {
    pub info: CpuInfo,
    pub util: History,
    pub speed_ghz: f32,
    prev: Option<CpuTimes>,
}

pub struct Memory {
    pub info: MemInfo,
    /// Percent in use.
    pub used: History,
}

pub struct Disk {
    pub name: String,
    pub model: String,
    pub kind: &'static str,
    pub capacity: u64,
    /// Percent of time with I/O in flight.
    pub active: History,
    /// Bytes per second.
    pub read: History,
    pub write: History,
    pub response_ms: f32,
    prev: Option<DiskCounters>,
}

pub struct Net {
    pub name: String,
    pub wireless: bool,
    pub link_mbps: Option<u32>,
    pub ipv4: Option<String>,
    /// Bytes per second.
    pub rx: History,
    pub tx: History,
    prev: Option<NetCounters>,
}

enum GpuSource {
    /// amdgpu's `gpu_busy_percent`.
    BusyPercent(PathBuf),
    /// Intel (xe, i915): milliseconds the render engine spent idle.
    IdleResidency(PathBuf, Option<u64>),
}

pub struct Gpu {
    pub name: String,
    pub driver: String,
    pub util: History,
    /// (used, total) dedicated memory, where the driver reports it.
    pub vram: Option<(u64, u64)>,
    source: GpuSource,
    device: PathBuf,
}

pub struct System {
    pub cpu: Cpu,
    pub mem: Memory,
    pub disks: Vec<Disk>,
    pub nets: Vec<Net>,
    pub gpu: Option<Gpu>,
    pub uptime: f64,
    pub handles: u64,
    /// CPU ticks of all CPUs together, as of the last sample.
    pub machine_ticks: u64,
    last: Option<Instant>,
}

impl System {
    pub fn new() -> Self {
        Self {
            cpu: Cpu { info: cpu_info(), util: History::default(), speed_ghz: 0.0, prev: None },
            mem: Memory { info: MemInfo::default(), used: History::default() },
            disks: find_disks(),
            nets: find_nets(),
            gpu: find_gpu(),
            uptime: 0.0,
            handles: 0,
            machine_ticks: 0,
            last: None,
        }
    }

    pub fn sample(&mut self, now: Instant) {
        let dt = self.last.map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last = Some(now);

        if let Some(s) = read("/proc/stat") {
            let (total, _) = parse_stat(&s);
            if let Some(prev) = self.cpu.prev {
                self.cpu.util.push(utilization(&prev, &total));
            }
            self.cpu.prev = Some(total);
            self.machine_ticks = total.total();
        }
        self.cpu.speed_ghz = current_speed_ghz(self.cpu.info.logical).unwrap_or(self.cpu.speed_ghz);

        if let Some(s) = read("/proc/meminfo") {
            let m = parse_meminfo(&s);
            self.mem.info = m;
            self.mem.used.push(if m.total > 0 { 100.0 * m.used() as f32 / m.total as f32 } else { 0.0 });
        }

        let stats = read("/proc/diskstats").map(|s| parse_diskstats(&s)).unwrap_or_default();
        for d in &mut self.disks {
            let Some(cur) = stats.iter().find(|c| c.name == d.name) else { continue };
            if let (Some(p), true) = (&d.prev, dt > 0.0) {
                let ms = dt * 1000.0;
                d.active.push((100.0 * cur.io_ms.saturating_sub(p.io_ms) as f64 / ms).min(100.0) as f32);
                d.read.push((cur.sectors_read.saturating_sub(p.sectors_read) * 512) as f32 / dt as f32);
                d.write.push((cur.sectors_written.saturating_sub(p.sectors_written) * 512) as f32 / dt as f32);
                let ops = (cur.reads + cur.writes).saturating_sub(p.reads + p.writes);
                let wait = (cur.read_ms + cur.write_ms).saturating_sub(p.read_ms + p.write_ms);
                d.response_ms = if ops > 0 { wait as f32 / ops as f32 } else { 0.0 };
            }
            d.prev = Some(cur.clone());
        }

        let counters = read("/proc/net/dev").map(|s| parse_net_dev(&s)).unwrap_or_default();
        let addrs = ipv4_addrs();
        for n in &mut self.nets {
            let Some(cur) = counters.iter().find(|c| c.name == n.name) else { continue };
            if let (Some(p), true) = (&n.prev, dt > 0.0) {
                n.rx.push(cur.rx_bytes.saturating_sub(p.rx_bytes) as f32 / dt as f32);
                n.tx.push(cur.tx_bytes.saturating_sub(p.tx_bytes) as f32 / dt as f32);
            }
            n.prev = Some(cur.clone());
            n.ipv4 = addrs.iter().find(|(name, _)| *name == n.name).map(|(_, a)| a.clone());
            n.link_mbps = read_num(format!("/sys/class/net/{}/speed", n.name)).map(|v| v as u32).filter(|v| *v > 0 && *v < 1_000_000);
        }

        if let Some(g) = &mut self.gpu {
            match &mut g.source {
                GpuSource::BusyPercent(p) => {
                    if let Some(v) = read_num(&*p) {
                        g.util.push(v as f32);
                    }
                }
                GpuSource::IdleResidency(p, prev) => {
                    let cur = read_num(&*p);
                    if let (Some(a), Some(b), true) = (*prev, cur, dt > 0.0) {
                        let idle = b.saturating_sub(a) as f64 / (dt * 1000.0);
                        g.util.push((100.0 * (1.0 - idle)).clamp(0.0, 100.0) as f32);
                    }
                    *prev = cur;
                }
            }
            let used = read_num(g.device.join("mem_info_vram_used"));
            let total = read_num(g.device.join("mem_info_vram_total"));
            g.vram = used.zip(total);
        }

        self.uptime = read("/proc/uptime").map_or(0.0, |s| parse_uptime(&s));
        self.handles = read("/proc/sys/fs/file-nr").map_or(0, |s| parse_file_nr(&s));
    }
}

fn cpu_info() -> CpuInfo {
    let text = read("/proc/cpuinfo").unwrap_or_default();
    let mut model = String::new();
    let mut sockets = std::collections::HashSet::new();
    let mut cores = std::collections::HashSet::new();
    let mut logical = 0;
    let mut phys = String::new();
    let mut virt = false;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "processor" => logical += 1,
            "model name" if model.is_empty() => model = v.to_string(),
            "physical id" => {
                phys = v.to_string();
                sockets.insert(v.to_string());
            }
            "core id" => {
                cores.insert((phys.clone(), v.to_string()));
            }
            "flags" if !virt => virt = v.split_whitespace().any(|f| f == "vmx" || f == "svm"),
            _ => {}
        }
    }
    let logical = logical.max(1);
    let base_khz = read_num("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency")
        .or_else(|| read_num("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq"));
    CpuInfo {
        model: if model.is_empty() { "Processor".into() } else { model },
        base_ghz: base_khz.map(|k| k as f32 / 1e6),
        sockets: sockets.len().max(1),
        cores: if cores.is_empty() { logical } else { cores.len() },
        logical,
        caches: cache_sizes(logical),
        virtualization: virt,
    }
}

/// Total size per cache level, counting each shared cache instance once.
fn cache_sizes(logical: usize) -> Vec<(String, u64)> {
    let mut seen = std::collections::HashSet::new();
    let mut totals = std::collections::BTreeMap::new();
    for cpu in 0..logical {
        for idx in 0..8 {
            let dir = PathBuf::from(format!("/sys/devices/system/cpu/cpu{cpu}/cache/index{idx}"));
            let Some(level) = read_trim(dir.join("level")) else { break };
            let shared = read_trim(dir.join("shared_cpu_list")).unwrap_or_default();
            let kind = read_trim(dir.join("type")).unwrap_or_default();
            if !seen.insert((level.clone(), kind, shared)) {
                continue;
            }
            let size = read_trim(dir.join("size")).and_then(|s| parse_size(&s)).unwrap_or(0);
            *totals.entry(level).or_insert(0u64) += size;
        }
    }
    totals.into_iter().map(|(l, b)| (format!("L{l} cache"), b)).collect()
}

/// "48K" / "2048K" / "24M" → bytes.
fn parse_size(s: &str) -> Option<u64> {
    let (num, mul) = match s.chars().last()? {
        'K' => (&s[..s.len() - 1], 1024),
        'M' => (&s[..s.len() - 1], 1024 * 1024),
        _ => (s, 1),
    };
    num.parse::<u64>().ok().map(|n| n * mul)
}

fn current_speed_ghz(logical: usize) -> Option<f32> {
    let khz: Vec<u64> = (0..logical)
        .filter_map(|i| read_num(format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq")))
        .collect();
    (!khz.is_empty()).then(|| khz.iter().sum::<u64>() as f32 / khz.len() as f32 / 1e6)
}

fn find_disks() -> Vec<Disk> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/block")
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    names.retain(|n| !["loop", "ram", "zram", "dm-", "sr", "fd"].iter().any(|p| n.starts_with(p)));
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let dir = PathBuf::from("/sys/block").join(&name);
            let capacity = read_num(dir.join("size"))? * 512;
            if capacity == 0 {
                return None;
            }
            let rotational = read_num(dir.join("queue/rotational")) == Some(1);
            let kind = match (name.starts_with("nvme"), rotational) {
                (true, _) => "SSD (NVMe)",
                (_, true) => "HDD",
                _ => "SSD",
            };
            Some(Disk {
                model: read_trim(dir.join("device/model")).unwrap_or_else(|| name.clone()),
                name,
                kind,
                capacity,
                active: History::default(),
                read: History::default(),
                write: History::default(),
                response_ms: 0.0,
                prev: None,
            })
        })
        .collect()
}

/// Physical network interfaces (those backed by a device).
fn find_nets() -> Vec<Net> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/net")
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    names.retain(|n| Path::new("/sys/class/net").join(n).join("device").exists());
    names.sort();
    names
        .into_iter()
        .map(|name| Net {
            wireless: Path::new("/sys/class/net").join(&name).join("wireless").exists(),
            name,
            link_mbps: None,
            ipv4: None,
            rx: History::default(),
            tx: History::default(),
            prev: None,
        })
        .collect()
}

fn find_gpu() -> Option<Gpu> {
    let mut cards: Vec<PathBuf> = std::fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("card") && !n.contains('-')))
        .collect();
    cards.sort();
    for card in cards {
        let device = card.join("device");
        let driver = std::fs::read_link(device.join("driver"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_default();
        let source = if device.join("gpu_busy_percent").exists() {
            GpuSource::BusyPercent(device.join("gpu_busy_percent"))
        } else if let Some(p) = intel_idle_counter(&card) {
            GpuSource::IdleResidency(p, None)
        } else {
            continue;
        };
        let hex = |f: &str| read_trim(device.join(f)).and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok());
        let name = match (hex("vendor"), hex("device")) {
            (Some(v), Some(d)) => pci_name(v, d),
            _ => None,
        };
        return Some(Gpu { name: name.unwrap_or_else(|| "GPU".into()), driver, util: History::default(), vram: None, source, device });
    }
    None
}

fn intel_idle_counter(card: &Path) -> Option<PathBuf> {
    // xe: device/tile0/gt0/gtidle (the render/compute GT); i915: gt/gt0.
    let xe = card.join("device/tile0/gt0/gtidle/idle_residency_ms");
    let i915 = card.join("gt/gt0/rc6_residency_ms");
    [xe, i915].into_iter().find(|p| p.exists())
}

/// "Intel Graphics" etc. from the system's PCI id database.
fn pci_name(vendor: u32, device: u32) -> Option<String> {
    let db = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids"].iter().find_map(|p| read(p))?;
    let (v, d) = (format!("{vendor:04x}"), format!("\t{device:04x}"));
    let mut vendor_name = None;
    for line in db.lines() {
        if vendor_name.is_none() {
            if let Some(rest) = line.strip_prefix(&v) {
                vendor_name = Some(rest.trim().to_string());
            }
        } else if !line.starts_with('\t') && !line.starts_with('#') && !line.is_empty() {
            break;
        } else if let Some(rest) = line.strip_prefix(&d) {
            let model = rest.trim();
            // Prefer the marketing name in brackets: "Panther Lake [Intel Graphics]".
            if let (Some(a), Some(b)) = (model.find('['), model.rfind(']')) {
                return Some(model[a + 1..b].to_string());
            }
            let vendor = vendor_name.unwrap_or_default();
            let short = vendor.split_whitespace().next().unwrap_or("");
            return Some(format!("{short} {model}").trim().to_string());
        }
    }
    vendor_name
}

/// (interface, address) for every IPv4 address.
fn ipv4_addrs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_addr.is_null() && (*ifa.ifa_addr).sa_family as i32 == libc::AF_INET {
                let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                let name = std::ffi::CStr::from_ptr(ifa.ifa_name).to_string_lossy().to_string();
                out.push((name, ip.to_string()));
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_capped() {
        let mut h = History::default();
        for i in 0..100 {
            h.push(i as f32);
        }
        assert_eq!(h.values().len(), HISTORY);
        assert_eq!(h.last(), 99.0);
        assert_eq!(h.values()[0], 40.0);
    }

    #[test]
    fn cache_size_units() {
        assert_eq!(parse_size("48K"), Some(48 * 1024));
        assert_eq!(parse_size("24M"), Some(24 * 1024 * 1024));
        assert_eq!(parse_size("x"), None);
    }
}
