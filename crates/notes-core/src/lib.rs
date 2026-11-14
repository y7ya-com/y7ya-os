//! Shared core of the Notes apps: the notes document and device-to-device sync.

pub mod model;
pub mod net;
pub mod notebook;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use model::{DEFAULT_FOLDER, Folder, Note, new_id, now};
pub use net::{Listener, PeerStatus, Status};
pub use notebook::{Heads, Notebook, Snapshot};

/// The notebook plus sync, as used by an app.
pub struct Core {
    notebook: Arc<Mutex<Notebook>>,
    sync: net::Sync,
}

impl Core {
    /// Opens the notebook in `dir` and starts syncing with paired devices.
    pub fn open(dir: PathBuf, device_name: String, listener: Arc<dyn Listener>) -> Self {
        let mut notebook = Notebook::open(dir.join("notes.automerge"));
        if notebook.is_new {
            import_legacy(&mut notebook, &dir);
        }
        let notebook = Arc::new(Mutex::new(notebook));
        let sync = net::Sync::start(notebook.clone(), dir, device_name, listener);
        Core { notebook, sync }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.notebook.lock().unwrap().snapshot()
    }

    /// Saves an edit made starting from the version `base` (see
    /// [`Notebook::apply`]) and sends it to paired devices. Returns the new
    /// version; if it differs from what the caller expects, reload.
    pub fn apply(&self, base: &Heads, folders: &[Folder], notes: &[Note]) -> Heads {
        let heads = self.notebook.lock().unwrap().apply(base, folders, notes);
        self.sync.local_changed();
        heads
    }

    pub fn sync(&self) -> &net::Sync {
        &self.sync
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        self.sync.shutdown();
    }
}

/// Brings in notes saved by the first desktop version (`notes.json`).
fn import_legacy(notebook: &mut Notebook, dir: &std::path::Path) {
    #[derive(serde::Deserialize)]
    struct Legacy {
        folders: Vec<Folder>,
        notes: Vec<Note>,
    }
    let path = dir.join("notes.json");
    let Some(legacy) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Legacy>(&b).ok()) else {
        return;
    };
    let base = notebook.snapshot();
    let mut folders = base.folders;
    folders.extend(legacy.folders.into_iter().filter(|f| f.id != DEFAULT_FOLDER));
    notebook.apply(&base.heads, &folders, &legacy.notes);
    let _ = std::fs::rename(&path, dir.join("notes.json.imported"));
}
