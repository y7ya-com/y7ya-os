//! Reading and writing documents, and the path completions the save and open
//! sheets offer.

use std::path::{Path, PathBuf};

/// A document as read from disk, with `\n` line endings.
pub struct Loaded {
    pub text: String,
    /// The file used `\r\n`; saving puts them back.
    pub crlf: bool,
}

pub fn load(path: &Path) -> std::io::Result<Loaded> {
    let bytes = std::fs::read(path)?;
    let text = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    let text = text.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(text);
    let crlf = text.contains("\r\n");
    let text = if crlf { text.replace("\r\n", "\n") } else { text };
    Ok(Loaded { text, crlf })
}

/// Writes through a temporary file and a rename, so a crash never leaves a
/// half-written document. Keeps the old file's permissions.
pub fn save(path: &Path, text: &str, crlf: bool) -> std::io::Result<()> {
    let data = if crlf { text.replace('\n', "\r\n") } else { text.to_string() };
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("document");
    let tmp = path.with_file_name(format!(".{name}.textedit-{}", std::process::id()));
    std::fs::write(&tmp, data)?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Turns what's typed in a sheet into a path: `~` is the home folder and
/// relative paths start there too.
pub fn expand(typed: &str) -> PathBuf {
    let typed = typed.trim();
    if typed == "~" {
        return home();
    }
    if let Some(rest) = typed.strip_prefix("~/") {
        return home().join(rest);
    }
    let p = PathBuf::from(typed);
    if p.is_absolute() { p } else { home().join(p) }
}

/// The reverse of [`expand`] for display: `/home/me/x` reads `~/x`.
pub fn contract(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Where a new document is saved unless the user picks somewhere else.
pub fn default_save_path(name: &str) -> String {
    let docs = home().join("Documents");
    let dir = if docs.is_dir() { docs } else { home() };
    format!("{}/{name}", contract(&dir))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub name: String,
    pub is_dir: bool,
    /// The field's text after picking it.
    pub completion: String,
}

/// Folders (and files, unless `folders_only`) in the folder being typed that
/// start with the last path component. For saving, the last component is the
/// file name, so when nothing matches every folder there is offered.
pub fn suggestions(typed: &str, folders_only: bool) -> Vec<Suggestion> {
    let (dir_part, prefix) = match typed.rfind('/') {
        Some(i) => (&typed[..=i], &typed[i + 1..]),
        None => ("", typed),
    };
    let dir = if dir_part.is_empty() { home() } else { expand(dir_part) };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut all: Vec<(String, bool)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let is_dir = e.path().is_dir();
            (!name.starts_with('.') || prefix.starts_with('.')).then_some((name, is_dir))
        })
        .filter(|(_, is_dir)| *is_dir || !folders_only)
        .collect();
    all.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    let lower = prefix.to_lowercase();
    let matching: Vec<(String, bool)> = all.iter().filter(|(n, _)| n.to_lowercase().starts_with(&lower)).cloned().collect();
    let (list, keep_name) = if matching.is_empty() && folders_only { (all, true) } else { (matching, false) };
    list.into_iter()
        .take(8)
        .map(|(name, is_dir)| {
            let tail = if keep_name { prefix } else { "" };
            let completion = if is_dir { format!("{dir_part}{name}/{tail}") } else { format!("{dir_part}{name}") };
            Suggestion { name, is_dir, completion }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("textedit-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trip_keeps_line_endings() {
        let dir = temp_dir("crlf");
        let path = dir.join("dos.txt");
        std::fs::write(&path, "one\r\ntwo\r\n").unwrap();
        let doc = load(&path).unwrap();
        assert!(doc.crlf);
        assert_eq!(doc.text, "one\ntwo\n");
        save(&path, &format!("{}three\n", doc.text), doc.crlf).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\r\ntwo\r\nthree\r\n");

        let unix = dir.join("unix.txt");
        save(&unix, "a\nb", false).unwrap();
        let doc = load(&unix).unwrap();
        assert!(!doc.crlf);
        assert_eq!(doc.text, "a\nb");
        // No temporary files are left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    }

    #[test]
    fn invalid_utf8_and_bom() {
        let dir = temp_dir("utf8");
        let path = dir.join("odd.txt");
        std::fs::write(&path, b"\xef\xbb\xbfcaf\xe9").unwrap();
        assert_eq!(load(&path).unwrap().text, "caf\u{fffd}");
        assert!(save(&dir.join("missing/x.txt"), "x", false).is_err());
    }

    #[test]
    fn completions() {
        let dir = temp_dir("complete");
        std::fs::create_dir(dir.join("Projects")).unwrap();
        std::fs::create_dir(dir.join("photos")).unwrap();
        std::fs::create_dir(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("plan.txt"), "").unwrap();
        let base = format!("{}/", dir.display());

        let s = suggestions(&format!("{base}p"), false);
        let names: Vec<&str> = s.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["photos", "Projects", "plan.txt"]);
        assert_eq!(s[0].completion, format!("{base}photos/"));
        assert_eq!(s[2].completion, format!("{base}plan.txt"));

        // Saving: a file name that matches no folder lists every folder and keeps the name.
        let s = suggestions(&format!("{base}Untitled.txt"), true);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].completion, format!("{base}Projects/Untitled.txt"));

        assert_eq!(suggestions(&format!("{base}.h"), true)[0].name, ".hidden");
        assert!(expand("~/x").ends_with("x"));
        assert_eq!(contract(&expand("~/notes.txt")), "~/notes.txt");
    }
}
