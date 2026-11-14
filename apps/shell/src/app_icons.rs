//! App icons: a `.desktop` file's `Icon=` looked up in the icon themes
//! (hicolor and Breeze layouts, PNG or SVG), decoded off the UI thread.
//!
//! The user's own icons (`~/.local/share/icons`, where y7ya-os installs its
//! apps' icons) win over the system theme's.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use ui::widgets::image::Image;

/// Pixels per side icons are decoded at: sharp at 48pt on a 2x screen.
const SIZE: u32 = 96;

/// Icon theme roots, most preferred first.
pub fn roots() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let mut roots = vec![data_home.join("icons"), home.join(".icons")];
    roots.extend(system.split(':').filter(|s| !s.is_empty()).map(|d| PathBuf::from(d).join("icons")));
    roots
}

/// Finds the file for icon `name` under `roots`. Every root's themes are
/// tried before the next root's, so a user-installed icon beats the system
/// theme's.
pub fn find(name: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return path.exists().then(|| path.to_path_buf());
    }
    // Some entries say `Icon=foo.png`.
    let name = name.strip_suffix(".png").or_else(|| name.strip_suffix(".svg")).unwrap_or(name);
    const THEMES: [&str; 4] = ["hicolor", "breeze", "breeze-dark", "Adwaita"];
    // Freedesktop themes use `48x48/apps`, Breeze uses `apps/48`. Big first.
    const SIZES: [&str; 8] = ["scalable", "256x256", "512x512", "128x128", "96x96", "64x64", "48x48", "32x32"];
    const BREEZE: [&str; 4] = ["48", "64", "32", "22"];
    for root in roots {
        for theme in THEMES {
            let base = root.join(theme);
            if !base.is_dir() {
                continue;
            }
            let dirs = SIZES
                .iter()
                .map(|s| base.join(s).join("apps"))
                .chain(BREEZE.iter().map(|s| base.join("apps").join(s)));
            for dir in dirs {
                for ext in ["svg", "png"] {
                    let p = dir.join(format!("{name}.{ext}"));
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    roots
        .iter()
        .filter_map(|r| r.parent())
        .flat_map(|d| ["png", "svg"].map(|e| d.join("pixmaps").join(format!("{name}.{e}"))))
        .find(|p| p.exists())
}

/// Decodes an icon file to a square image.
pub fn decode(path: &Path) -> Option<Image> {
    if path.extension().is_some_and(|e| e == "svg") {
        let data = std::fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(SIZE, SIZE)?;
        let s = tree.size();
        let k = SIZE as f32 / s.width().max(s.height());
        let (dx, dy) = ((SIZE as f32 - s.width() * k) / 2.0, (SIZE as f32 - s.height() * k) / 2.0);
        resvg::render(&tree, resvg::tiny_skia::Transform::from_row(k, 0.0, 0.0, k, dx, dy), &mut pixmap.as_mut());
        // tiny-skia is premultiplied; `Image` wants straight alpha.
        let rgba = pixmap
            .data()
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3] as u32;
                let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
                [un(p[0]), un(p[1]), un(p[2]), p[3]]
            })
            .collect();
        Some(Image::from_rgba(SIZE, SIZE, rgba))
    } else {
        Image::load(path, SIZE)
    }
}

/// Icons by name, loaded in the background. `get` returns `None` until an
/// icon is ready (or if there isn't one); the waker runs when one arrives.
#[derive(Clone)]
pub struct IconCache {
    loaded: Arc<Mutex<HashMap<String, Option<Image>>>>,
    asked: Arc<Mutex<HashSet<String>>>,
    tx: Sender<String>,
}

impl IconCache {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> IconCache {
        let loaded: Arc<Mutex<HashMap<String, Option<Image>>>> = Arc::default();
        let (tx, rx) = channel::<String>();
        let out = loaded.clone();
        std::thread::Builder::new()
            .name("icons".into())
            .spawn(move || {
                let roots = roots();
                for name in rx {
                    let image = find(&name, &roots).and_then(|p| decode(&p));
                    out.lock().unwrap().insert(name, image);
                    wake();
                }
            })
            .expect("icon thread");
        IconCache { loaded, asked: Arc::default(), tx }
    }

    pub fn get(&self, name: &str) -> Option<Image> {
        if name.is_empty() {
            return None;
        }
        if let Some(found) = self.loaded.lock().unwrap().get(name) {
            return found.clone();
        }
        if self.asked.lock().unwrap().insert(name.to_string()) {
            let _ = self.tx.send(name.to_string());
        }
        None
    }

    /// Loads `name` now, on this thread (for snapshots).
    pub fn load_now(&self, name: &str) {
        let image = find(name, &roots()).and_then(|p| decode(&p));
        self.asked.lock().unwrap().insert(name.to_string());
        self.loaded.lock().unwrap().insert(name.to_string(), image);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="24"><rect width="48" height="24" fill="red"/></svg>"#;

    #[test]
    fn finds_icons_in_theme_layouts() {
        let base = std::env::temp_dir().join(format!("y7-icons-test-{}", std::process::id()));
        let (user, system) = (base.join("home/icons"), base.join("usr/icons"));
        let write = |p: PathBuf, data: &[u8]| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
        };
        write(system.join("breeze/apps/48/notes.svg"), SVG.as_bytes());
        write(system.join("hicolor/48x48/apps/files.png"), b"png");
        write(system.join("hicolor/scalable/apps/files.svg"), SVG.as_bytes());
        // The user's copy of `notes` wins over the system theme.
        write(user.join("hicolor/256x256/apps/notes.png"), b"png");
        write(base.join("usr/pixmaps/old.png"), b"png");
        let roots = [user.clone(), system.clone()];

        assert_eq!(find("notes", &roots), Some(user.join("hicolor/256x256/apps/notes.png")));
        assert_eq!(find("notes", &roots[1..]), Some(system.join("breeze/apps/48/notes.svg")));
        // Scalable beats a fixed size; `Icon=files.svg` works too.
        assert_eq!(find("files", &roots), Some(system.join("hicolor/scalable/apps/files.svg")));
        assert_eq!(find("files.svg", &roots), find("files", &roots));
        assert_eq!(find("old", &roots), Some(base.join("usr/pixmaps/old.png")));
        assert_eq!(find("missing", &roots), None);
        assert_eq!(find("", &roots), None);

        // SVGs render square, centered.
        let img = decode(&system.join("breeze/apps/48/notes.svg")).unwrap();
        assert_eq!((img.width(), img.height()), (SIZE, SIZE));
        std::fs::remove_dir_all(base).unwrap();
    }
}
