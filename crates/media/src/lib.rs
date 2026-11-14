//! What a file is and what it looks like: kinds by extension, decoded image
//! and PDF previews, text excerpts, and a worker pool to load them off the UI
//! thread. Shared by Files (thumbnails, Quick Look) and Preview.

pub mod kind;
pub mod loader;
pub mod pdf;

use std::io::Read;
use std::path::Path;

pub use kind::{Kind, kind_of};
pub use loader::Loader;

/// The first `max_bytes` of a text file, or `None` if it looks binary.
pub fn text_excerpt(path: &Path, max_bytes: usize) -> Option<String> {
    let mut buf = Vec::with_capacity(max_bytes.min(1 << 20));
    std::fs::File::open(path).ok()?.take(max_bytes as u64).read_to_end(&mut buf).ok()?;
    if buf.contains(&0) {
        return None;
    }
    let text = match String::from_utf8(buf) {
        Ok(s) => s,
        // A multibyte character cut off at the end is fine; anything else isn't text.
        Err(e) if e.utf8_error().error_len().is_none() => {
            let valid = e.utf8_error().valid_up_to();
            let mut bytes = e.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).ok()?
        }
        Err(_) => return None,
    };
    Some(text.replace("\r\n", "\n").replace('\t', "    "))
}

/// Whether a file without a telling extension holds text.
pub fn sniff_text(path: &Path) -> bool {
    text_excerpt(path, 4096).is_some_and(|s| !s.is_empty())
}

/// Finds `program` on `PATH`.
pub fn which(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(program)).find(|p| {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpts() {
        let dir = std::env::temp_dir().join(format!("media-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = dir.join("a.txt");
        std::fs::write(&text, "héllo\r\nworld").unwrap();
        assert_eq!(text_excerpt(&text, 100).unwrap(), "héllo\nworld");
        // Cut in the middle of "é": still text.
        assert_eq!(text_excerpt(&text, 2).unwrap(), "h");
        let bin = dir.join("b.bin");
        std::fs::write(&bin, [0x89, b'P', 0, 1]).unwrap();
        assert!(text_excerpt(&bin, 100).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
