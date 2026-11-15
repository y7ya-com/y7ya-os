//! Recently used files, for Start's "Recent" section: the freedesktop
//! `recently-used.xbel` that GTK and KDE apps (and file pickers) write.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

#[derive(Clone, Debug, PartialEq)]
pub struct RecentFile {
    pub path: PathBuf,
    pub is_dir: bool,
    pub used: DateTime<Utc>,
}

/// `~/.local/share/recently-used.xbel`.
pub fn xbel_path() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(dirs::data_dir)?;
    Some(data.join("recently-used.xbel"))
}

/// The `limit` most recently used local files that still exist.
pub fn load(xbel: &Path, limit: usize) -> Vec<RecentFile> {
    let Ok(text) = std::fs::read_to_string(xbel) else { return Vec::new() };
    let mut all = parse(&text);
    all.sort_by(|a, b| b.used.cmp(&a.used));
    all.into_iter()
        .filter_map(|mut f| {
            let meta = std::fs::metadata(&f.path).ok()?;
            f.is_dir = meta.is_dir();
            Some(f)
        })
        .take(limit)
        .collect()
}

/// Every local `file://` bookmark in an xbel document, in file order.
pub fn parse(xbel: &str) -> Vec<RecentFile> {
    let mut out = Vec::new();
    for tag in xbel.split("<bookmark ").skip(1) {
        let tag = tag.split('>').next().unwrap_or("");
        let Some(href) = attr(tag, "href") else { continue };
        let Some(path) = href.strip_prefix("file://") else { continue };
        // The latest of when it was modified and visited.
        let used = ["modified", "visited", "added"]
            .iter()
            .filter_map(|a| attr(tag, a))
            .filter_map(|v| DateTime::parse_from_rfc3339(&v).ok())
            .map(|d| d.with_timezone(&Utc))
            .max();
        let Some(used) = used else { continue };
        out.push(RecentFile { path: PathBuf::from(percent_decode(path)), is_dir: false, used });
    }
    out
}

/// An attribute's value in a tag's text, with XML entities decoded.
fn attr(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!(" {name}=\"")).map(|i| i + name.len() + 3).or_else(|| {
        tag.starts_with(&format!("{name}=\"")).then_some(name.len() + 2)
    })?;
    let value = &tag[start..];
    let value = &value[..value.find('"')?];
    Some(value.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%' && i + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        match hex {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// "Just now", "12 min ago", "3 hr ago", "Yesterday", "Mon", "Nov 3".
pub fn ago(used: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - used).num_seconds().max(0);
    let local = used.with_timezone(&chrono::Local);
    let today = now.with_timezone(&chrono::Local).date_naive();
    match secs {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        _ if local.date_naive() == today => format!("{} hr ago", secs / 3600),
        _ if today.pred_opt() == Some(local.date_naive()) => "Yesterday".into(),
        _ if secs < 6 * 86400 => local.format("%A").to_string(),
        _ => local.format("%b %-d").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_xbel() {
        let xbel = r#"<?xml version="1.0"?>
<xbel version="1.0">
  <bookmark href="file:///home/a/My%20Notes.txt" added="2026-10-01T10:00:00Z" modified="2026-10-02T10:00:00Z" visited="2026-10-03T09:00:00.5Z">
    <info/>
  </bookmark>
  <bookmark href="https://example.com/?a=1&amp;b=2" added="2026-10-01T10:00:00Z" modified="2026-10-01T10:00:00Z" visited="2026-10-01T10:00:00Z"/>
  <bookmark href="file:///tmp/R&amp;D.pdf" added="2026-10-04T10:00:00Z" modified="2026-10-04T10:00:00Z" visited="2026-10-04T10:00:00Z"/>
</xbel>"#;
        let files = parse(xbel);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from("/home/a/My Notes.txt"));
        assert_eq!(files[0].used, DateTime::parse_from_rfc3339("2026-10-03T09:00:00.5Z").unwrap());
        assert_eq!(files[1].path, PathBuf::from("/tmp/R&D.pdf"));
        assert_eq!(percent_decode("%E2%9C%93ok%2"), "\u{2713}ok%2");
    }

    #[test]
    fn says_how_long_ago() {
        let now = DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(ago(now - chrono::Duration::seconds(20), now), "Just now");
        assert_eq!(ago(now - chrono::Duration::minutes(12), now), "12 min ago");
        assert_eq!(ago(now - chrono::Duration::days(30), now), (now - chrono::Duration::days(30)).with_timezone(&chrono::Local).format("%b %-d").to_string());
    }
}
