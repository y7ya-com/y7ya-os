//! Shared core of the Notes apps: the notes document, synced with y7-sync.

pub mod model;
pub mod notebook;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use model::{DEFAULT_FOLDER, Folder, Note, new_id, now};
pub use notebook::Snapshot;
pub use y7_sync::{Doc, Heads, Hub, NOTES_DOC, PeerStatus, Status};

/// Receives sync events. Called from background threads.
pub trait Listener: Send + Sync + 'static {
    /// The notes changed because of another device.
    fn notes_changed(&self);
    /// Paired devices, their connection state, or pairing changed.
    fn status_changed(&self);
}

/// Passes on the events that concern Notes.
pub struct NotesEvents<L: ?Sized>(pub Arc<L>);

impl<L: Listener + ?Sized> y7_sync::Listener for NotesEvents<L> {
    fn doc_changed(&self, doc: &str) {
        if doc == NOTES_DOC {
            self.0.notes_changed();
        }
    }
    fn status_changed(&self) {
        self.0.status_changed();
    }
}

/// The notebook plus sync, as used by an app.
pub struct Core {
    hub: Arc<dyn Hub>,
    notebook: Arc<Mutex<Doc>>,
    /// Whether the hub is ours to shut down (an in-process engine).
    owns_hub: bool,
}

impl Core {
    /// Runs sync in this process with the identity and notebook in `dir`
    /// (the phone, and tests). On the desktop use [`Core::with_hub`].
    pub fn open(dir: PathBuf, device_name: String, listener: Arc<dyn Listener>) -> Self {
        let engine = y7_sync::Engine::start(dir.clone(), device_name, Arc::new(NotesEvents(listener)));
        let mut core = Core::with_hub(Arc::new(engine));
        core.owns_hub = true;
        core.import_legacy(&dir);
        core
    }

    /// The notebook from `hub` (e.g. a [`y7_sync::Client`] of the sync daemon).
    pub fn with_hub(hub: Arc<dyn Hub>) -> Self {
        let notebook = hub.doc(NOTES_DOC);
        notebook::ensure_default_folder(&mut notebook.lock().unwrap());
        hub.local_changed(NOTES_DOC);
        Core { hub, notebook, owns_hub: false }
    }

    pub fn snapshot(&self) -> Snapshot {
        notebook::snapshot(&mut self.notebook.lock().unwrap())
    }

    /// Saves an edit made starting from the version `base` and sends it to
    /// paired devices. Returns the new version; if it differs from what the
    /// caller expects, reload.
    pub fn apply(&self, base: &Heads, folders: &[Folder], notes: &[Note]) -> Heads {
        let heads = notebook::apply(&mut self.notebook.lock().unwrap(), base, folders, notes);
        self.hub.local_changed(NOTES_DOC);
        heads
    }

    pub fn sync(&self) -> &dyn Hub {
        &*self.hub
    }

    pub fn hub(&self) -> Arc<dyn Hub> {
        self.hub.clone()
    }

    /// Brings in notes saved by the first desktop version (`notes.json` in
    /// `dir`), once.
    pub fn import_legacy(&self, dir: &Path) {
        #[derive(serde::Deserialize)]
        struct Legacy {
            folders: Vec<Folder>,
            notes: Vec<Note>,
        }
        let path = dir.join("notes.json");
        let Some(legacy) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Legacy>(&b).ok()) else {
            return;
        };
        let base = self.snapshot();
        let mut folders = base.folders;
        let new: Vec<Folder> = legacy.folders.into_iter().filter(|f| !folders.iter().any(|g| g.id == f.id)).collect();
        folders.extend(new);
        let mut notes = base.notes;
        let new: Vec<Note> = legacy.notes.into_iter().filter(|n| !notes.iter().any(|m| m.id == n.id)).collect();
        notes.extend(new);
        self.apply(&base.heads, &folders, &notes);
        let _ = std::fs::rename(&path, dir.join("notes.json.imported"));
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        if self.owns_hub {
            self.hub.shutdown();
        }
    }
}
