//! The pointer, drawn by us: a client's own cursor surface, or a named one
//! from the user's xcursor theme.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::utils::{Logical, Point, Transform};
use smithay::wayland::compositor::with_states;

use crate::render::{self, DeskElement};
use crate::state::Desk;

/// The pointer: a client's own cursor surface, or one from the cursor theme.
pub fn elements(desk: &Desk, renderer: &mut GlesRenderer, cursors: &mut Cursors, scale: f64) -> Vec<DeskElement> {
    let pointer = desk.seat.get_pointer().unwrap();
    let pos = pointer.current_location();
    let mut out = Vec::new();
    match &desk.cursor_status {
        CursorImageStatus::Hidden => {}
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(surface, |states| {
                states
                    .data_map
                    .get::<CursorImageSurfaceData>()
                    .map(|d| d.lock().unwrap().hotspot)
                    .unwrap_or_default()
            });
            let at = (pos - hotspot.to_f64()).to_physical_precise_round(scale);
            let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(renderer, surface, at, scale, 1.0, Kind::Cursor);
            out.extend(elems.into_iter().map(|e| DeskElement::Surface(render::unmoved(e, scale))));
        }
        CursorImageStatus::Named(icon) => {
            let int_scale = scale.ceil() as i32;
            if let Some((buf, hot)) = cursors.get(*icon, int_scale) {
                let at: Point<f64, Logical> = pos - hot.to_f64();
                let loc = at.to_physical_precise_round::<f64, i32>(scale).to_f64();
                if let Ok(elem) = MemoryRenderBufferRenderElement::from_buffer(renderer, loc, &buf, None, None, None, Kind::Cursor) {
                    out.push(DeskElement::Text(render::unmoved(elem, scale)));
                }
            }
        }
    }
    out
}

/// Cursor images from the user's xcursor theme.
pub struct Cursors {
    theme: xcursor::CursorTheme,
    size: u32,
    cache: std::collections::HashMap<(CursorIcon, i32), Option<(MemoryRenderBuffer, Point<i32, Logical>)>>,
}

impl Cursors {
    pub fn load() -> Self {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| kde_cursor_theme().unwrap_or_else(|| "default".into()));
        let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
        Cursors { theme: xcursor::CursorTheme::load(&name), size, cache: Default::default() }
    }

    fn get(&mut self, icon: CursorIcon, scale: i32) -> Option<(MemoryRenderBuffer, Point<i32, Logical>)> {
        let (theme, size) = (&self.theme, self.size);
        self.cache
            .entry((icon, scale))
            .or_insert_with(|| {
                let names = std::iter::once(icon.name()).chain(icon.alt_names().iter().copied()).chain(["default", "left_ptr"]);
                let path = names.filter_map(|n| theme.load_icon(n)).next()?;
                let data = std::fs::read(path).ok()?;
                let images = xcursor::parser::parse_xcursor(&data)?;
                let want = size * scale as u32;
                let img = images.iter().min_by_key(|i| (i.size as i64 - want as i64).abs())?;
                // RGBA (straight) → premultiplied BGRA.
                let px: Vec<u8> = img
                    .pixels_rgba
                    .chunks_exact(4)
                    .flat_map(|p| {
                        let a = p[3] as u32;
                        [(p[2] as u32 * a / 255) as u8, (p[1] as u32 * a / 255) as u8, (p[0] as u32 * a / 255) as u8, p[3]]
                    })
                    .collect();
                let img_scale = (img.size / size).max(1) as i32;
                let buf = MemoryRenderBuffer::from_slice(
                    &px,
                    Fourcc::Argb8888,
                    (img.width as i32, img.height as i32),
                    img_scale,
                    Transform::Normal,
                    None,
                );
                let hot = Point::from((img.xhot as i32 / img_scale, img.yhot as i32 / img_scale));
                Some((buf, hot))
            })
            .clone()
    }
}

/// The cursor theme set in KDE, if any.
fn kde_cursor_theme() -> Option<String> {
    let path = dirs::config_dir()?.join("kcminputrc");
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| l.strip_prefix("cursorTheme=").map(str::to_string))
}
