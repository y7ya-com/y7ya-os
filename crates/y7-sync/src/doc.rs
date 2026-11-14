//! One Automerge document, saved to disk (on the device that owns sync) or
//! held in memory (an app's replica of a document the daemon owns).
//!
//! Apps edit documents the same way: take a [`Heads`]-stamped snapshot, change
//! a copy freely, then [`Doc::edit`] relative to those heads. Edits that
//! arrived from elsewhere in the meantime are merged instead of overwritten.

use std::path::{Path, PathBuf};

use automerge::{AutoCommit, ChangeHash, ObjId, ObjType, ROOT, ReadDoc, ScalarValue, Value, transaction::Transactable};

/// Identifies a version of a document.
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

pub struct Doc {
    am: AutoCommit,
    path: Option<PathBuf>,
    /// True when nothing was saved before, so callers can import old data.
    pub is_new: bool,
}

impl Doc {
    pub fn open(path: PathBuf) -> Self {
        let loaded = std::fs::read(&path).ok().map(|bytes| AutoCommit::load(&bytes));
        let (am, is_new) = match loaded {
            Some(Ok(am)) => (am, false),
            Some(Err(e)) => {
                // Keep the unreadable file rather than silently replacing it.
                tracing::error!("{} unreadable ({e}); starting fresh", path.display());
                let _ = std::fs::rename(&path, path.with_extension("corrupt"));
                (AutoCommit::new(), true)
            }
            None => (AutoCommit::new(), true),
        };
        Doc { am, path: Some(path), is_new }
    }

    pub fn in_memory() -> Self {
        Doc { am: AutoCommit::new(), path: None, is_new: true }
    }

    pub fn heads(&mut self) -> Heads {
        Heads(self.am.get_heads())
    }

    /// Read access for schema code.
    pub fn read(&self) -> &AutoCommit {
        &self.am
    }

    /// Records the edit `f` makes, as made starting from the version `base`,
    /// then saves. Returns the new version.
    pub fn edit(&mut self, base: &Heads, f: impl FnOnce(&mut AutoCommit)) -> Heads {
        if *base == self.heads() {
            f(&mut self.am);
            self.am.commit();
        } else {
            // Other changes landed since `base`: record this edit as a branch
            // off `base` and merge it, so neither side's work is lost.
            match self.am.fork_at(&base.0) {
                Ok(mut fork) => {
                    f(&mut fork);
                    fork.commit();
                    if let Err(e) = self.am.merge(&mut fork) {
                        tracing::error!("merge failed: {e}");
                    }
                }
                Err(_) => {
                    f(&mut self.am);
                    self.am.commit();
                }
            }
        }
        self.save();
        self.heads()
    }

    /// Merges in a whole document saved elsewhere (see [`Doc::save_bytes`]).
    pub fn merge_bytes(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        self.am.load_incremental(bytes)?;
        Ok(())
    }

    pub fn save_bytes(&mut self) -> Vec<u8> {
        self.am.save()
    }

    pub fn save(&mut self) {
        let Some(path) = self.path.clone() else { return };
        if let Err(e) = write_atomic(&path, &self.am.save()) {
            tracing::error!("saving {} failed: {e}", path.display());
        }
    }

    pub(crate) fn am(&mut self) -> &mut AutoCommit {
        &mut self.am
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

// -- helpers for schema code ------------------------------------------------
//
// Documents keep every record as a map at the root (`<kind>:<id>`), never
// under shared container maps: two devices that each create a `notes` map
// would conflict, and one side's records would vanish behind the other's.

/// Records of one kind at the root: `(id, map)` for every `<prefix>:<id>` key.
pub fn records(doc: &AutoCommit, prefix: &str) -> Vec<(u64, ObjId)> {
    let mut out = Vec::new();
    for key in doc.keys(ROOT) {
        let Some(id) = key.strip_prefix(prefix).and_then(|s| s.strip_prefix(':')).and_then(|s| s.parse().ok()) else {
            continue;
        };
        if let Ok(Some((Value::Object(ObjType::Map), obj))) = doc.get(ROOT, key.as_str()) {
            out.push((id, obj));
        }
    }
    out
}

/// The map for record `<prefix>:<id>`, created if missing.
pub fn record(doc: &mut AutoCommit, prefix: &str, id: u64) -> ObjId {
    let key = format!("{prefix}:{id}");
    match doc.get(ROOT, key.as_str()) {
        Ok(Some((Value::Object(ObjType::Map), obj))) => obj,
        _ => doc.put_object(ROOT, key.as_str(), ObjType::Map).unwrap(),
    }
}

/// Deletes `<prefix>:<id>` records whose id isn't in `keep`.
pub fn retain_records(doc: &mut AutoCommit, prefix: &str, keep: impl Fn(u64) -> bool) {
    for (id, _) in records(doc, prefix) {
        if !keep(id) {
            doc.delete(ROOT, format!("{prefix}:{id}").as_str()).unwrap();
        }
    }
}

fn scalar(doc: &AutoCommit, obj: &ObjId, prop: &str) -> Option<ScalarValue> {
    match doc.get(obj, prop).ok().flatten() {
        Some((Value::Scalar(s), _)) => Some(s.into_owned()),
        _ => None,
    }
}

pub fn get_i64(doc: &AutoCommit, obj: &ObjId, prop: &str) -> Option<i64> {
    match scalar(doc, obj, prop)? {
        ScalarValue::Int(v) => Some(v),
        ScalarValue::Uint(v) => Some(v as i64),
        ScalarValue::F64(v) => Some(v as i64),
        ScalarValue::Timestamp(v) => Some(v),
        _ => None,
    }
}

pub fn get_bool(doc: &AutoCommit, obj: &ObjId, prop: &str) -> Option<bool> {
    match scalar(doc, obj, prop)? {
        ScalarValue::Boolean(v) => Some(v),
        _ => None,
    }
}

/// A string, stored either as a plain value or as collaborative text.
pub fn get_str(doc: &AutoCommit, obj: &ObjId, prop: &str) -> Option<String> {
    match doc.get(obj, prop).ok().flatten()? {
        (Value::Scalar(s), _) => s.as_str().map(str::to_owned),
        (Value::Object(ObjType::Text), text) => doc.text(&text).ok(),
        _ => None,
    }
}

pub fn put_i64(doc: &mut AutoCommit, obj: &ObjId, prop: &str, v: i64) {
    if get_i64(doc, obj, prop) != Some(v) {
        doc.put(obj, prop, v).unwrap();
    }
}

pub fn put_bool(doc: &mut AutoCommit, obj: &ObjId, prop: &str, v: bool) {
    if get_bool(doc, obj, prop) != Some(v) {
        doc.put(obj, prop, v).unwrap();
    }
}

pub fn put_str(doc: &mut AutoCommit, obj: &ObjId, prop: &str, v: &str) {
    if get_str(doc, obj, prop).as_deref() != Some(v) {
        doc.put(obj, prop, v).unwrap();
    }
}

/// Optional values are absent from the map when `None`.
pub fn put_opt_i64(doc: &mut AutoCommit, obj: &ObjId, prop: &str, v: Option<i64>) {
    match v {
        Some(v) => put_i64(doc, obj, prop, v),
        None => delete(doc, obj, prop),
    }
}

pub fn put_opt_u64(doc: &mut AutoCommit, obj: &ObjId, prop: &str, v: Option<u64>) {
    put_opt_i64(doc, obj, prop, v.map(|v| v as i64));
}

/// Collaborative text: concurrent edits to the same field merge by character.
pub fn put_text(doc: &mut AutoCommit, obj: &ObjId, prop: &str, v: &str) {
    let text = match doc.get(obj, prop).ok().flatten() {
        Some((Value::Object(ObjType::Text), text)) => text,
        _ => doc.put_object(obj, prop, ObjType::Text).unwrap(),
    };
    if doc.text(&text).unwrap_or_default() != v {
        doc.update_text(&text, v).unwrap();
    }
}

pub fn delete(doc: &mut AutoCommit, obj: &ObjId, prop: &str) {
    if doc.get(obj, prop).ok().flatten().is_some() {
        doc.delete(obj, prop).unwrap();
    }
}
