//! Where things are: the user's standard folders, mounted drives (from
//! `/proc/mounts`, with free space) and recently used files.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The standard folders, all derived from one home so that `--snapshot` can
/// point everything at a fixture directory.
#[derive(Clone, Debug)]
pub struct Places {
    pub home: PathBuf,
    pub desktop: PathBuf,
    pub documents: PathBuf,
    pub downloads: PathBuf,
    pub pictures: PathBuf,
    pub music: PathBuf,
    pub videos: PathBuf,
    /// `~/.local/share`: the trash and recently-used list live here.
    pub data_home: PathBuf,
}

impl Places {
    /// The real user's folders, honoring `user-dirs.dirs` and `XDG_DATA_HOME`.
    pub fn user() -> Places {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let or = |d: Option<PathBuf>, name: &str| d.unwrap_or_else(|| home.join(name));
        Places {
            desktop: or(dirs::desktop_dir(), "Desktop"),
            documents: or(dirs::document_dir(), "Documents"),
            downloads: or(dirs::download_dir(), "Downloads"),
            pictures: or(dirs::picture_dir(), "Pictures"),
            music: or(dirs::audio_dir(), "Music"),
            videos: or(dirs::video_dir(), "Videos"),
            data_home: or(dirs::data_dir(), ".local/share"),
            home,
        }
    }

    /// Everything under `home`, with the usual English folder names.
    pub fn rooted(home: &Path) -> Places {
        Places {
            home: home.to_path_buf(),
            desktop: home.join("Desktop"),
            documents: home.join("Documents"),
            downloads: home.join("Downloads"),
            pictures: home.join("Pictures"),
            music: home.join("Music"),
            videos: home.join("Videos"),
            data_home: home.join(".local/share"),
        }
    }

    /// The display name of a folder: "Home" folders keep the user's name like
    /// Finder, `/` is the system disk.
    pub fn display_name(&self, path: &Path, disk_name: &str) -> String {
        if path == Path::new("/") {
            return disk_name.to_string();
        }
        path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mount {
    pub device: String,
    pub path: PathBuf,
    pub fs_type: String,
    pub name: String,
    pub removable: bool,
}

/// Free and total bytes on the filesystem holding `path`.
pub fn space(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let unit = st.f_frsize as u64;
    Some((st.f_bavail as u64 * unit, st.f_blocks as u64 * unit))
}

/// Undoes the octal escapes (`\040` for space) in `/proc/mounts` fields.
fn unescape_octal(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const DISK_FS: &[&str] = &["ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3", "fuseblk", "iso9660", "udf", "hfsplus", "apfs", "zfs", "bcachefs"];
const NETWORK_FS: &[&str] = &["nfs", "nfs4", "cifs", "smb3", "fuse.sshfs", "fuse.rclone"];
/// System mount points that are part of the computer's own disk, not places.
const SYSTEM_MOUNTS: &[&str] = &["/boot", "/efi", "/home", "/var", "/usr", "/tmp", "/opt", "/srv", "/root", "/nix", "/gnu", "/snap"];

/// The drives worth showing in the sidebar, the system disk first.
pub fn parse_mounts(text: &str, labels: &dyn Fn(&str) -> Option<String>, disk_name: &str) -> Vec<Mount> {
    let mut out: Vec<Mount> = Vec::new();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        let (Some(dev), Some(path), Some(fs_type)) = (f.next(), f.next(), f.next()) else { continue };
        let (dev, path) = (unescape_octal(dev), unescape_octal(path));
        let disk = DISK_FS.contains(&fs_type) && dev.starts_with("/dev/") && !dev.starts_with("/dev/loop");
        if !disk && !NETWORK_FS.contains(&fs_type) {
            continue;
        }
        if SYSTEM_MOUNTS.iter().any(|m| path == *m || path.starts_with(&format!("{m}/"))) {
            continue;
        }
        // Btrfs subvolumes and bind mounts repeat a device: keep its first,
        // shortest mount point.
        if let Some(prev) = out.iter_mut().find(|m| m.device == dev && disk) {
            if path.len() < prev.path.as_os_str().len() {
                prev.path = PathBuf::from(&path);
            }
            continue;
        }
        let removable = path.starts_with("/run/media/") || path.starts_with("/media/");
        let name = if path == "/" {
            labels(&dev).unwrap_or_else(|| disk_name.to_string())
        } else {
            labels(&dev).unwrap_or_else(|| Path::new(&path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned()))
        };
        out.push(Mount { device: dev, path: PathBuf::from(path), fs_type: fs_type.to_string(), name, removable });
    }
    out.sort_by_key(|m| (m.path != Path::new("/"), m.removable));
    out
}

/// Filesystem labels from `/dev/disk/by-label`, keyed by device path.
fn label_of(dev: &str) -> Option<String> {
    let target = std::fs::canonicalize(dev).ok()?;
    std::fs::read_dir("/dev/disk/by-label").ok()?.flatten().find_map(|e| {
        (std::fs::canonicalize(e.path()).ok()? == target).then(|| unescape_hex(&e.file_name().to_string_lossy()))
    })
}

/// `/dev/disk/by-label` names escape spaces as `\x20`.
fn unescape_hex(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("\\x") {
        out.push_str(&rest[..i]);
        match rest.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(b) => {
                out.push(b as char);
                rest = &rest[i + 4..];
            }
            None => {
                out.push_str("\\x");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The computer's own disk name when it has no label.
pub fn disk_name() -> String {
    std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Computer".into())
}

pub fn mounts() -> Vec<Mount> {
    let text = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    parse_mounts(&text, &label_of, &disk_name())
}

/// Recently used files from the shared `recently-used.xbel`, newest first.
pub fn recents(data_home: &Path, limit: usize) -> Vec<(PathBuf, SystemTime)> {
    let Ok(text) = std::fs::read_to_string(data_home.join("recently-used.xbel")) else { return Vec::new() };
    let mut out = parse_xbel(&text);
    out.retain(|(p, _)| p.exists());
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out.dedup_by(|a, b| a.0 == b.0);
    out.truncate(limit);
    out
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[at..].find('"')?;
    Some(&tag[at..at + end])
}

pub fn parse_xbel(text: &str) -> Vec<(PathBuf, SystemTime)> {
    let mut out = Vec::new();
    for chunk in text.split("<bookmark ").skip(1) {
        let tag = &chunk[..chunk.find('>').unwrap_or(chunk.len())];
        let tag = format!(" {tag}");
        let Some(href) = attr(&tag, "href") else { continue };
        let Some(path) = href.strip_prefix("file://") else { continue };
        let path = super::trash::unescape_path(&xml_unescape(path));
        let when = attr(&tag, "modified")
            .or_else(|| attr(&tag, "visited"))
            .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
            .map(SystemTime::from)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        out.push((PathBuf::from(path), when));
    }
    out
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounts_from_proc() {
        let text = "\
proc /proc proc rw 0 0
/dev/nvme0n1p2 / btrfs rw,subvol=/@ 0 0
/dev/nvme0n1p2 /home btrfs rw,subvol=/@home 0 0
/dev/nvme0n1p1 /boot vfat rw 0 0
tmpfs /tmp tmpfs rw 0 0
/dev/sda1 /run/media/me/My\\040Stick vfat rw 0 0
/dev/loop3 /var/lib/snapd/snap/x squashfs ro 0 0
server:/share /mnt/nas nfs4 rw 0 0
/dev/sdb1 /mnt/data ext4 rw 0 0
";
        let labels = |dev: &str| (dev == "/dev/sdb1").then(|| "Archive".to_string());
        let m = parse_mounts(text, &labels, "laptop");
        let names: Vec<&str> = m.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["laptop", "nas", "Archive", "My Stick"]);
        assert!(m[3].removable && !m[0].removable);
        assert_eq!(m[3].path, PathBuf::from("/run/media/me/My Stick"));
        assert_eq!(unescape_hex("My\\x20Disk"), "My Disk");
    }

    #[test]
    fn xbel() {
        let text = r#"<?xml version="1.0"?>
<xbel version="1.0">
  <bookmark href="file:///home/me/Documents/Report%20Q3.pdf" added="2024-05-01T10:00:00Z" modified="2024-05-02T10:00:00.123Z" visited="2024-05-02T10:00:00Z">
  </bookmark>
  <bookmark href="https://example.com" modified="2024-05-03T10:00:00Z"/>
  <bookmark href="file:///tmp/a&amp;b.txt" modified="2024-05-04T10:00:00Z"/>
</xbel>"#;
        let v = parse_xbel(text);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0, PathBuf::from("/home/me/Documents/Report Q3.pdf"));
        assert_eq!(v[1].0, PathBuf::from("/tmp/a&b.txt"));
        assert!(v[1].1 > v[0].1);
    }
}
