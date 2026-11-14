//! PDF pages rendered by poppler (`pdftoppm`) or MuPDF (`mutool`), whichever
//! is installed. Without either, PDFs get an icon instead of pages.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use ui::Image;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Poppler,
    Mupdf,
}

/// The renderer to use, found once per process.
pub fn backend() -> Option<Backend> {
    static FOUND: OnceLock<Option<Backend>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        if crate::which("pdftoppm").is_some() {
            Some(Backend::Poppler)
        } else if crate::which("mutool").is_some() {
            Some(Backend::Mupdf)
        } else {
            None
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub pages: u32,
    /// Size of the first page in points.
    pub page_size: (f32, f32),
}

fn output(cmd: &mut Command) -> Option<Vec<u8>> {
    let out = cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

/// Page count and size, from `pdfinfo` or `mutool info`.
pub fn info(path: &Path) -> Option<Info> {
    let text = match backend()? {
        Backend::Poppler if crate::which("pdfinfo").is_some() => output(Command::new("pdfinfo").arg(path))?,
        Backend::Poppler => return Some(Info { pages: 1, page_size: (612.0, 792.0) }),
        Backend::Mupdf => output(Command::new("mutool").arg("info").arg(path))?,
    };
    parse_info(&String::from_utf8_lossy(&text))
}

fn parse_info(text: &str) -> Option<Info> {
    let mut pages = None;
    let mut size = (612.0, 792.0);
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("Pages:") {
            pages = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("Page size:") {
            // "612 x 792 pts (letter)"
            let mut nums = v.split_whitespace().filter_map(|w| w.parse::<f32>().ok());
            if let (Some(w), Some(h)) = (nums.next(), nums.next()) {
                size = (w, h);
            }
        } else if line.starts_with("<</MediaBox") || line.contains("[0 0 ") {
            // mutool: "1	(5 0 R):	[ 0 0 612 792 ]"
            let nums: Vec<f32> = line
                .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                .filter_map(|w| w.parse().ok())
                .collect();
            if nums.len() >= 4 {
                let n = nums.len();
                size = (nums[n - 2] - nums[n - 4], nums[n - 1] - nums[n - 3]);
            }
        }
    }
    Some(Info { pages: pages?, page_size: size })
}

/// Renders page `page` (from 1) with its longer side `max_px` pixels.
pub fn render_page(path: &Path, page: u32, max_px: u32) -> Option<Image> {
    let png = match backend()? {
        Backend::Poppler => output(
            Command::new("pdftoppm")
                .args(["-png", "-singlefile", "-f", &page.to_string(), "-l", &page.to_string()])
                .args(["-scale-to", &max_px.to_string()])
                .arg(path),
        )?,
        Backend::Mupdf => output(
            Command::new("mutool")
                .args(["draw", "-q", "-F", "png", "-o", "-"])
                .args(["-w", &max_px.to_string(), "-h", &max_px.to_string()])
                .arg(path)
                .arg(page.to_string()),
        )?,
    };
    Image::decode(&png, max_px.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pdfinfo() {
        let text = "Producer:        GPL Ghostscript\nPages:           3\nPage size:       595.276 x 841.89 pts (A4)\n";
        let info = parse_info(text).unwrap();
        assert_eq!(info.pages, 3);
        assert!((info.page_size.0 - 595.276).abs() < 0.01);
        assert!(parse_info("nothing here").is_none());
    }
}
