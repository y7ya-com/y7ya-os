//! Folder listings, sorting, Finder-style names and formatting, and the file
//! operations behind copy, move and duplicate.

use std::cmp::Ordering;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local};
use media::Kind;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub kind: Kind,
    pub kind_name: String,
}

impl Entry {
    pub fn from_path(path: &Path) -> Option<Entry> {
        let link = fs::symlink_metadata(path).ok()?;
        // Follow links for what they point at; a broken link is still listed.
        let meta = fs::metadata(path).unwrap_or_else(|_| link.clone());
        let name = path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
        let is_dir = meta.is_dir();
        let (mut kind, mut kind_name) = media::kind::describe(&name, is_dir);
        if kind == Kind::Other && media::kind::extension(&name).is_none() && meta.is_file() {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o111 != 0 {
                (kind, kind_name) = (Kind::Executable, "Unix executable".into());
            } else if meta.len() > 0 && meta.len() < 1 << 20 && media::sniff_text(path) {
                (kind, kind_name) = (Kind::Text, "Plain Text Document".into());
            }
        }
        if link.file_type().is_symlink() {
            kind_name = "Alias".into();
        }
        Some(Entry {
            path: path.to_path_buf(),
            name,
            is_dir,
            is_link: link.file_type().is_symlink(),
            size: if is_dir { 0 } else { meta.len() },
            modified: meta.modified().ok(),
            created: meta.created().ok(),
            kind,
            kind_name,
        })
    }
}

/// The entries of `dir`, unsorted. Dotfiles and names listed in the folder's
/// `.hidden` file are skipped unless `show_hidden`.
pub fn list_dir(dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let hidden_list: Vec<String> = if show_hidden {
        Vec::new()
    } else {
        fs::read_to_string(dir.join(".hidden")).map(|s| s.lines().map(str::to_string).collect()).unwrap_or_default()
    };
    let mut out = Vec::new();
    for item in fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let name = item.file_name();
        let name = name.to_string_lossy();
        if !show_hidden && (name.starts_with('.') || hidden_list.iter().any(|h| *h == *name)) {
            continue;
        }
        if let Some(e) = Entry::from_path(&item.path()) {
            out.push(e);
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Modified,
    Size,
    Kind,
}

impl SortKey {
    pub const ALL: [SortKey; 4] = [SortKey::Name, SortKey::Modified, SortKey::Size, SortKey::Kind];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Modified => "Date Modified",
            SortKey::Size => "Size",
            SortKey::Kind => "Kind",
        }
    }

    /// The direction a column sorts in when first clicked: dates and sizes
    /// biggest first, like Finder.
    pub fn default_ascending(self) -> bool {
        matches!(self, SortKey::Name | SortKey::Kind)
    }
}

/// Compares names the way Finder does: case-insensitive, with runs of digits
/// compared by value ("File 2" before "File 10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| na.len().cmp(&nb.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

pub fn sort_entries(v: &mut [Entry], key: SortKey, ascending: bool) {
    v.sort_by(|a, b| {
        let by_name = || natural_cmp(&a.name, &b.name).then_with(|| a.name.cmp(&b.name));
        let ord = match key {
            SortKey::Name => by_name(),
            SortKey::Modified => a.modified.cmp(&b.modified).then_with(by_name),
            // Folders have no size ("--") and sort below the smallest file.
            SortKey::Size => {
                let size = |e: &Entry| if e.is_dir { -1 } else { e.size as i64 };
                size(a).cmp(&size(b)).then_with(by_name)
            }
            SortKey::Kind => natural_cmp(&a.kind_name, &b.kind_name).then_with(by_name),
        };
        if ascending { ord } else { ord.reverse() }
    });
}

// -- formatting ---------------------------------------------------------------

/// Finder's file sizes: decimal units, "Zero bytes", "4 KB", "1.2 MB".
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    match bytes {
        0 => "Zero bytes".into(),
        1 => "1 byte".into(),
        b if b < 1000 => format!("{b} bytes"),
        b => {
            let mut v = b as f64 / 1000.0;
            let mut unit = 0;
            while v >= 999.95 && unit < UNITS.len() - 1 {
                v /= 1000.0;
                unit += 1;
            }
            if unit == 0 {
                format!("{} KB", v.round() as u64)
            } else {
                let s = format!("{v:.1}");
                format!("{} {}", s.trim_end_matches(".0"), UNITS[unit])
            }
        }
    }
}

/// "Today at 9:41 AM", "Yesterday at 6:02 PM", "12 Mar 2024 at 10:00 AM".
pub fn format_date(t: SystemTime, now: SystemTime) -> String {
    let t: DateTime<Local> = t.into();
    let now: DateTime<Local> = now.into();
    let time = t.format("%-I:%M %p");
    let days = now.date_naive().signed_duration_since(t.date_naive()).num_days();
    match days {
        0 => format!("Today at {time}"),
        1 => format!("Yesterday at {time}"),
        _ if t.year() == now.year() => format!("{} at {time}", t.format("%-d %b")),
        _ => format!("{} at {time}", t.format("%-d %b %Y")),
    }
}

/// "Monday, 12 March 2024 at 10:00 AM" for Quick Look and the preview column.
pub fn format_date_long(t: SystemTime) -> String {
    let t: DateTime<Local> = t.into();
    t.format("%A, %-d %B %Y at %-I:%M %p").to_string()
}

pub fn count_items(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{n} items") }
}

// -- names --------------------------------------------------------------------

/// Splits "photo.jpg" into ("photo", ".jpg"); folders and dotfiles keep their name.
pub fn split_ext(name: &str, is_dir: bool) -> (&str, &str) {
    if is_dir {
        return (name, "");
    }
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 9 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

fn first_free(dir: &Path, candidates: impl Iterator<Item = String>) -> String {
    for name in candidates {
        if fs::symlink_metadata(dir.join(&name)).is_err() {
            return name;
        }
    }
    unreachable!("candidate names are unbounded")
}

/// "name", then "name 2", "name 3"…: for new folders and pasted duplicates.
pub fn numbered_name(dir: &Path, name: &str, is_dir: bool) -> String {
    let (stem, ext) = split_ext(name, is_dir);
    let first = std::iter::once(name.to_string());
    first_free(dir, first.chain((2..).map(|n| format!("{stem} {n}{ext}"))))
}

/// "name copy", then "name copy 2"…: for Duplicate and pastes into the same folder.
pub fn copy_name(dir: &Path, name: &str, is_dir: bool) -> String {
    let (stem, ext) = split_ext(name, is_dir);
    // Duplicating "a copy" gives "a copy 2", not "a copy copy".
    let base = match stem.rsplit_once(" copy") {
        Some((b, rest)) if rest.is_empty() || rest.trim().parse::<u32>().is_ok() => b,
        _ => stem,
    };
    let first = std::iter::once(format!("{base} copy{ext}"));
    first_free(dir, first.chain((2..).map(|n| format!("{base} copy {n}{ext}"))))
}

pub fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("Enter a name.".into());
    }
    if name.contains('/') || name.contains('\0') {
        return Err("Names can't contain \u{201c}/\u{201d}.".into());
    }
    if name.len() > 255 {
        return Err("That name is too long.".into());
    }
    Ok(())
}

pub fn new_folder(dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join(numbered_name(dir, "untitled folder", true));
    fs::create_dir(&path)?;
    Ok(path)
}

pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let new_name = new_name.trim();
    valid_name(new_name)?;
    let target = path.with_file_name(new_name);
    if target == path {
        return Ok(target);
    }
    if fs::symlink_metadata(&target).is_ok() {
        return Err(format!("The name \u{201c}{new_name}\u{201d} is already taken. Please choose a different name."));
    }
    fs::rename(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

// -- transfers ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Copy,
    Move,
    Duplicate,
}

/// Shared between a running transfer and the UI that shows its progress.
#[derive(Default)]
pub struct Progress {
    pub total: AtomicU64,
    pub done: AtomicU64,
    pub cancel: AtomicBool,
    pub finished: AtomicBool,
    pub current: Mutex<String>,
    pub error: Mutex<Option<String>>,
    /// Where the items ended up.
    pub created: Mutex<Vec<PathBuf>>,
}

/// Bytes a copy of `path` will write (links count as nothing).
pub fn tree_size(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    if meta.is_dir() {
        fs::read_dir(path).map(|it| it.flatten().map(|e| tree_size(&e.path())).sum()).unwrap_or(0)
    } else if meta.is_file() {
        meta.len()
    } else {
        0
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled")
}

/// Copies a file, folder or link, keeping permissions and dates.
pub fn copy_tree(src: &Path, dst: &Path, progress: &Progress, tick: &dyn Fn()) -> io::Result<()> {
    if progress.cancel.load(AtomicOrdering::Relaxed) {
        return Err(cancelled());
    }
    let meta = fs::symlink_metadata(src)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(src)?, dst)?;
    } else if ft.is_dir() {
        fs::create_dir(dst)?;
        for item in fs::read_dir(src)? {
            let item = item?;
            copy_tree(&item.path(), &dst.join(item.file_name()), progress, tick)?;
        }
        fs::set_permissions(dst, meta.permissions())?;
    } else {
        let mut from = fs::File::open(src)?;
        let mut to = fs::OpenOptions::new().write(true).create_new(true).open(dst)?;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = from.read(&mut buf)?;
            if n == 0 {
                break;
            }
            to.write_all(&buf[..n])?;
            progress.done.fetch_add(n as u64, AtomicOrdering::Relaxed);
            tick();
            if progress.cancel.load(AtomicOrdering::Relaxed) {
                drop(to);
                let _ = fs::remove_file(dst);
                return Err(cancelled());
            }
        }
        to.set_permissions(meta.permissions())?;
        if let Ok(m) = meta.modified() {
            let _ = to.set_modified(m);
        }
    }
    Ok(())
}

fn is_cross_device(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::EXDEV)
}

/// Copies, moves or duplicates `sources` into `dest`. Name clashes keep both
/// ("name 2"; "name copy" when copying into the item's own folder).
pub fn transfer(op: Op, sources: &[PathBuf], dest: &Path, progress: &Progress, tick: &dyn Fn()) {
    let total = if op == Op::Move { 0 } else { sources.iter().map(|s| tree_size(s)).sum() };
    progress.total.store(total, AtomicOrdering::Relaxed);
    for src in sources {
        if progress.cancel.load(AtomicOrdering::Relaxed) {
            break;
        }
        let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        *progress.current.lock().unwrap() = name.clone();
        let is_dir = src.is_dir();
        let same_dir = src.parent() == Some(dest);
        let result = (|| -> io::Result<PathBuf> {
            if op == Op::Move && same_dir {
                return Ok(src.clone());
            }
            if is_dir && dest.starts_with(src) {
                return Err(io::Error::other(format!("\u{201c}{name}\u{201d} can't be moved or copied into itself.")));
            }
            let target = match op {
                Op::Duplicate => dest.join(copy_name(dest, &name, is_dir)),
                Op::Copy if same_dir => dest.join(copy_name(dest, &name, is_dir)),
                _ => dest.join(numbered_name(dest, &name, is_dir)),
            };
            if op == Op::Move {
                match fs::rename(src, &target) {
                    Ok(()) => return Ok(target),
                    Err(e) if is_cross_device(&e) => {
                        // Another disk: copy, then remove the original.
                        progress.total.fetch_add(tree_size(src), AtomicOrdering::Relaxed);
                        copy_tree(src, &target, progress, tick)?;
                        if is_dir { fs::remove_dir_all(src)? } else { fs::remove_file(src)? }
                        return Ok(target);
                    }
                    Err(e) => return Err(e),
                }
            }
            if let Err(e) = copy_tree(src, &target, progress, tick) {
                // Don't leave half a folder behind.
                if e.kind() == io::ErrorKind::Interrupted && target.is_dir() {
                    let _ = fs::remove_dir_all(&target);
                }
                return Err(e);
            }
            Ok(target)
        })();
        match result {
            Ok(p) => progress.created.lock().unwrap().push(p),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => break,
            Err(e) => {
                *progress.error.lock().unwrap() = Some(format!("Couldn\u{2019}t copy \u{201c}{name}\u{201d}: {e}"));
                break;
            }
        }
    }
    progress.finished.store(true, AtomicOrdering::Relaxed);
    tick();
}

/// Whether `a` and `b` are on the same filesystem (moves are instant).
pub fn same_device(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn entry(name: &str, is_dir: bool, size: u64, age: u64) -> Entry {
        let (kind, kind_name) = media::kind::describe(name, is_dir);
        Entry {
            path: PathBuf::from(name),
            name: name.into(),
            is_dir,
            is_link: false,
            size,
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 - age)),
            created: None,
            kind,
            kind_name,
        }
    }

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10.txt", "File2.txt", "file1.txt", "apple", "Zebra", "file02.txt"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["apple", "file1.txt", "File2.txt", "file02.txt", "file10.txt", "Zebra"]);
    }

    #[test]
    fn sorting() {
        let mut v = vec![
            entry("b.png", false, 300, 5),
            entry("Docs", true, 0, 1),
            entry("a.txt", false, 10, 50),
            entry("c.rs", false, 2000, 20),
        ];
        sort_entries(&mut v, SortKey::Name, true);
        assert_eq!(names(&v), ["a.txt", "b.png", "c.rs", "Docs"]);
        sort_entries(&mut v, SortKey::Name, false);
        assert_eq!(names(&v), ["Docs", "c.rs", "b.png", "a.txt"]);
        sort_entries(&mut v, SortKey::Modified, false);
        assert_eq!(names(&v), ["Docs", "b.png", "c.rs", "a.txt"]);
        sort_entries(&mut v, SortKey::Size, false);
        assert_eq!(names(&v), ["c.rs", "b.png", "a.txt", "Docs"]);
        sort_entries(&mut v, SortKey::Kind, true);
        assert_eq!(names(&v), ["Docs", "a.txt", "b.png", "c.rs"]);
    }

    #[test]
    fn sizes_and_dates() {
        assert_eq!(format_size(0), "Zero bytes");
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(4_096), "4 KB");
        assert_eq!(format_size(1_260_000), "1.3 MB");
        assert_eq!(format_size(2_000_000_000), "2 GB");
        assert_eq!(format_size(999_999), "1 MB");
        let now = SystemTime::now();
        assert!(format_date(now, now).starts_with("Today at "));
        assert!(format_date(now - std::time::Duration::from_secs(86_400), now).starts_with("Yesterday at "));
    }

    #[test]
    fn names_and_rename() {
        let d = temp_dir("names");
        assert_eq!(numbered_name(&d, "untitled folder", true), "untitled folder");
        let f = new_folder(&d).unwrap();
        assert_eq!(f.file_name().unwrap(), "untitled folder");
        assert_eq!(new_folder(&d).unwrap().file_name().unwrap(), "untitled folder 2");

        fs::write(d.join("photo.jpg"), b"x").unwrap();
        assert_eq!(copy_name(&d, "photo.jpg", false), "photo copy.jpg");
        fs::write(d.join("photo copy.jpg"), b"x").unwrap();
        assert_eq!(copy_name(&d, "photo.jpg", false), "photo copy 2.jpg");
        assert_eq!(copy_name(&d, "photo copy.jpg", false), "photo copy 2.jpg");
        assert_eq!(numbered_name(&d, "photo.jpg", false), "photo 2.jpg");
        assert_eq!(split_ext(".bashrc", false), (".bashrc", ""));
        assert_eq!(split_ext("v1.2 notes", true), ("v1.2 notes", ""));

        let renamed = rename(&d.join("photo.jpg"), "beach.jpg").unwrap();
        assert!(renamed.exists() && !d.join("photo.jpg").exists());
        assert!(rename(&renamed, "photo copy.jpg").unwrap_err().contains("already taken"));
        assert!(rename(&renamed, "a/b").is_err());
        assert!(rename(&renamed, "  ").is_err());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn copy_move_duplicate() {
        let d = temp_dir("transfer");
        let src = d.join("src");
        fs::create_dir_all(src.join("inner")).unwrap();
        fs::write(src.join("inner/data.bin"), vec![7u8; 3_000_000]).unwrap();
        fs::write(src.join("note.txt"), b"hello").unwrap();
        std::os::unix::fs::symlink("note.txt", src.join("link")).unwrap();
        let dest = d.join("dest");
        fs::create_dir(&dest).unwrap();

        let p = Progress::default();
        transfer(Op::Copy, &[src.clone()], &dest, &p, &|| {});
        assert!(p.finished.load(AtomicOrdering::Relaxed));
        assert_eq!(p.done.load(AtomicOrdering::Relaxed), 3_000_005);
        assert_eq!(p.total.load(AtomicOrdering::Relaxed), 3_000_005);
        assert_eq!(fs::read(dest.join("src/inner/data.bin")).unwrap().len(), 3_000_000);
        assert_eq!(fs::read_link(dest.join("src/link")).unwrap(), PathBuf::from("note.txt"));
        let mtime = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(mtime(&dest.join("src/note.txt")), mtime(&src.join("note.txt")));

        // Copying again keeps both.
        let p = Progress::default();
        transfer(Op::Copy, &[src.clone()], &dest, &p, &|| {});
        assert_eq!(*p.created.lock().unwrap(), vec![dest.join("src 2")]);

        // Duplicate in place.
        let p = Progress::default();
        transfer(Op::Duplicate, &[src.join("note.txt")], &src, &p, &|| {});
        assert!(src.join("note copy.txt").exists());

        // Move, and refuse to move a folder into itself.
        let p = Progress::default();
        transfer(Op::Move, &[src.join("note copy.txt")], &dest, &p, &|| {});
        assert!(dest.join("note copy.txt").exists() && !src.join("note copy.txt").exists());
        let p = Progress::default();
        transfer(Op::Move, &[src.clone()], &src.join("inner"), &p, &|| {});
        assert!(p.error.lock().unwrap().as_deref().unwrap().contains("into itself"));
        assert!(src.exists());

        // Cancelling removes the partial copy.
        let p = Progress::default();
        p.cancel.store(true, AtomicOrdering::Relaxed);
        let target = d.join("cancel");
        fs::create_dir(&target).unwrap();
        transfer(Op::Copy, &[src.clone()], &target, &p, &|| {});
        assert!(!target.join("src").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn listing_hides_dotfiles() {
        let d = temp_dir("list");
        fs::write(d.join(".secret"), b"").unwrap();
        fs::write(d.join("shown.txt"), b"hi").unwrap();
        fs::write(d.join("tucked.txt"), b"").unwrap();
        fs::write(d.join(".hidden"), b"tucked.txt\n").unwrap();
        let mut v = list_dir(&d, false).unwrap();
        sort_entries(&mut v, SortKey::Name, true);
        assert_eq!(names(&v), ["shown.txt"]);
        assert_eq!(v[0].kind_name, "Plain Text Document");
        assert_eq!(list_dir(&d, true).unwrap().len(), 4);
        fs::remove_dir_all(&d).unwrap();
    }
}
