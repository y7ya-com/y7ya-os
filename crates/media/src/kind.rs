//! File kinds from names, with Finder-style descriptions ("PNG image").

/// Broad categories, which pick the icon and how a file previews.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Folder,
    /// Decodable by `ui::Image`.
    Image,
    /// Other images (RAW, SVG, HEIC…): an icon, no thumbnail.
    OtherImage,
    Pdf,
    Text,
    Code,
    Audio,
    Video,
    Archive,
    Document,
    Spreadsheet,
    Presentation,
    Font,
    Executable,
    DiskImage,
    Other,
}

impl Kind {
    /// Whether Quick Look shows the file's text.
    pub fn is_textual(self) -> bool {
        matches!(self, Kind::Text | Kind::Code)
    }

    /// Whether a real thumbnail can stand in for the icon.
    pub fn has_thumbnail(self) -> bool {
        matches!(self, Kind::Image | Kind::Pdf)
    }
}

/// The file's extension, lowercased; none for dotfiles like `.bashrc`.
pub fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty() && ext.len() <= 8).then(|| ext.to_lowercase())
}

/// The kind and the description Finder would show in its Kind column.
pub fn describe(name: &str, is_dir: bool) -> (Kind, String) {
    if is_dir {
        let app = name.ends_with(".app");
        return (Kind::Folder, if app { "Application".into() } else { "Folder".into() });
    }
    let lower = name.to_lowercase();
    match lower.as_str() {
        "readme" | "license" | "copying" | "authors" | "changelog" | "todo" | "notes" => {
            return (Kind::Text, "Plain Text Document".into());
        }
        "makefile" | "dockerfile" | "cmakelists.txt" | "pkgbuild" | "justfile" => return (Kind::Code, "Script".into()),
        _ => {}
    }
    let Some(ext) = extension(name) else { return (Kind::Other, "Document".into()) };
    let (kind, desc): (Kind, &str) = match ext.as_str() {
        "png" => (Kind::Image, "PNG image"),
        "jpg" | "jpeg" => (Kind::Image, "JPEG image"),
        "gif" => (Kind::Image, "GIF image"),
        "webp" => (Kind::Image, "WebP image"),
        "bmp" => (Kind::Image, "Windows BMP image"),
        "svg" => (Kind::OtherImage, "SVG image"),
        "heic" | "heif" => (Kind::OtherImage, "HEIF image"),
        "tif" | "tiff" => (Kind::OtherImage, "TIFF image"),
        "ico" => (Kind::OtherImage, "Windows icon image"),
        "avif" => (Kind::OtherImage, "AVIF image"),
        "psd" => (Kind::OtherImage, "Adobe Photoshop file"),
        "xcf" => (Kind::OtherImage, "GIMP image"),
        "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" => (Kind::OtherImage, "Raw image"),
        "pdf" => (Kind::Pdf, "PDF document"),
        "txt" | "text" | "log" => (Kind::Text, "Plain Text Document"),
        "md" | "markdown" => (Kind::Text, "Markdown document"),
        "rtf" => (Kind::Text, "Rich Text Document"),
        "csv" => (Kind::Text, "Comma-separated values"),
        "tsv" => (Kind::Text, "Tab-separated values"),
        "rs" => (Kind::Code, "Rust source"),
        "c" => (Kind::Code, "C source"),
        "h" | "hpp" => (Kind::Code, "C header"),
        "cc" | "cpp" | "cxx" => (Kind::Code, "C++ source"),
        "py" => (Kind::Code, "Python script"),
        "js" | "mjs" | "cjs" => (Kind::Code, "JavaScript script"),
        "ts" | "tsx" => (Kind::Code, "TypeScript source"),
        "jsx" => (Kind::Code, "JSX source"),
        "go" => (Kind::Code, "Go source"),
        "java" => (Kind::Code, "Java source"),
        "kt" | "kts" => (Kind::Code, "Kotlin source"),
        "swift" => (Kind::Code, "Swift source"),
        "rb" => (Kind::Code, "Ruby script"),
        "lua" => (Kind::Code, "Lua script"),
        "sh" | "bash" | "zsh" | "fish" => (Kind::Code, "Shell script"),
        "html" | "htm" => (Kind::Code, "HTML document"),
        "css" | "scss" => (Kind::Code, "CSS style sheet"),
        "json" => (Kind::Code, "JSON document"),
        "toml" => (Kind::Code, "TOML document"),
        "yaml" | "yml" => (Kind::Code, "YAML document"),
        "xml" => (Kind::Code, "XML document"),
        "ini" | "conf" | "cfg" | "desktop" | "service" => (Kind::Code, "Configuration file"),
        "sql" => (Kind::Code, "SQL script"),
        "wgsl" | "glsl" | "hlsl" => (Kind::Code, "Shader source"),
        "lock" => (Kind::Code, "Lock file"),
        "mp3" => (Kind::Audio, "MP3 audio"),
        "m4a" | "aac" => (Kind::Audio, "MPEG-4 audio"),
        "flac" => (Kind::Audio, "FLAC audio"),
        "wav" => (Kind::Audio, "Waveform audio"),
        "ogg" | "oga" | "opus" => (Kind::Audio, "Ogg audio"),
        "aiff" | "aif" => (Kind::Audio, "AIFF audio"),
        "mp4" | "m4v" => (Kind::Video, "MPEG-4 movie"),
        "mov" => (Kind::Video, "QuickTime movie"),
        "mkv" => (Kind::Video, "Matroska video"),
        "webm" => (Kind::Video, "WebM video"),
        "avi" => (Kind::Video, "AVI movie"),
        "zip" => (Kind::Archive, "ZIP archive"),
        "gz" | "tgz" => (Kind::Archive, "Gzip archive"),
        "xz" | "txz" => (Kind::Archive, "XZ archive"),
        "zst" => (Kind::Archive, "Zstandard archive"),
        "bz2" => (Kind::Archive, "Bzip2 archive"),
        "tar" => (Kind::Archive, "Tar archive"),
        "7z" => (Kind::Archive, "7-Zip archive"),
        "rar" => (Kind::Archive, "RAR archive"),
        "deb" | "rpm" => (Kind::Archive, "Software package"),
        "doc" | "docx" => (Kind::Document, "Microsoft Word document"),
        "odt" => (Kind::Document, "OpenDocument text"),
        "pages" => (Kind::Document, "Pages document"),
        "epub" => (Kind::Document, "EPUB book"),
        "xls" | "xlsx" => (Kind::Spreadsheet, "Microsoft Excel spreadsheet"),
        "ods" => (Kind::Spreadsheet, "OpenDocument spreadsheet"),
        "numbers" => (Kind::Spreadsheet, "Numbers spreadsheet"),
        "ppt" | "pptx" => (Kind::Presentation, "Microsoft PowerPoint presentation"),
        "odp" => (Kind::Presentation, "OpenDocument presentation"),
        "key" => (Kind::Presentation, "Keynote presentation"),
        "ttf" | "otf" | "woff" | "woff2" => (Kind::Font, "Font"),
        "appimage" | "exe" | "bin" | "run" => (Kind::Executable, "Application"),
        "iso" | "img" | "dmg" => (Kind::DiskImage, "Disk image"),
        _ => (Kind::Other, ""),
    };
    let desc = if desc.is_empty() { format!("{} file", ext.to_uppercase()) } else { desc.to_string() };
    (kind, desc)
}

pub fn kind_of(name: &str, is_dir: bool) -> Kind {
    describe(name, is_dir).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(describe("Photo.JPG", false), (Kind::Image, "JPEG image".into()));
        assert_eq!(kind_of("main.rs", false), Kind::Code);
        assert_eq!(kind_of(".bashrc", false), Kind::Other);
        assert_eq!(describe("thing.xyz", false).1, "XYZ file");
        assert_eq!(kind_of("Music", true), Kind::Folder);
        assert_eq!(extension("archive.tar.gz").as_deref(), Some("gz"));
    }
}
