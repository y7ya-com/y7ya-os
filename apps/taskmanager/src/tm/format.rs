//! Number formatting in Task Manager's style.

const KB: f64 = 1024.0;
const MB: f64 = KB * 1024.0;
const GB: f64 = MB * 1024.0;

/// Memory: "812.4 MB", "3.1 GB".
pub fn bytes(b: u64) -> String {
    let b = b as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{b:.0} B")
    }
}

/// Disk throughput: "0 MB/s", "0.1 MB/s", "12.3 MB/s".
pub fn disk_rate(bps: f64) -> String {
    let mb = bps / MB;
    if mb < 0.05 { "0 MB/s".into() } else { format!("{mb:.1} MB/s") }
}

/// Throughput for detail panes: "48 KB/s", "1.2 MB/s".
pub fn rate(bps: f64) -> String {
    if bps >= MB {
        format!("{:.1} MB/s", bps / MB)
    } else {
        format!("{:.0} KB/s", bps / KB)
    }
}

/// Network throughput in bits: "0 Kbps", "840 Kbps", "12.4 Mbps".
pub fn bits(bps: f64) -> String {
    let bits = bps * 8.0;
    if bits >= 1e6 { format!("{:.1} Mbps", bits / 1e6) } else { format!("{:.0} Kbps", bits / 1e3) }
}

pub fn percent(v: f32) -> String {
    if v > 0.0 && v < 0.05 { "0.1%".into() } else { format!("{v:.1}%") }
}

/// Up time as Windows shows it: "3:04:12:09" (days:hours:minutes:seconds).
pub fn uptime(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}:{:02}:{:02}", s / 86400, s / 3600 % 24, s / 60 % 60, s % 60)
}

/// Thousands separators: 22071 → "22,071".
pub fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(300 * 1024 * 1024), "300.0 MB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024 + 200 * 1024 * 1024), "3.2 GB");
        assert_eq!(disk_rate(10_000.0), "0 MB/s");
        assert_eq!(disk_rate(3.0 * MB), "3.0 MB/s");
        assert_eq!(bits(125_000.0), "1.0 Mbps");
        assert_eq!(bits(1_000.0), "8 Kbps");
        assert_eq!(percent(0.01), "0.1%");
        assert_eq!(percent(12.34), "12.3%");
        assert_eq!(uptime(90061.0), "1:01:01:01");
        assert_eq!(count(22071), "22,071");
        assert_eq!(count(7), "7");
    }
}
