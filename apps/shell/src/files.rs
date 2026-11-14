//! An in-memory index of file names under home for Spotlight. Built once in
//! the background, then kept current incrementally: a directory's mtime
//! changes when entries are added or removed, so a refresh only re-reads
//! directories whose mtime moved. Hidden, generated and huge directories are
//! skipped.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Directories that are never worth searching.
const SKIP: &[&str] = &["node_modules", "target", "__pycache__", "venv", "build", "dist", "snap", "go", "Android"];
/// Directories with more entries than this are skipped (caches, datasets).
const MAX_DIR_ENTRIES: usize = 5000;
/// Overall cap, so a giant home can't eat memory.
const MAX_FILES: usize = 400_000;
const MAX_DEPTH: usize = 12;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    lower: String,
    pub is_dir: bool,
}

struct Dir {
    mtime: Option<SystemTime>,
    depth: usize,
    entries: Vec<FileEntry>,
}

#[derive(Default)]
pub struct FileIndex {
    root: PathBuf,
    dirs: HashMap<PathBuf, Dir>,
    count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileHit {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub score: i32,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::symlink_metadata(p).and_then(|m| m.modified()).ok()
}

/// Reads one directory; `None` if it should be skipped.
fn read_dir(path: &Path) -> Option<Vec<FileEntry>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(path).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // Don't follow symlinks (loops, other disks); index them as files.
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if is_dir && SKIP.contains(&name.as_str()) {
            continue;
        }
        out.push(FileEntry { lower: name.to_lowercase(), name, is_dir });
        if out.len() > MAX_DIR_ENTRIES {
            return None;
        }
    }
    Some(out)
}

impl FileIndex {
    pub fn build(root: &Path) -> FileIndex {
        let mut index = FileIndex { root: root.to_path_buf(), dirs: HashMap::new(), count: 0 };
        index.walk(root.to_path_buf(), 0);
        index
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Indexes `start` and everything below it.
    fn walk(&mut self, start: PathBuf, depth: usize) {
        let mut queue = VecDeque::from([(start, depth)]);
        while let Some((dir, depth)) = queue.pop_front() {
            if self.count >= MAX_FILES {
                break;
            }
            let stamp = mtime(&dir);
            let Some(entries) = read_dir(&dir) else { continue };
            if depth < MAX_DEPTH {
                for e in entries.iter().filter(|e| e.is_dir) {
                    queue.push_back((dir.join(&e.name), depth + 1));
                }
            }
            self.count += entries.len();
            self.dirs.insert(dir, Dir { mtime: stamp, depth, entries });
        }
    }

    /// Picks up changes since the last build or refresh. Returns how many
    /// directories changed.
    pub fn refresh(&mut self) -> usize {
        let changed: Vec<PathBuf> =
            self.dirs.iter().filter(|(p, d)| mtime(p) != d.mtime).map(|(p, _)| p.clone()).collect();
        for path in &changed {
            let Some(old) = self.dirs.remove(path) else { continue };
            self.count -= old.entries.len();
            let old_subdirs: Vec<String> = old.entries.iter().filter(|e| e.is_dir).map(|e| e.name.clone()).collect();
            let Some(entries) = read_dir(path).filter(|_| path.exists()) else {
                self.forget(path);
                continue;
            };
            // Removed subdirectories go; new ones get indexed.
            for name in &old_subdirs {
                if !entries.iter().any(|e| e.is_dir && &e.name == name) {
                    self.forget(&path.join(name));
                }
            }
            let new_subdirs: Vec<PathBuf> = entries
                .iter()
                .filter(|e| e.is_dir && !old_subdirs.contains(&e.name))
                .map(|e| path.join(&e.name))
                .collect();
            self.count += entries.len();
            self.dirs.insert(path.clone(), Dir { mtime: mtime(path), depth: old.depth, entries });
            if old.depth < MAX_DEPTH {
                for sub in new_subdirs {
                    self.walk(sub, old.depth + 1);
                }
            }
        }
        if !self.dirs.contains_key(&self.root) {
            let root = self.root.clone();
            self.walk(root, 0);
        }
        changed.len()
    }

    /// Drops `path` and everything below it.
    fn forget(&mut self, path: &Path) {
        let gone: Vec<PathBuf> = self.dirs.keys().filter(|p| p.starts_with(path)).cloned().collect();
        for p in gone {
            if let Some(d) = self.dirs.remove(&p) {
                self.count -= d.entries.len();
            }
        }
    }

    /// The best `limit` matches for `query` by file name.
    pub fn search(&self, query: &str, limit: usize) -> Vec<FileHit> {
        let q = query.trim().to_lowercase();
        if q.chars().count() < 2 {
            return Vec::new();
        }
        let mut hits: Vec<FileHit> = Vec::new();
        for (dir, d) in &self.dirs {
            for e in &d.entries {
                let Some(score) = name_score(&e.lower, &q) else { continue };
                // Shallower paths and folders first, like Spotlight's "Top Hit".
                let score = score - d.depth as i32 * 8 + if e.is_dir { 5 } else { 0 };
                hits.push(FileHit { path: dir.join(&e.name), name: e.name.clone(), is_dir: e.is_dir, score });
            }
        }
        hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.len().cmp(&b.name.len())).then(a.path.cmp(&b.path)));
        hits.truncate(limit);
        hits
    }
}

/// How well a lowercase file name matches a lowercase query, or `None`.
pub fn name_score(name: &str, q: &str) -> Option<i32> {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    if stem == q || name == q {
        return Some(300);
    }
    if name.starts_with(q) {
        return Some(250);
    }
    let word_start = name.match_indices(q).any(|(i, _)| {
        name[..i].chars().next_back().is_some_and(|c| !c.is_alphanumeric())
    });
    if word_start {
        return Some(200);
    }
    name.contains(q).then_some(120)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_search_and_refresh() {
        let root = std::env::temp_dir().join(format!("y7-files-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["Documents/Taxes", "Projects/app/node_modules/left-pad", ".config/secret", "Pictures"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in [
            "Documents/Report 2026.pdf",
            "Documents/Taxes/report-final.txt",
            "Projects/app/node_modules/left-pad/report.js",
            ".config/secret/report.txt",
            "Pictures/holiday.png",
            "notes-report.md",
        ] {
            std::fs::write(root.join(f), "x").unwrap();
        }
        let mut index = FileIndex::build(&root);
        let names: Vec<String> = index.search("report", 10).into_iter().map(|h| h.name).collect();
        // Hidden and node_modules are skipped; prefix beats word match, shallow beats deep.
        assert_eq!(names, ["Report 2026.pdf", "report-final.txt", "notes-report.md"]);
        assert!(index.search("r", 10).is_empty());
        assert_eq!(index.search("holiday", 10)[0].path, root.join("Pictures/holiday.png"));
        assert!(index.search("taxes", 10)[0].is_dir);

        // New files and folders show up after a refresh; deleted ones go.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::create_dir_all(root.join("Pictures/Trip")).unwrap();
        std::fs::write(root.join("Pictures/Trip/beach.jpg"), "x").unwrap();
        std::fs::remove_dir_all(root.join("Documents/Taxes")).unwrap();
        assert!(index.refresh() >= 2);
        assert_eq!(index.search("beach", 10).len(), 1);
        assert!(index.search("report-final", 10).is_empty());
        let before = index.len();
        assert_eq!(index.refresh(), 0);
        assert_eq!(index.len(), before);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scoring() {
        assert_eq!(name_score("budget.xlsx", "budget"), Some(300));
        assert_eq!(name_score("budget-2026.xlsx", "budget"), Some(250));
        assert_eq!(name_score("home budget.xlsx", "budget"), Some(200));
        assert_eq!(name_score("homebudget.xlsx", "budget"), Some(120));
        assert_eq!(name_score("readme.md", "budget"), None);
    }
}
