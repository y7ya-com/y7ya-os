use serde::{Deserialize, Serialize};

/// The built-in "Notes" folder; it can't be renamed or deleted.
pub const DEFAULT_FOLDER: u64 = 0;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// A fresh id that won't collide with ones made on other devices (never
/// [`DEFAULT_FOLDER`]).
pub use y7_sync::new_id;

/// Seconds since the Unix epoch.
pub use y7_sync::now;
