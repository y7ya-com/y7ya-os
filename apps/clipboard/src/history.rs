//! Clipboard history on disk: `~/.local/share/clipboard/` (or
//! `$CLIPBOARD_DATA_DIR`) holds `history.tsv`, one line per entry, and each
//! entry's content under `items/`. Entries are named by a hash of their
//! content, so copying the same thing again just moves it to the top.
//!
//! The daemon and the picker both write here, so every change takes a lock,
//! re-reads the index, edits it and replaces it atomically.

use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

/// Unpinned entries kept; older ones are dropped. Pinned entries always stay.
pub const CAP: usize = 200;
pub const MAX_TEXT: usize = 1024 * 1024;
pub const MAX_IMAGE: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// PNG.
    Image,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Text(String),
    Png(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    pub pinned: bool,
    /// When it was (last) copied, in Unix milliseconds.
    pub time: i64,
    /// Bytes of content.
    pub size: u64,
    /// Image width and height.
    pub dims: Option<(u32, u32)>,
}

pub struct Store {
    dir: PathBuf,
}

/// 64-bit FNV-1a: stable across builds, unlike std's hasher.
pub fn content_id(kind: Kind, bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in [kind as u8].iter().chain(bytes) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Width and height from a PNG's IHDR chunk.
pub fn png_dims(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
    Some((be(16), be(20)))
}

fn parse_index(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let kind = match *f.get(1)? {
                "text" => Kind::Text,
                "image" => Kind::Image,
                _ => return None,
            };
            let dims = match (f.get(5).and_then(|v| v.parse().ok()), f.get(6).and_then(|v| v.parse().ok())) {
                (Some(w), Some(h)) => Some((w, h)),
                _ => None,
            };
            Some(Entry {
                id: f.first()?.to_string(),
                kind,
                pinned: *f.get(2)? == "1",
                time: f.get(3)?.parse().ok()?,
                size: f.get(4)?.parse().ok()?,
                dims,
            })
        })
        .collect()
}

fn format_index(entries: &[Entry]) -> String {
    let mut s = String::new();
    for e in entries {
        let kind = if e.kind == Kind::Text { "text" } else { "image" };
        let (w, h) = e.dims.map_or((String::new(), String::new()), |(w, h)| (w.to_string(), h.to_string()));
        s.push_str(&format!("{}\t{kind}\t{}\t{}\t{}\t{w}\t{h}\n", e.id, if e.pinned { 1 } else { 0 }, e.time, e.size));
    }
    s
}

/// Pinned first, then newest first.
pub fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.time.cmp(&a.time)));
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn default_dir() -> PathBuf {
        if let Some(d) = std::env::var_os("CLIPBOARD_DATA_DIR") {
            return d.into();
        }
        dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("clipboard")
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("history.tsv")
    }

    pub fn item_path(&self, e: &Entry) -> PathBuf {
        let ext = if e.kind == Kind::Text { "txt" } else { "png" };
        self.dir.join("items").join(format!("{}.{ext}", e.id))
    }

    /// Entries, pinned first and then newest first.
    pub fn load(&self) -> Vec<Entry> {
        let mut v = parse_index(&std::fs::read_to_string(self.index_path()).unwrap_or_default());
        sort(&mut v);
        v
    }

    pub fn read(&self, e: &Entry) -> Option<Vec<u8>> {
        std::fs::read(self.item_path(e)).ok()
    }

    pub fn read_text(&self, e: &Entry) -> Option<String> {
        (e.kind == Kind::Text).then(|| self.read(e)).flatten().map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Runs `f` on the index under an exclusive lock and saves the result.
    fn update<T>(&self, f: impl FnOnce(&mut Vec<Entry>) -> T) -> std::io::Result<T> {
        std::fs::create_dir_all(self.dir.join("items"))?;
        let lock = std::fs::File::create(self.dir.join(".lock"))?;
        // Released when `lock` is closed.
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) };
        let mut entries = self.load();
        let out = f(&mut entries);
        sort(&mut entries);
        let tmp = self.dir.join("history.tsv.tmp");
        std::fs::File::create(&tmp)?.write_all(format_index(&entries).as_bytes())?;
        std::fs::rename(&tmp, self.index_path())?;
        Ok(out)
    }

    /// Records a copy. Returns the entry id, or `None` if it was skipped
    /// (empty, or over the size limit).
    pub fn add(&self, content: &Content, time: i64) -> std::io::Result<Option<String>> {
        let (kind, bytes): (Kind, &[u8]) = match content {
            Content::Text(t) if t.trim().is_empty() || t.len() > MAX_TEXT => return Ok(None),
            Content::Text(t) => (Kind::Text, t.as_bytes()),
            Content::Png(p) if p.len() > MAX_IMAGE || png_dims(p).is_none() => return Ok(None),
            Content::Png(p) => (Kind::Image, p),
        };
        let id = content_id(kind, bytes);
        let entry = Entry { id: id.clone(), kind, pinned: false, time, size: bytes.len() as u64, dims: png_dims(bytes) };
        // Content first, so the index never points at a missing file.
        let path = self.item_path(&entry);
        std::fs::create_dir_all(self.dir.join("items"))?;
        if !path.exists() {
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, &path)?;
        }
        let dropped = self.update(|entries| {
            if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                e.time = time;
            } else {
                entries.push(entry);
            }
            // Over the cap: drop the oldest unpinned entries.
            sort(entries);
            let mut unpinned = 0;
            let mut dropped = Vec::new();
            entries.retain(|e| {
                if e.pinned {
                    return true;
                }
                unpinned += 1;
                if unpinned <= CAP {
                    true
                } else {
                    dropped.push(e.clone());
                    false
                }
            });
            dropped
        })?;
        for e in dropped {
            let _ = std::fs::remove_file(self.item_path(&e));
        }
        Ok(Some(id))
    }

    pub fn set_pinned(&self, id: &str, pinned: bool) -> std::io::Result<()> {
        self.update(|entries| {
            if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                e.pinned = pinned;
            }
        })
    }

    /// Moves an entry to the top, as if just copied.
    pub fn touch(&self, id: &str, time: i64) -> std::io::Result<()> {
        self.update(|entries| {
            if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                e.time = time;
            }
        })
    }

    pub fn remove(&self, id: &str) -> std::io::Result<()> {
        let gone = self.update(|entries| {
            let gone: Vec<Entry> = entries.iter().filter(|e| e.id == id).cloned().collect();
            entries.retain(|e| e.id != id);
            gone
        })?;
        for e in gone {
            let _ = std::fs::remove_file(self.item_path(&e));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("clipboard-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::open(dir)
    }

    /// The smallest valid-looking PNG header for `png_dims`.
    fn fake_png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        v.extend(w.to_be_bytes());
        v.extend(h.to_be_bytes());
        v.extend([8, 6, 0, 0, 0]);
        v
    }

    #[test]
    fn index_round_trips() {
        let e = vec![
            Entry { id: "a".into(), kind: Kind::Text, pinned: true, time: 5, size: 3, dims: None },
            Entry { id: "b".into(), kind: Kind::Image, pinned: false, time: 7, size: 9, dims: Some((640, 480)) },
        ];
        assert_eq!(parse_index(&format_index(&e)), e);
        assert!(parse_index("garbage\nx\tweird\t0\t1\t2\n").is_empty());
    }

    #[test]
    fn dedupes_and_orders() {
        let s = temp_store("dedupe");
        let a = s.add(&Content::Text("hello".into()), 100).unwrap().unwrap();
        s.add(&Content::Text("world".into()), 200).unwrap();
        // Copying "hello" again moves it to the top instead of duplicating it.
        let again = s.add(&Content::Text("hello".into()), 300).unwrap().unwrap();
        assert_eq!(a, again);
        let entries = s.load();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, a);
        assert_eq!(entries[0].time, 300);
        assert_eq!(s.read_text(&entries[0]).as_deref(), Some("hello"));
        // Whitespace-only text is ignored.
        assert_eq!(s.add(&Content::Text("  \n".into()), 400).unwrap(), None);
        std::fs::remove_dir_all(&s.dir).unwrap();
    }

    #[test]
    fn images_pins_and_removal() {
        let s = temp_store("pins");
        let img = s.add(&Content::Png(fake_png(1920, 1080)), 10).unwrap().unwrap();
        let txt = s.add(&Content::Text("pin me".into()), 20).unwrap().unwrap();
        s.add(&Content::Text("newer".into()), 30).unwrap();
        s.set_pinned(&img, true).unwrap();
        let entries = s.load();
        assert_eq!(entries[0].id, img, "pinned entries come first");
        assert_eq!(entries[0].dims, Some((1920, 1080)));
        assert_eq!(entries[0].kind, Kind::Image);
        // Not a PNG: skipped.
        assert_eq!(s.add(&Content::Png(b"nope".to_vec()), 40).unwrap(), None);

        let path = s.item_path(entries.iter().find(|e| e.id == txt).unwrap());
        assert!(path.exists());
        s.remove(&txt).unwrap();
        assert!(!path.exists());
        assert!(s.load().iter().all(|e| e.id != txt));
        std::fs::remove_dir_all(&s.dir).unwrap();
    }

    #[test]
    fn cap_keeps_pinned_and_newest() {
        let s = temp_store("cap");
        let pinned = s.add(&Content::Text("keep forever".into()), 0).unwrap().unwrap();
        s.set_pinned(&pinned, true).unwrap();
        for i in 1..=CAP as i64 + 5 {
            s.add(&Content::Text(format!("item {i}")), i).unwrap();
        }
        let entries = s.load();
        assert_eq!(entries.len(), CAP + 1);
        assert_eq!(entries[0].id, pinned);
        assert_eq!(s.read_text(&entries[1]).as_deref(), Some(format!("item {}", CAP + 5).as_str()));
        // The five oldest unpinned items (and their files) are gone.
        assert!(entries.iter().all(|e| s.read_text(e).as_deref() != Some("item 5")));
        let files = std::fs::read_dir(&s.dir.join("items")).unwrap().count();
        assert_eq!(files, CAP + 1);
        std::fs::remove_dir_all(&s.dir).unwrap();
    }
}
