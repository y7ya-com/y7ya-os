use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
pub use notes_core::{DEFAULT_FOLDER, Folder, Note, now};
use notes_core::{Core, Heads};

/// The notes as the UI sees them: a working copy of the shared notebook.
/// Edit `folders` and `notes` freely, then `save()` to merge them into the
/// notebook (and on to paired devices).
#[derive(Default)]
pub struct Store {
    pub folders: Vec<Folder>,
    pub notes: Vec<Note>,
    /// The notebook version this copy was loaded from.
    heads: Heads,
    pub core: Option<Arc<Core>>,
}

impl Store {
    pub fn data_dir() -> PathBuf {
        std::env::var_os("NOTES_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("notes"))
    }

    pub fn new(core: Arc<Core>) -> Self {
        let mut store = Store { core: Some(core), ..Default::default() };
        store.reload();
        if store.notes.is_empty() && store.folders.len() == 1 && !Self::data_dir().join("peers.json").exists() {
            store.notes.push(welcome_note());
            store.save();
        }
        store
    }

    /// An in-memory store that isn't saved (previews and tests).
    pub fn detached() -> Self {
        Store { folders: vec![Folder { id: DEFAULT_FOLDER, name: "Notes".into() }], ..Default::default() }
    }

    /// Takes the notebook's current state, including edits from other devices.
    pub fn reload(&mut self) {
        let Some(core) = &self.core else { return };
        let s = core.snapshot();
        self.folders = s.folders;
        self.notes = s.notes;
        self.heads = s.heads;
    }

    /// Merges local edits into the notebook. Returns true if edits from
    /// another device arrived meanwhile, so the store now has new content.
    pub fn save(&mut self) -> bool {
        let Some(core) = &self.core else { return false };
        let heads = core.apply(&self.heads, &self.folders, &self.notes);
        let s = core.snapshot();
        let changed = s.folders != self.folders || s.notes != self.notes;
        self.folders = s.folders;
        self.notes = s.notes;
        self.heads = if changed { s.heads } else { heads };
        changed
    }

    pub fn alloc_id(&mut self) -> u64 {
        notes_core::new_id()
    }

    pub fn note(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn note_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    pub fn folder_name(&self, id: u64) -> &str {
        self.folders.iter().find(|f| f.id == id).map_or("Notes", |f| f.name.as_str())
    }

    pub fn count_in(&self, folder: u64) -> usize {
        self.notes.iter().filter(|n| n.folder == folder && n.deleted_at.is_none()).count()
    }

    pub fn deleted_count(&self) -> usize {
        self.notes.iter().filter(|n| n.deleted_at.is_some()).count()
    }

    pub fn unique_folder_name(&self, base: &str) -> String {
        let taken = |n: &str| self.folders.iter().any(|f| f.name.eq_ignore_ascii_case(n));
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap()
    }
}

fn welcome_note() -> Note {
    let t = now();
    Note {
        id: notes_core::new_id(),
        folder: DEFAULT_FOLDER,
        body: "Welcome to Notes\nJot down anything here. The first line becomes the title.\n\n\
               • Ctrl+N creates a new note\n• Ctrl+F searches all notes\n• Right-click a note to pin, move or delete it\n\
               • Sync with Phone (bottom left) pairs your Android phone"
            .into(),
        created: t,
        modified: t,
        pinned: false,
        deleted_at: None,
    }
}

fn local(ts: i64) -> DateTime<Local> {
    Local.timestamp_opt(ts, 0).single().unwrap_or_else(Local::now)
}

fn days_ago(ts: i64) -> i64 {
    let today = Local::now().date_naive();
    let day: NaiveDate = local(ts).date_naive();
    (today - day).num_days()
}

/// Short date for the notes list, as Notes shows it.
pub fn list_date(ts: i64) -> String {
    let t = local(ts);
    match days_ago(ts) {
        0 => t.format("%-I:%M %p").to_string(),
        1 => "Yesterday".into(),
        2..=6 => t.format("%A").to_string(),
        _ => t.format("%-m/%-d/%y").to_string(),
    }
}

/// The centered timestamp above a note.
pub fn full_date(ts: i64) -> String {
    local(ts).format("%B %-d, %Y at %-I:%M %p").to_string()
}

/// Section heading for a note in the list.
pub fn section(ts: i64) -> String {
    let t = local(ts);
    match days_ago(ts) {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=7 => "Previous 7 Days".into(),
        8..=30 => "Previous 30 Days".into(),
        _ if t.year() == Local::now().year() => t.format("%B").to_string(),
        _ => t.year().to_string(),
    }
}
