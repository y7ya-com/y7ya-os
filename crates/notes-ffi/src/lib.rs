//! Kotlin-facing API for the Android app, generated with UniFFI.

use std::sync::Arc;

use notes_core::{Core, Folder, Heads, Listener, Note};

uniffi::setup_scaffolding!();

#[derive(uniffi::Record)]
pub struct FolderRecord {
    pub id: i64,
    pub name: String,
}

#[derive(uniffi::Record)]
pub struct NoteRecord {
    pub id: i64,
    pub folder: i64,
    pub body: String,
    pub created: i64,
    pub modified: i64,
    pub pinned: bool,
    pub deleted_at: Option<i64>,
}

/// The notebook at one version. Pass `heads` back to `apply` with your edits.
#[derive(uniffi::Record)]
pub struct SnapshotRecord {
    pub heads: Vec<u8>,
    pub folders: Vec<FolderRecord>,
    pub notes: Vec<NoteRecord>,
}

#[derive(uniffi::Record)]
pub struct PeerRecord {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub last_sync: Option<i64>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PairError {
    #[error("{reason}")]
    Failed { reason: String },
}

/// Called from a background thread.
#[uniffi::export(with_foreign)]
pub trait CoreListener: Send + Sync {
    fn notes_changed(&self);
    fn status_changed(&self);
}

struct Forward(Arc<dyn CoreListener>);

impl Listener for Forward {
    fn notes_changed(&self) {
        self.0.notes_changed();
    }
    fn status_changed(&self) {
        self.0.status_changed();
    }
}

#[derive(uniffi::Object)]
pub struct NotesCore {
    core: Core,
}

#[uniffi::export]
impl NotesCore {
    #[uniffi::constructor]
    pub fn new(data_dir: String, device_name: String, listener: Arc<dyn CoreListener>) -> Arc<Self> {
        Arc::new(NotesCore { core: Core::open(data_dir.into(), device_name, Arc::new(Forward(listener))) })
    }

    pub fn snapshot(&self) -> SnapshotRecord {
        to_record(self.core.snapshot())
    }

    /// Saves the full set of folders and notes, as edited starting from the
    /// snapshot whose `heads` are given. Returns the notebook afterwards, which
    /// also includes anything that arrived from other devices meanwhile.
    pub fn apply(&self, heads: Vec<u8>, folders: Vec<FolderRecord>, notes: Vec<NoteRecord>) -> SnapshotRecord {
        let folders: Vec<Folder> = folders.into_iter().map(|f| Folder { id: f.id as u64, name: f.name }).collect();
        let notes: Vec<Note> = notes
            .into_iter()
            .map(|n| Note {
                id: n.id as u64,
                folder: n.folder as u64,
                body: n.body,
                created: n.created,
                modified: n.modified,
                pinned: n.pinned,
                deleted_at: n.deleted_at,
            })
            .collect();
        self.core.apply(&Heads::from_bytes(&heads), &folders, &notes);
        self.snapshot()
    }

    pub fn new_id(&self) -> i64 {
        notes_core::new_id() as i64
    }

    pub fn peers(&self) -> Vec<PeerRecord> {
        self.core
            .sync()
            .status()
            .peers
            .into_iter()
            .map(|p| PeerRecord { id: p.id, name: p.name, connected: p.connected, last_sync: p.last_sync })
            .collect()
    }

    /// Pairs with the computer showing this code. Blocks for up to ~30 s, so
    /// call it off the main thread. Returns the computer's name.
    pub fn pair(&self, code: String) -> Result<String, PairError> {
        self.core.sync().pair(&code).map_err(|e| PairError::Failed { reason: e.to_string() })
    }

    pub fn unpair(&self, id: String) {
        self.core.sync().unpair(&id);
    }

    /// Reconnect to paired devices now, e.g. when the app comes to the front.
    pub fn sync_now(&self) {
        self.core.sync().sync_now();
    }
}

fn to_record(s: notes_core::Snapshot) -> SnapshotRecord {
    SnapshotRecord {
        heads: s.heads.to_bytes(),
        folders: s.folders.into_iter().map(|f| FolderRecord { id: f.id as i64, name: f.name }).collect(),
        notes: s
            .notes
            .into_iter()
            .map(|n| NoteRecord {
                id: n.id as i64,
                folder: n.folder as i64,
                body: n.body,
                created: n.created,
                modified: n.modified,
                pinned: n.pinned,
                deleted_at: n.deleted_at,
            })
            .collect(),
    }
}
