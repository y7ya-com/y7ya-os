//! The freedesktop.org Trash (spec 1.0): `$XDG_DATA_HOME/Trash` for the home
//! filesystem, and `$topdir/.Trash/$uid` or `$topdir/.Trash-$uid` on other
//! disks. Each trashed item sits in `files/` next to an `info/<name>.trashinfo`
//! recording where it came from and when.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};

/// One trash directory (holding `files/` and `info/`).
#[derive(Clone, Debug, PartialEq)]
pub struct Trash {
    pub root: PathBuf,
    /// The filesystem root that relative `Path=` entries are relative to;
    /// `None` for the home trash, which stores absolute paths.
    pub topdir: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct Item {
    /// The item itself, inside `files/`.
    pub path: PathBuf,
    pub original: Option<PathBuf>,
    pub deleted: Option<NaiveDateTime>,
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

impl Trash {
    /// The home trash of `data_home` (normally `~/.local/share`).
    pub fn home(data_home: &Path) -> Trash {
        Trash { root: data_home.join("Trash"), topdir: None }
    }

    pub fn files(&self) -> PathBuf {
        self.root.join("files")
    }

    pub fn info(&self) -> PathBuf {
        self.root.join("info")
    }

    fn info_for(&self, name: &str) -> PathBuf {
        self.info().join(format!("{name}.trashinfo"))
    }

    fn ensure(&self) -> io::Result<()> {
        for d in [self.files(), self.info()] {
            fs::create_dir_all(&d)?;
        }
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
    }

    /// The trash for items on `topdir`'s filesystem: `.Trash/$uid` when the
    /// admin made a shared, sticky `.Trash`, otherwise `.Trash-$uid`.
    pub fn for_topdir(topdir: &Path) -> Trash {
        let shared = topdir.join(".Trash");
        let ok = fs::symlink_metadata(&shared).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink() && m.mode() & 0o1000 != 0);
        let root = if ok { shared.join(uid().to_string()) } else { topdir.join(format!(".Trash-{}", uid())) };
        Trash { root, topdir: Some(topdir.to_path_buf()) }
    }

    /// Moves `path` here, returning where it went.
    pub fn put(&self, path: &Path) -> io::Result<PathBuf> {
        self.ensure()?;
        let abs = absolute(path)?;
        let name = abs.file_name().ok_or_else(|| io::Error::other("can't trash a filesystem root"))?.to_string_lossy().into_owned();
        let stored = match &self.topdir {
            Some(top) => abs.strip_prefix(top).map(Path::to_path_buf).unwrap_or_else(|_| abs.clone()),
            None => abs.clone(),
        };
        let info = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            escape_path(&stored.to_string_lossy()),
            Local::now().format("%Y-%m-%dT%H:%M:%S")
        );
        // Claim a name by creating its .trashinfo exclusively, then move the item.
        let (stem, ext) = super::fs::split_ext(&name, abs.is_dir());
        for n in 1.. {
            let candidate = if n == 1 { name.clone() } else { format!("{stem}.{n}{ext}") };
            let info_path = self.info_for(&candidate);
            match fs::OpenOptions::new().write(true).create_new(true).open(&info_path) {
                Ok(mut f) => {
                    f.write_all(info.as_bytes())?;
                    let dest = self.files().join(&candidate);
                    if fs::symlink_metadata(&dest).is_ok() {
                        // An orphan without info already has this name.
                        let _ = fs::remove_file(&info_path);
                        continue;
                    }
                    return match fs::rename(&abs, &dest) {
                        Ok(()) => Ok(dest),
                        Err(e) => {
                            let _ = fs::remove_file(&info_path);
                            Err(e)
                        }
                    };
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    pub fn items(&self) -> Vec<Item> {
        let Ok(dir) = fs::read_dir(self.files()) else { return Vec::new() };
        dir.flatten()
            .map(|e| {
                let path = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                let (original, deleted) = fs::read_to_string(self.info_for(&name)).ok().map(|s| self.parse_info(&s)).unwrap_or_default();
                Item { path, original, deleted }
            })
            .collect()
    }

    fn parse_info(&self, text: &str) -> (Option<PathBuf>, Option<NaiveDateTime>) {
        let mut in_section = false;
        let (mut path, mut date) = (None, None);
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_section = line == "[Trash Info]";
            } else if in_section {
                if let Some(v) = line.strip_prefix("Path=") {
                    let p = PathBuf::from(unescape_path(v));
                    path = Some(match &self.topdir {
                        Some(top) if p.is_relative() => top.join(p),
                        _ => p,
                    });
                } else if let Some(v) = line.strip_prefix("DeletionDate=") {
                    date = NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S").ok();
                }
            }
        }
        (path, date)
    }

    /// Whether `path` is an item directly inside this trash's `files/`.
    pub fn holds(&self, path: &Path) -> bool {
        path.parent() == Some(self.files().as_path())
    }

    /// Puts an item back where it came from (recreating missing folders).
    /// Fails rather than overwrite something already there.
    pub fn restore(&self, item: &Path) -> io::Result<PathBuf> {
        let name = item.file_name().ok_or_else(|| io::Error::other("bad item"))?.to_string_lossy().into_owned();
        let info_path = self.info_for(&name);
        let (original, _) = self.parse_info(&fs::read_to_string(&info_path)?);
        let original = original.ok_or_else(|| io::Error::other("the original location is unknown"))?;
        if fs::symlink_metadata(&original).is_ok() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("an item named \u{201c}{}\u{201d} already exists there", original.file_name().unwrap_or_default().to_string_lossy())));
        }
        if let Some(parent) = original.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(item, &original)?;
        let _ = fs::remove_file(info_path);
        Ok(original)
    }

    /// Deletes one item for good.
    pub fn erase(&self, item: &Path) -> io::Result<()> {
        let name = item.file_name().ok_or_else(|| io::Error::other("bad item"))?.to_string_lossy().into_owned();
        remove_any(item)?;
        let _ = fs::remove_file(self.info_for(&name));
        Ok(())
    }

    /// Deletes everything.
    pub fn empty(&self) -> io::Result<()> {
        for dir in [self.files(), self.info()] {
            let Ok(it) = fs::read_dir(&dir) else { continue };
            for e in it.flatten() {
                remove_any(&e.path())?;
            }
        }
        let _ = fs::remove_file(self.root.join("directorysizes"));
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        fs::read_dir(self.files()).map_or(true, |mut it| it.next().is_none())
    }
}

fn remove_any(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        // Read-only folders inside would stop removal; make them writable.
        make_writable(path);
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn make_writable(dir: &Path) {
    if let Ok(meta) = fs::symlink_metadata(dir) {
        if meta.is_dir() && !meta.file_type().is_symlink() {
            let mode = meta.permissions().mode();
            if mode & 0o700 != 0o700 {
                let _ = fs::set_permissions(dir, fs::Permissions::from_mode(mode | 0o700));
            }
            if let Ok(it) = fs::read_dir(dir) {
                for e in it.flatten() {
                    make_writable(&e.path());
                }
            }
        }
    }
}

/// Absolute path without resolving the final component (a link is trashed
/// itself, not its target).
fn absolute(path: &Path) -> io::Result<PathBuf> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| io::Error::other("can't trash this"))?;
    Ok(fs::canonicalize(parent)?.join(name))
}

/// The mount point holding `path`: walk up while the device stays the same.
pub fn topdir_of(path: &Path) -> Option<PathBuf> {
    let dev = fs::symlink_metadata(path).ok()?.dev();
    let mut cur = absolute(path).ok()?;
    while let Some(parent) = cur.parent() {
        if fs::metadata(parent).ok()?.dev() != dev {
            break;
        }
        cur = parent.to_path_buf();
    }
    Some(cur)
}

/// Picks the right trash for `path` and moves it there: the home trash when
/// it's on the same disk, otherwise the disk's own trash.
pub fn trash(path: &Path, home: &Trash) -> io::Result<PathBuf> {
    let dev = fs::symlink_metadata(path)?.dev();
    let _ = home.ensure();
    let home_dev = fs::metadata(&home.root).map(|m| m.dev()).ok();
    if home_dev == Some(dev) {
        return home.put(path);
    }
    let top = topdir_of(path).ok_or_else(|| io::Error::other("can't find the disk's top folder"))?;
    Trash::for_topdir(&top).put(path)
}

/// Every trash in use: home, plus `.Trash-$uid` on the given mount points.
pub fn all(home: &Trash, mount_points: &[PathBuf]) -> Vec<Trash> {
    let mut v = vec![home.clone()];
    for m in mount_points {
        let t = Trash::for_topdir(m);
        if t.files().is_dir() && !v.contains(&t) && t.root != home.root {
            v.push(t);
        }
    }
    v
}

/// Percent-escapes a path for `Path=` (RFC 2396 path characters stay).
pub fn escape_path(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn unescape_path(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-trash-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn escaping() {
        assert_eq!(escape_path("/home/a b/ü%.txt"), "/home/a%20b/%C3%BC%25.txt");
        assert_eq!(unescape_path("/home/a%20b/%C3%BC%25.txt"), "/home/a b/ü%.txt");
        assert_eq!(unescape_path("100%"), "100%");
    }

    #[test]
    fn trash_restore_empty() {
        let d = temp_dir("cycle");
        let trash = Trash::home(&d.join("data"));
        let docs = d.join("My Docs");
        fs::create_dir_all(docs.join("sub")).unwrap();
        fs::write(docs.join("a file.txt"), b"one").unwrap();
        fs::write(docs.join("sub/x"), b"x").unwrap();

        let t1 = super::trash(&docs.join("a file.txt"), &trash).unwrap();
        assert_eq!(t1, trash.files().join("a file.txt"));
        assert!(!docs.join("a file.txt").exists());
        let info = fs::read_to_string(trash.info().join("a file.txt.trashinfo")).unwrap();
        let mut lines = info.lines();
        assert_eq!(lines.next(), Some("[Trash Info]"));
        let path_line = lines.next().unwrap();
        assert!(path_line.starts_with("Path=/") && path_line.ends_with("/My%20Docs/a%20file.txt"), "{path_line}");
        let date = lines.next().unwrap().strip_prefix("DeletionDate=").unwrap();
        assert!(NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S").is_ok());
        assert_eq!(fs::metadata(&trash.root).unwrap().permissions().mode() & 0o777, 0o700);

        // A second item with the same name gets a fresh one.
        fs::write(docs.join("a file.txt"), b"two").unwrap();
        let t2 = super::trash(&docs.join("a file.txt"), &trash).unwrap();
        assert_eq!(t2, trash.files().join("a file.2.txt"));
        super::trash(&docs.join("sub"), &trash).unwrap();

        let items = trash.items();
        assert_eq!(items.len(), 3);
        let first = items.iter().find(|i| i.path == t1).unwrap();
        assert_eq!(first.original.as_deref(), Some(fs::canonicalize(&docs).unwrap().join("a file.txt").as_path()));
        assert!(first.deleted.is_some());

        // Put back, but never over an existing file.
        assert_eq!(trash.restore(&t1).unwrap(), fs::canonicalize(&docs).unwrap().join("a file.txt"));
        assert_eq!(fs::read(docs.join("a file.txt")).unwrap(), b"one");
        assert!(trash.restore(&t2).is_err());
        assert!(t2.exists());

        // Restoring recreates a missing parent folder.
        fs::remove_dir_all(&docs).unwrap();
        trash.restore(&trash.files().join("sub")).unwrap();
        assert!(docs.join("sub/x").exists());

        trash.erase(&t2).unwrap();
        assert!(trash.items().is_empty());
        fs::write(docs.join("c"), b"").unwrap();
        super::trash(&docs.join("c"), &trash).unwrap();
        assert!(!trash.is_empty());
        trash.empty().unwrap();
        assert!(trash.is_empty());
        assert_eq!(fs::read_dir(trash.info()).unwrap().count(), 0);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn topdir_trash_uses_relative_paths() {
        let d = temp_dir("topdir");
        let t = Trash::for_topdir(&d);
        assert_eq!(t.root, d.join(format!(".Trash-{}", uid())));
        fs::create_dir_all(d.join("x")).unwrap();
        fs::write(d.join("x/f.txt"), b"").unwrap();
        let top = fs::canonicalize(&d).unwrap();
        let t = Trash::for_topdir(&top);
        t.put(&top.join("x/f.txt")).unwrap();
        let info = fs::read_to_string(t.info().join("f.txt.trashinfo")).unwrap();
        assert!(info.contains("\nPath=x/f.txt\n"), "{info}");
        assert_eq!(t.items()[0].original.as_deref(), Some(top.join("x/f.txt").as_path()));
        t.restore(&t.files().join("f.txt")).unwrap();
        assert!(top.join("x/f.txt").exists());
        fs::remove_dir_all(&d).unwrap();
    }

}
