//! All notes and folders live in one Automerge document, so edits made on
//! different devices (even to the same note, even offline) merge cleanly.
//!
//! Layout: the root map holds `folder:<id>` and `note:<id>` maps. Keeping every
//! object at the root (instead of under shared `folders`/`notes` maps) matters:
//! two devices that each create a `notes` map would conflict, and one side's
//! notes would vanish behind the other's.

use y7_sync::automerge::{AutoCommit, ROOT, ReadDoc, transaction::Transactable};
use y7_sync::doc::{get_bool, get_i64, get_str, put_bool, put_i64, put_opt_i64, put_str, put_text, record, records, retain_records};
use y7_sync::{Doc, Heads};

use crate::model::{DEFAULT_FOLDER, Folder, Note, now};

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub heads: Heads,
    pub folders: Vec<Folder>,
    pub notes: Vec<Note>,
}

/// Gives a new notebook its built-in folder.
pub fn ensure_default_folder(doc: &mut Doc) {
    if records(doc.read(), "folder").iter().any(|(id, _)| *id == DEFAULT_FOLDER) {
        return;
    }
    let base = doc.heads();
    doc.edit(&base, |am| {
        let obj = record(am, "folder", DEFAULT_FOLDER);
        am.put(&obj, "name", "Notes").unwrap();
        am.put(&obj, "created", 0i64).unwrap();
    });
}

pub fn snapshot(doc: &mut Doc) -> Snapshot {
    let heads = doc.heads();
    let (folders, notes) = read(doc.read());
    Snapshot { heads, folders, notes }
}

/// Makes the notebook match `folders` and `notes`, which were edited
/// starting from the version `base`. Returns the new version.
pub fn apply(doc: &mut Doc, base: &Heads, folders: &[Folder], notes: &[Note]) -> Heads {
    doc.edit(base, |am| write(am, folders, notes))
}

fn read(doc: &AutoCommit) -> (Vec<Folder>, Vec<Note>) {
    let mut folders: Vec<(i64, Folder)> = records(doc, "folder")
        .into_iter()
        .map(|(id, obj)| {
            let name = get_str(doc, &obj, "name").unwrap_or_else(|| "Untitled".into());
            (get_i64(doc, &obj, "created").unwrap_or(0), Folder { id, name })
        })
        .collect();
    let mut notes: Vec<Note> = records(doc, "note")
        .into_iter()
        .map(|(id, obj)| Note {
            id,
            folder: get_i64(doc, &obj, "folder").unwrap_or(0) as u64,
            body: get_str(doc, &obj, "body").unwrap_or_default(),
            created: get_i64(doc, &obj, "created").unwrap_or(0),
            modified: get_i64(doc, &obj, "modified").unwrap_or(0),
            pinned: get_bool(doc, &obj, "pinned").unwrap_or(false),
            deleted_at: get_i64(doc, &obj, "deleted_at"),
        })
        .collect();
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
    let t = now();
    for f in folders {
        let exists = doc.get(ROOT, format!("folder:{}", f.id).as_str()).ok().flatten().is_some();
        let obj = record(doc, "folder", f.id);
        put_str(doc, &obj, "name", &f.name);
        if !exists {
            doc.put(&obj, "created", if f.id == DEFAULT_FOLDER { 0 } else { t }).unwrap();
        }
    }
    for n in notes {
        let obj = record(doc, "note", n.id);
        put_i64(doc, &obj, "folder", n.folder as i64);
        put_i64(doc, &obj, "created", n.created);
        put_i64(doc, &obj, "modified", n.modified);
        put_opt_i64(doc, &obj, "deleted_at", n.deleted_at);
        put_bool(doc, &obj, "pinned", n.pinned);
        put_text(doc, &obj, "body", &n.body);
    }
    retain_records(doc, "folder", |id| id == DEFAULT_FOLDER || folders.iter().any(|f| f.id == id));
    retain_records(doc, "note", |id| notes.iter().any(|n| n.id == id));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u64, body: &str) -> Note {
        Note { id, folder: DEFAULT_FOLDER, body: body.into(), created: 1, modified: 1, pinned: false, deleted_at: None }
    }

    fn notebook() -> Doc {
        let mut doc = Doc::in_memory();
        ensure_default_folder(&mut doc);
        doc
    }

    /// What syncing two devices amounts to: each ends up with both histories.
    fn sync_docs(a: &mut Doc, b: &mut Doc) {
        let (ab, bb) = (a.save_bytes(), b.save_bytes());
        a.merge_bytes(&bb).unwrap();
        b.merge_bytes(&ab).unwrap();
    }

    #[test]
    fn concurrent_edits_merge() {
        let mut a = notebook();
        let mut b = notebook();
        // Both devices start out with their own default folder and notes.
        let s = snapshot(&mut a);
        apply(&mut a, &s.heads, &s.folders, &[note(1, "Groceries\nmilk")]);
        let s = snapshot(&mut b);
        apply(&mut b, &s.heads, &s.folders, &[note(2, "Phone note")]);
        sync_docs(&mut a, &mut b);
        let (sa, sb) = (snapshot(&mut a), snapshot(&mut b));
        assert_eq!(sa.notes.len(), 2);
        assert_eq!(sa.folders, sb.folders);
        assert_eq!(sa.folders.len(), 1);

        // Both edit the same note offline: both edits survive.
        let mut na = sa.notes.clone();
        na.iter_mut().find(|n| n.id == 1).unwrap().body = "Groceries\nmilk\neggs".into();
        apply(&mut a, &sa.heads, &sa.folders, &na);
        let mut nb = sb.notes.clone();
        nb.iter_mut().find(|n| n.id == 1).unwrap().body = "Groceries list\nmilk".into();
        apply(&mut b, &sb.heads, &sb.folders, &nb);
        sync_docs(&mut a, &mut b);
        let body = snapshot(&mut a).notes.into_iter().find(|n| n.id == 1).unwrap().body;
        assert_eq!(body, "Groceries list\nmilk\neggs");
        assert_eq!(snapshot(&mut b).notes.into_iter().find(|n| n.id == 1).unwrap().body, body);
    }

    #[test]
    fn stale_edit_does_not_drop_new_notes() {
        let mut a = notebook();
        let base = snapshot(&mut a);
        // A note arrives (e.g. from the phone) after the UI took its snapshot...
        let mut with_new = base.notes.clone();
        with_new.push(note(7, "From phone"));
        apply(&mut a, &base.heads, &base.folders, &with_new);
        // ...then the UI saves its own edit based on the old snapshot.
        apply(&mut a, &base.heads, &base.folders, &[note(8, "From desktop")]);
        let ids: Vec<u64> = snapshot(&mut a).notes.iter().map(|n| n.id).collect();
        assert!(ids.contains(&7) && ids.contains(&8), "{ids:?}");
    }

    #[test]
    fn saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("notes-core-test-{}", std::process::id()));
        let path = dir.join("notes.automerge");
        let mut a = Doc::open(path.clone());
        assert!(a.is_new);
        ensure_default_folder(&mut a);
        let s = snapshot(&mut a);
        let folders = [s.folders.clone(), vec![Folder { id: 5, name: "Work".into() }]].concat();
        apply(&mut a, &s.heads, &folders, &[note(3, "Saved")]);
        let mut b = Doc::open(path);
        assert!(!b.is_new);
        let s = snapshot(&mut b);
        assert_eq!(s.notes[0].body, "Saved");
        assert_eq!(s.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Notes", "Work"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
