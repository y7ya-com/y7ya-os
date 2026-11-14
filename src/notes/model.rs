use std::path::PathBuf;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

/// The built-in "Notes" folder; it can't be renamed or deleted.
pub const DEFAULT_FOLDER: u64 = 0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Folder {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub folder: u64,
    pub body: String,
    pub created: i64,
    pub modified: i64,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub deleted_at: Option<i64>,
}

impl Note {
    pub fn title(&self) -> &str {
        self.body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New Note")
    }

    pub fn preview(&self) -> &str {
        self.body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .nth(1)
            .unwrap_or("No additional text")
    }

    pub fn is_blank(&self) -> bool {
        self.body.trim().is_empty()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    pub folders: Vec<Folder>,
    pub notes: Vec<Note>,
    pub next_id: u64,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

pub fn now() -> i64 {
    Local::now().timestamp()
}

impl Store {
    pub fn data_path() -> PathBuf {
        let dir = std::env::var_os("NOTES_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("notes"));
        dir.join("notes.json")
    }

    pub fn load(path: PathBuf) -> Self {
        let mut store = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Store>(&s).ok())
            .unwrap_or_else(Store::first_run);
        store.path = Some(path);
        store.ensure_default_folder();
        store
    }

    fn first_run() -> Self {
        let mut s = Store { next_id: 1, ..Default::default() };
        s.ensure_default_folder();
        let id = s.alloc_id();
        let t = now();
        s.notes.push(Note {
            id,
            folder: DEFAULT_FOLDER,
            body: "Welcome to Notes\nJot down anything here. The first line becomes the title.\n\n\
                   • Ctrl+N creates a new note\n• Ctrl+F searches all notes\n• Right-click a note to pin, move or delete it"
                .into(),
            created: t,
            modified: t,
            pinned: false,
            deleted_at: None,
        });
        s
    }

    fn ensure_default_folder(&mut self) {
        if !self.folders.iter().any(|f| f.id == DEFAULT_FOLDER) {
            self.folders.insert(0, Folder { id: DEFAULT_FOLDER, name: "Notes".into() });
        }
        self.next_id = self.next_id.max(1);
    }

    pub fn alloc_id(&mut self) -> u64 {
        let max = self.notes.iter().map(|n| n.id).chain(self.folders.iter().map(|f| f.id)).max().unwrap_or(0);
        self.next_id = self.next_id.max(max + 1);
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Writes atomically (temp file + rename) so a crash can't corrupt notes.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
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
