//! Renders the icons in `assets/icons/apps/` into an installable hicolor tree:
//! `<out>/hicolor/<n>x<n>/apps/y7ya-<name>.png` and `<out>/hicolor/scalable/apps/`.
//! Also writes `<out>/contact-sheet.png` (every icon at 512, 128, 32 and 16 on light
//! and dark) and `<out>/small-sizes.png` (16, 24 and 32 magnified, for pixel review).
//!
//! `cargo run -p render-icons [-- <out dir>]`; the default is `target/icons`.
//! Sizes up to 32px render from `assets/icons/apps/small/<name>.svg` when one exists,
//! a simplified drawing for tiny sizes. `*-symbolic.svg` glyphs are only copied.

use resvg::tiny_skia::{Color, FilterQuality, Pixmap, PixmapPaint, Transform};
use resvg::usvg;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

const SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];
/// Largest size drawn from the `small/` variant.
const SMALL: u32 = 32;
/// Window backgrounds from docs/design.md.
const LIGHT: [u8; 3] = [0xf2, 0xf2, 0xf4];
const DARK: [u8; 3] = [0x1e, 0x1e, 0x1e];
/// Symbolic glyphs are drawn in this color; the sheet swaps it for white on dark.
const SYMBOLIC_INK: &str = "#2e3436";

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Icon {
    name: String,
    /// One render per entry of `SIZES`.
    renders: Vec<Pixmap>,
}

/// The repository root.
fn root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

fn main() -> Result<()> {
    let root = root();
    let src = root.join("assets/icons/apps");
    let out = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| root.join("target/icons"));

    let scalable = out.join("hicolor/scalable/apps");
    fs::create_dir_all(&scalable)?;
    let mut icons = Vec::new();
    let mut glyphs = Vec::new();
    for name in svg_names(&src)? {
        let svg = src.join(format!("{name}.svg"));
        fs::copy(&svg, scalable.join(format!("y7ya-{name}.svg")))?;
        if name.ends_with("-symbolic") {
            glyphs.push(fs::read_to_string(&svg)?);
            continue;
        }
        let full = load(&fs::read_to_string(&svg)?)?;
        let small = src.join("small").join(format!("{name}.svg"));
        let small = if small.exists() { Some(load(&fs::read_to_string(&small)?)?) } else { None };
        let mut renders = Vec::new();
        for size in SIZES {
            let tree = if size <= SMALL { small.as_ref().unwrap_or(&full) } else { &full };
            let px = render(tree, size)?;
            let dir = out.join(format!("hicolor/{size}x{size}/apps"));
            fs::create_dir_all(&dir)?;
            px.save_png(dir.join(format!("y7ya-{name}.png")))?;
            renders.push(px);
        }
        icons.push(Icon { name, renders });
    }

    contact_sheet(&icons, &glyphs)?.save_png(out.join("contact-sheet.png"))?;
    small_sizes(&icons)?.save_png(out.join("small-sizes.png"))?;
    let names: Vec<_> = icons.iter().map(|i| format!("y7ya-{}", i.name)).collect();
    println!("{} icons, {} glyphs → {}", names.len(), glyphs.len(), out.display());
    println!("{}", names.join(" "));
    Ok(())
}

/// The stems of the `.svg` files in `dir`, sorted.
fn svg_names(dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "svg"))
        .filter_map(|p| p.file_stem()?.to_str().map(String::from))
        .collect();
    names.sort();
    Ok(names)
}

fn load(svg: &str) -> Result<usvg::Tree> {
    Ok(usvg::Tree::from_str(svg, &usvg::Options::default())?)
}

/// Renders `tree` scaled to `size` pixels square.
fn render(tree: &usvg::Tree, size: u32) -> Result<Pixmap> {
    let mut px = Pixmap::new(size, size).ok_or("bad size")?;
    let k = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(tree, Transform::from_scale(k, k), &mut px.as_mut());
    Ok(px)
}

fn rgb([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgba8(r, g, b, 0xff)
}

fn blit(sheet: &mut Pixmap, px: &Pixmap, x: u32, y: u32, zoom: u32) {
    let paint = PixmapPaint { quality: FilterQuality::Nearest, ..Default::default() };
    let t = Transform::from_row(zoom as f32, 0.0, 0.0, zoom as f32, x as f32, y as f32);
    sheet.draw_pixmap(0, 0, px.as_ref(), &paint, t, None);
}

/// One row per icon: 512, 128, 32 and 16 on light, then the same on dark. A last
/// row shows the symbolic glyphs at 64, 32 and 16.
fn contact_sheet(icons: &[Icon], glyphs: &[String]) -> Result<Pixmap> {
    const SHOWN: [u32; 4] = [512, 128, 32, 16];
    const PAD: u32 = 32;
    let panel = PAD + SHOWN.iter().map(|s| s + PAD).sum::<u32>();
    let row = 512 + PAD;
    let glyph_row = if glyphs.is_empty() { 0 } else { 64 + 2 * PAD };
    let mut sheet = Pixmap::new(2 * panel, PAD + icons.len() as u32 * row + glyph_row).ok_or("empty sheet")?;
    sheet.fill(rgb(LIGHT));
    let mut dark = Pixmap::new(panel, sheet.height()).ok_or("empty sheet")?;
    dark.fill(rgb(DARK));
    blit(&mut sheet, &dark, panel, 0, 1);

    for (i, icon) in icons.iter().enumerate() {
        let top = PAD + i as u32 * row;
        for side in [0, panel] {
            let mut x = side + PAD;
            for size in SHOWN {
                let px = &icon.renders[SIZES.iter().position(|&s| s == size).unwrap()];
                blit(&mut sheet, px, x, top + (512 - size) / 2, 1);
                x += size + PAD;
            }
        }
    }
    let top = PAD + icons.len() as u32 * row + PAD;
    for (side, ink) in [(0, SYMBOLIC_INK), (panel, "#ffffff")] {
        let mut x = side + PAD;
        for glyph in glyphs {
            let tree = load(&glyph.replace(SYMBOLIC_INK, ink))?;
            for size in [64, 32, 16] {
                blit(&mut sheet, &render(&tree, size)?, x, top + (64 - size) / 2, 1);
                x += size + PAD / 2;
            }
            x += PAD;
        }
    }
    Ok(sheet)
}

/// Every icon at 16, 24 and 32 magnified 8, 6 and 4 times (each 192px square), light
/// then dark, so tiny-size legibility can be judged pixel by pixel.
fn small_sizes(icons: &[Icon]) -> Result<Pixmap> {
    const SHOWN: [(u32, u32); 3] = [(16, 8), (24, 6), (32, 4)];
    const CELL: u32 = 192 + 16;
    let w = 16 + 2 * SHOWN.len() as u32 * CELL + 16;
    let mut sheet = Pixmap::new(w, 16 + icons.len() as u32 * CELL).ok_or("empty sheet")?;
    sheet.fill(rgb(LIGHT));
    let mut dark = Pixmap::new(w / 2, sheet.height()).ok_or("empty sheet")?;
    dark.fill(rgb(DARK));
    blit(&mut sheet, &dark, w / 2, 0, 1);
    for (i, icon) in icons.iter().enumerate() {
        for side in [0, w / 2] {
            for (k, (size, zoom)) in SHOWN.into_iter().enumerate() {
                let px = &icon.renders[SIZES.iter().position(|&s| s == size).unwrap()];
                blit(&mut sheet, px, side + 16 + k as u32 * CELL, 16 + i as u32 * CELL, zoom);
            }
        }
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_parse() {
        let src = root().join("assets/icons/apps");
        for dir in [src.clone(), src.join("small")] {
            for name in svg_names(&dir).unwrap() {
                let svg = fs::read_to_string(dir.join(format!("{name}.svg"))).unwrap();
                assert!(load(&svg).is_ok(), "{name}.svg doesn't parse");
            }
        }
    }

    #[test]
    fn every_desktop_icon_has_a_source() {
        let src = root().join("assets/icons/apps");
        for app in fs::read_dir(root().join("apps")).unwrap().flatten() {
            for entry in fs::read_dir(app.path()).unwrap().flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "desktop") {
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap();
                let icon = text.lines().find_map(|l| l.strip_prefix("Icon=")).unwrap_or_default();
                let name = icon.strip_prefix("y7ya-").unwrap_or_else(|| panic!("{}: Icon={icon}", path.display()));
                assert!(src.join(format!("{name}.svg")).exists(), "{}: no {name}.svg", path.display());
            }
        }
    }
}
