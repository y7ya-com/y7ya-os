//! All notes and folders live in one Automerge document, so edits made on
//! different devices (even to the same note, even offline) merge cleanly.
//!
//! Layout: the root map holds `folder:<id>` and `note:<id>` maps. Keeping every
//! object at the root (instead of under shared `folders`/`notes` maps) matters:
//! two devices that each create a `notes` map would conflict, and one side's
//! notes would vanish behind the other's.

use std::path::{Path, PathBuf};

use automerge::{AutoCommit, ChangeHash, ObjId, ObjType, ROOT, ReadDoc, Value, transaction::Transactable};

use crate::model::{DEFAULT_FOLDER, Folder, Note, now};

/// Identifies a version of the notebook. Edits are applied relative to the
/// version they were made against, so changes that arrived from another
/// device in the meantime are merged instead of overwritten.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Heads(pub Vec<ChangeHash>);

impl Heads {
    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.iter().flat_map(|h| h.0).collect()
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        Heads(bytes.chunks_exact(32).map(|c| ChangeHash(c.try_into().unwrap())).collect())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub heads: Heads,
    pub folders: Vec<Folder>,
    pub notes: Vec<Note>,
}

pub struct Notebook {
    doc: AutoCommit,
    path: Option<PathBuf>,
    /// True when no saved notebook existed, so callers can import old data.
    pub is_new: bool,
}

impl Notebook {
    pub fn open(path: PathBuf) -> Self {
        let loaded = std::fs::read(&path).ok().map(|bytes| AutoCommit::load(&bytes));
        let (doc, is_new) = match loaded {
            Some(Ok(doc)) => (doc, false),
            Some(Err(e)) => {
                // Keep the unreadable file rather than silently replacing it.
                tracing::error!("notebook unreadable ({e}); starting fresh");
                let _ = std::fs::rename(&path, path.with_extension("corrupt"));
                (AutoCommit::new(), true)
            }
            None => (AutoCommit::new(), true),
        };
        let mut nb = Notebook { doc, path: Some(path), is_new };
        nb.ensure_default_folder();
        nb
    }

    pub fn in_memory() -> Self {
        let mut nb = Notebook { doc: AutoCommit::new(), path: None, is_new: true };
        nb.ensure_default_folder();
        nb
    }

    fn ensure_default_folder(&mut self) {
        let key = folder_key(DEFAULT_FOLDER);
        if self.doc.get(ROOT, key.as_str()).ok().flatten().is_none() {
            let obj = self.doc.put_object(ROOT, key.as_str(), ObjType::Map).unwrap();
            self.doc.put(&obj, "name", "Notes").unwrap();
            self.doc.put(&obj, "created", 0i64).unwrap();
            self.doc.commit();
        }
    }

    pub fn heads(&mut self) -> Heads {
        Heads(self.doc.get_heads())
    }

    pub fn snapshot(&mut self) -> Snapshot {
        let heads = self.heads();
        let (folders, notes) = read(&self.doc);
        Snapshot { heads, folders, notes }
    }

    /// Makes the notebook match `folders` and `notes`, which were edited
    /// starting from the version `base`. Returns the new version.
    pub fn apply(&mut self, base: &Heads, folders: &[Folder], notes: &[Note]) -> Heads {
        if *base == self.heads() {
            write(&mut self.doc, folders, notes);
            self.doc.commit();
        } else {
            // Other changes landed since `base`: record this edit as a branch
            // off `base` and merge it, so neither side's work is lost.
            match self.doc.fork_at(&base.0) {
                Ok(mut fork) => {
                    write(&mut fork, folders, notes);
                    fork.commit();
                    if let Err(e) = self.doc.merge(&mut fork) {
                        tracing::error!("merge failed: {e}");
                    }
                }
                Err(_) => {
                    write(&mut self.doc, folders, notes);
                    self.doc.commit();
                }
            }
        }
        self.save();
        self.heads()
    }

    pub fn save(&mut self) {
        let Some(path) = self.path.clone() else { return };
        if let Err(e) = write_atomic(&path, &self.doc.save()) {
            tracing::error!("saving notebook failed: {e}");
        }
    }

    pub(crate) fn doc(&mut self) -> &mut AutoCommit {
        &mut self.doc
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

fn folder_key(id: u64) -> String {
    format!("folder:{id}")
}

fn note_key(id: u64) -> String {
    format!("note:{id}")
}

fn get_value<'a>(doc: &'a AutoCommit, obj: &ObjId, prop: &str) -> Option<(Value<'a>, ObjId)> {
    doc.get(obj, prop).ok().flatten()
}

fn get_i64(doc: &AutoCommit, obj: &ObjId, prop: &str) -> Option<i64> {
    get_value(doc, obj, prop).and_then(|(v, _)| v.as_i64())
}

fn get_map(doc: &AutoCommit, key: &str) -> Option<ObjId> {
    match get_value(doc, &ROOT, key) {
        Some((Value::Object(ObjType::Map), obj)) => Some(obj),
        _ => None,
    }
}

fn read(doc: &AutoCommit) -> (Vec<Folder>, Vec<Note>) {
    let mut folders = Vec::new();
    let mut notes = Vec::new();
    for key in doc.keys(ROOT) {
        if let Some(id) = key.strip_prefix("folder:").and_then(|s| s.parse().ok()) {
            let Some(obj) = get_map(doc, &key) else { continue };
            let name = get_value(doc, &obj, "name").and_then(|(v, _)| v.as_str().map(str::to_owned));
            let created = get_i64(doc, &obj, "created").unwrap_or(0);
            folders.push((created, Folder { id, name: name.unwrap_or_else(|| "Untitled".into()) }));
        } else if let Some(id) = key.strip_prefix("note:").and_then(|s| s.parse().ok()) {
            let Some(obj) = get_map(doc, &key) else { continue };
            let body = match get_value(doc, &obj, "body") {
                Some((Value::Object(ObjType::Text), text)) => doc.text(&text).unwrap_or_default(),
                _ => String::new(),
            };
            notes.push(Note {
                id,
                folder: get_i64(doc, &obj, "folder").unwrap_or(0) as u64,
                body,
                created: get_i64(doc, &obj, "created").unwrap_or(0),
                modified: get_i64(doc, &obj, "modified").unwrap_or(0),
                pinned: get_value(doc, &obj, "pinned").and_then(|(v, _)| v.as_bool()).unwrap_or(false),
                deleted_at: get_i64(doc, &obj, "deleted_at"),
            });
        }
    }
    folders.sort_by_key(|(created, f)| (f.id != DEFAULT_FOLDER, *created, f.id));
    let mut folders: Vec<Folder> = folders.into_iter().map(|(_, f)| f).collect();
    // A note in a folder that was deleted on another device shows up in Notes.
    for n in &mut notes {
        if !folders.iter().any(|f| f.id == n.folder) {
            n.folder = DEFAULT_FOLDER;
        }
    }
    if folders.is_empty() {
        folders.push(Folder { id: DEFAULT_FOLDER, name: "Notes".into() });
    }
    (folders, notes)
}

/// Writes the difference between the document and the desired state.
fn write(doc: &mut AutoCommit, folders: &[Folder], notes: &[Note]) {
    let keys: Vec<String> = doc.keys(ROOT).collect();
    let t = now();

    for f in folders {
        let key = folder_key(f.id);
        match get_map(doc, &key) {
            Some(obj) => {
                let name = get_value(doc, &obj, "name").and_then(|(v, _)| v.as_str().map(str::to_owned));
                if name.as_deref() != Some(f.name.as_str()) {
                    doc.put(&obj, "name", f.name.as_str()).unwrap();
                }
            }
            None => {
                let obj = doc.put_object(ROOT, key.as_str(), ObjType::Map).unwrap();
                doc.put(&obj, "name", f.name.as_str()).unwrap();
                doc.put(&obj, "created", if f.id == DEFAULT_FOLDER { 0 } else { t }).unwrap();
            }
        }
    }

    for n in notes {
        let key = note_key(n.id);
        let obj = match get_map(doc, &key) {
            Some(obj) => obj,
            None => doc.put_object(ROOT, key.as_str(), ObjType::Map).unwrap(),
        };
        let put_i64 = |doc: &mut AutoCommit, prop: &str, v: i64| {
            if get_i64(doc, &obj, prop) != Some(v) {
                doc.put(&obj, prop, v).unwrap();
            }
        };
        put_i64(doc, "folder", n.folder as i64);
        put_i64(doc, "created", n.created);
        put_i64(doc, "modified", n.modified);
        match n.deleted_at {
            Some(at) => put_i64(doc, "deleted_at", at),
            None => {
                if get_value(doc, &obj, "deleted_at").is_some() {
                    doc.delete(&obj, "deleted_at").unwrap();
                }
            }
        }
        if get_value(doc, &obj, "pinned").and_then(|(v, _)| v.as_bool()) != Some(n.pinned) {
            doc.put(&obj, "pinned", n.pinned).unwrap();
        }
        let text = match get_value(doc, &obj, "body") {
            Some((Value::Object(ObjType::Text), text)) => text,
            _ => doc.put_object(&obj, "body", ObjType::Text).unwrap(),
        };
        if doc.text(&text).unwrap_or_default() != n.body {
            doc.update_text(&text, &n.body).unwrap();
        }
    }

    for key in keys {
        let keep = if let Some(id) = key.strip_prefix("folder:").and_then(|s| s.parse::<u64>().ok()) {
            id == DEFAULT_FOLDER || folders.iter().any(|f| f.id == id)
        } else if let Some(id) = key.strip_prefix("note:").and_then(|s| s.parse::<u64>().ok()) {
            notes.iter().any(|n| n.id == id)
        } else {
            true
        };
        if !keep {
            doc.delete(ROOT, key.as_str()).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::sync::{self, SyncDoc};

    fn note(id: u64, body: &str) -> Note {
        Note { id, folder: DEFAULT_FOLDER, body: body.into(), created: 1, modified: 1, pinned: false, deleted_at: None }
    }

    fn sync_docs(a: &mut Notebook, b: &mut Notebook) {
        let (mut sa, mut sb) = (sync::State::new(), sync::State::new());
        loop {
            let ma = a.doc().sync().generate_sync_message(&mut sa);
            let mb = b.doc().sync().generate_sync_message(&mut sb);
            if ma.is_none() && mb.is_none() {
                break;
            }
            if let Some(m) = ma {
                b.doc().sync().receive_sync_message(&mut sb, m).unwrap();
            }
            if let Some(m) = mb {
                a.doc().sync().receive_sync_message(&mut sa, m).unwrap();
            }
        }
    }

    #[test]
    fn concurrent_edits_merge() {
        let mut a = Notebook::in_memory();
        let mut b = Notebook::in_memory();
        // Both devices start out with their own default folder and notes.
        let s = a.snapshot();
        a.apply(&s.heads, &s.folders, &[note(1, "Groceries\nmilk")]);
        let s = b.snapshot();
        b.apply(&s.heads, &s.folders, &[note(2, "Phone note")]);
        sync_docs(&mut a, &mut b);
        let (sa, sb) = (a.snapshot(), b.snapshot());
        assert_eq!(sa.notes.len(), 2);
        assert_eq!(sa.folders, sb.folders);
        assert_eq!(sa.folders.len(), 1);

        // Both edit the same note offline: both edits survive.
        let mut na = sa.notes.clone();
        na.iter_mut().find(|n| n.id == 1).unwrap().body = "Groceries\nmilk\neggs".into();
        a.apply(&sa.heads, &sa.folders, &na);
        let mut nb = sb.notes.clone();
        nb.iter_mut().find(|n| n.id == 1).unwrap().body = "Groceries list\nmilk".into();
        b.apply(&sb.heads, &sb.folders, &nb);
        sync_docs(&mut a, &mut b);
        let body = a.snapshot().notes.into_iter().find(|n| n.id == 1).unwrap().body;
        assert_eq!(body, "Groceries list\nmilk\neggs");
        assert_eq!(b.snapshot().notes.into_iter().find(|n| n.id == 1).unwrap().body, body);
    }

    #[test]
    fn stale_edit_does_not_drop_new_notes() {
        let mut a = Notebook::in_memory();
        let base = a.snapshot();
        // A note arrives (e.g. from the phone) after the UI took its snapshot...
        let mut with_new = base.notes.clone();
        with_new.push(note(7, "From phone"));
        a.apply(&base.heads, &base.folders, &with_new);
        // ...then the UI saves its own edit based on the old snapshot.
        a.apply(&base.heads, &base.folders, &[note(8, "From desktop")]);
        let ids: Vec<u64> = a.snapshot().notes.iter().map(|n| n.id).collect();
        assert!(ids.contains(&7) && ids.contains(&8), "{ids:?}");
    }

    #[test]
    fn saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("notes-core-test-{}", std::process::id()));
        let path = dir.join("notes.automerge");
        let mut a = Notebook::open(path.clone());
        assert!(a.is_new);
        let s = a.snapshot();
        let folders = [s.folders.clone(), vec![Folder { id: 5, name: "Work".into() }]].concat();
        a.apply(&s.heads, &folders, &[note(3, "Saved")]);
        let mut b = Notebook::open(path);
        assert!(!b.is_new);
        let s = b.snapshot();
        assert_eq!(s.notes[0].body, "Saved");
        assert_eq!(s.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Notes", "Work"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
