//! App icons for the switcher: app_id → .desktop file → icon theme image.

use std::path::{Path, PathBuf};

/// An icon as premultiplied BGRA pixels.
pub struct Icon {
    pub px: Vec<u8>,
    pub size: u32,
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![dirs::data_dir().unwrap_or_default()];
    let sys = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    dirs
}

/// The `Icon=` and `Name=` of the app's .desktop file.
pub fn desktop_entry(app_id: &str) -> Option<(String, String)> {
    let wanted = app_id.to_lowercase();
    for dir in data_dirs() {
        let Ok(entries) = std::fs::read_dir(dir.join("applications")) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_lowercase();
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let wm_class = field(&text, "StartupWMClass").map(|s| s.to_lowercase());
            // org.kde.konsole.desktop matches app_id "org.kde.konsole" or "konsole".
            let hit = stem == wanted || stem.rsplit('.').next() == Some(&wanted) || wm_class.as_deref() == Some(&wanted);
            if hit {
                let icon = field(&text, "Icon").unwrap_or_default();
                let name = field(&text, "Name").unwrap_or_else(|| app_id.to_string());
                return Some((icon, name));
            }
        }
    }
    None
}

fn field(text: &str, key: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry {
            if let Some(v) = line.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn find_icon_file(name: &str) -> Option<PathBuf> {
    if Path::new(name).is_absolute() {
        return Path::new(name).exists().then(|| PathBuf::from(name));
    }
    let themes = ["hicolor", "breeze", "breeze-dark", "Adwaita"];
    let sizes = ["256x256", "512x512", "128x128", "scalable", "96x96", "64x64", "48x48"];
    let mut roots: Vec<PathBuf> = vec![dirs::home_dir().unwrap_or_default().join(".icons")];
    roots.extend(data_dirs().into_iter().map(|d| d.join("icons")));
    // Freedesktop themes use `48x48/apps/`, Breeze uses `apps/48/`.
    let breeze_sizes = ["256", "128", "96", "64", "48"];
    for root in &roots {
        for theme in themes {
            let dirs = sizes
                .iter()
                .map(|size| root.join(theme).join(size).join("apps"))
                .chain(breeze_sizes.iter().map(|size| root.join(theme).join("apps").join(size)));
            for dir in dirs {
                for ext in ["png", "svg"] {
                    let p = dir.join(format!("{name}.{ext}"));
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    data_dirs()
        .into_iter()
        .flat_map(|d| ["png", "svg"].map(|e| d.join("pixmaps").join(format!("{name}.{e}"))))
        .find(|p| p.exists())
}

/// Loads the app's icon at `size` pixels square.
pub fn load(app_id: &str, size: u32) -> Option<Icon> {
    let (icon, _) = desktop_entry(app_id)?;
    let path = find_icon_file(if icon.is_empty() { app_id } else { &icon })?;
    let rgba = if path.extension().is_some_and(|e| e == "svg") { svg(&path, size)? } else { png(&path, size)? };
    // Straight RGBA → premultiplied BGRA.
    let px = rgba
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as u32;
            [(p[2] as u32 * a / 255) as u8, (p[1] as u32 * a / 255) as u8, (p[0] as u32 * a / 255) as u8, p[3]]
        })
        .collect();
    Some(Icon { px, size })
}

fn svg(path: &Path, size: u32) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let s = tree.size();
    let k = size as f32 / s.width().max(s.height());
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k, k), &mut pixmap.as_mut());
    // tiny-skia is premultiplied; undo that so both paths agree.
    Some(
        pixmap
            .data()
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3] as u32;
                let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255) / a).min(255) as u8 };
                [un(p[0]), un(p[1]), un(p[2]), p[3]]
            })
            .collect(),
    )
}

fn png(path: &Path, size: u32) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..(w * h * 4) as usize].to_vec(),
        png::ColorType::Rgb => buf[..(w * h * 3) as usize].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    // Nearest-neighbor resample to the requested size (icons are already near it).
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let sx = x * w / size;
            let sy = y * h / size;
            let s = ((sy * w + sx) * 4) as usize;
            let d = ((y * size + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    Some(out)
}
