use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use cosmic_text::{
    Attrs, Buffer, CacheKey, Ellipsize, EllipsizeHeightLimit, Family, FeatureTag, FontFeatures, FontSystem, Metrics,
    Shaping, SwashCache, SwashContent, Wrap,
};

use super::geom::{Bounds, Color, Point};
use super::scene::Scene;

pub const ATLAS_SIZE: u32 = 2048;
pub const FONT_FAMILY: &str = "Inter";

const FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-Bold.ttf"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Weight {
    #[default]
    Regular,
    Medium,
    Semibold,
    Bold,
}

impl Weight {
    fn to_cosmic(self) -> cosmic_text::Weight {
        match self {
            Weight::Regular => cosmic_text::Weight::NORMAL,
            Weight::Medium => cosmic_text::Weight::MEDIUM,
            Weight::Semibold => cosmic_text::Weight::SEMIBOLD,
            Weight::Bold => cosmic_text::Weight::BOLD,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    /// Multiplier of `size`.
    pub line_height: f32,
    pub weight: Weight,
    pub color: Color,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: Color::BLACK }
    }
}

impl TextStyle {
    pub fn metrics(&self) -> Metrics {
        Metrics::new(self.size, (self.size * self.line_height).round())
    }

    pub fn attrs(&self) -> Attrs<'static> {
        Attrs::new().family(Family::Name(FONT_FAMILY)).weight(self.weight.to_cosmic())
    }
}

pub struct AtlasUpload {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
struct GlyphEntry {
    uv: [f32; 4],
    left: i32,
    top: i32,
    w: u32,
    h: u32,
    color: bool,
}

/// Rasterizes glyphs with swash and packs them into one RGBA texture using a
/// simple shelf packer. When it fills up, everything is dropped and re-rasterized.
pub struct GlyphAtlas {
    swash: SwashCache,
    entries: HashMap<CacheKey, Option<GlyphEntry>>,
    images: HashMap<u64, Option<[f32; 4]>>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    uploads: Vec<AtlasUpload>,
    pub overflowed: bool,
}

impl GlyphAtlas {
    fn new() -> Self {
        Self {
            swash: SwashCache::new(),
            entries: HashMap::new(),
            images: HashMap::new(),
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            uploads: Vec::new(),
            overflowed: false,
        }
    }

    pub fn reset(&mut self) {
        self.entries.clear();
        self.images.clear();
        self.shelf_x = 0;
        self.shelf_y = 0;
        self.shelf_h = 0;
        self.overflowed = false;
    }

    pub fn take_uploads(&mut self) -> Vec<AtlasUpload> {
        std::mem::take(&mut self.uploads)
    }

    fn get(&mut self, fonts: &mut FontSystem, key: CacheKey) -> Option<GlyphEntry> {
        if let Some(e) = self.entries.get(&key) {
            return *e;
        }
        let entry = self.rasterize(fonts, key);
        self.entries.insert(key, entry);
        entry
    }

    fn rasterize(&mut self, fonts: &mut FontSystem, key: CacheKey) -> Option<GlyphEntry> {
        let image = self.swash.get_image_uncached(fonts, key)?;
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            return None;
        }
        let (rgba, color) = match image.content {
            SwashContent::Mask => (image.data.iter().flat_map(|&a| [255, 255, 255, a]).collect::<Vec<u8>>(), false),
            SwashContent::Color => (
                image
                    .data
                    .chunks_exact(4)
                    .flat_map(|p| {
                        let a = p[3] as u32;
                        [(p[0] as u32 * a / 255) as u8, (p[1] as u32 * a / 255) as u8, (p[2] as u32 * a / 255) as u8, p[3]]
                    })
                    .collect(),
                true,
            ),
            SwashContent::SubpixelMask => (
                image
                    .data
                    .chunks_exact(4)
                    .flat_map(|p| [255, 255, 255, ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8])
                    .collect(),
                false,
            ),
        };

        Some(GlyphEntry {
            uv: self.alloc(w, h, rgba)?,
            left: image.placement.left,
            top: image.placement.top,
            w,
            h,
            color,
        })
    }
}

impl GlyphAtlas {
    /// Packs `w`×`h` premultiplied RGBA pixels and returns their UV rect.
    fn alloc(&mut self, w: u32, h: u32, rgba: Vec<u8>) -> Option<[f32; 4]> {
        // One pixel of padding keeps linear sampling from bleeding neighbors in.
        if self.shelf_x + w + 1 > ATLAS_SIZE {
            self.shelf_x = 0;
            self.shelf_y += self.shelf_h + 1;
            self.shelf_h = 0;
        }
        if self.shelf_y + h + 1 > ATLAS_SIZE {
            self.overflowed = true;
            return None;
        }
        let (x, y) = (self.shelf_x, self.shelf_y);
        self.shelf_x += w + 1;
        self.shelf_h = self.shelf_h.max(h);
        self.uploads.push(AtlasUpload { x, y, w, h, rgba });
        let s = ATLAS_SIZE as f32;
        Some([x as f32 / s, y as f32 / s, (x + w) as f32 / s, (y + h) as f32 / s])
    }

    /// Packs a straight-alpha RGBA image (at most 512×512) once per `key` and
    /// returns its UV rect, for drawing with [`Scene::glyph`] as a color glyph.
    /// Images are dropped with everything else when the atlas resets.
    pub fn image(&mut self, key: u64, w: u32, h: u32, rgba: &[u8]) -> Option<[f32; 4]> {
        if let Some(uv) = self.images.get(&key) {
            return *uv;
        }
        let ok = w > 0 && h > 0 && w <= 512 && h <= 512 && rgba.len() == (w * h * 4) as usize;
        let premultiplied = rgba
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3] as u32;
                [(p[0] as u32 * a / 255) as u8, (p[1] as u32 * a / 255) as u8, (p[2] as u32 * a / 255) as u8, p[3]]
            })
            .collect();
        let uv = if ok { self.alloc(w, h, premultiplied) } else { None };
        self.images.insert(key, uv);
        uv
    }
}

struct CachedBuffer {
    buffer: Buffer,
    used: u64,
}

pub struct TextSystem {
    pub fonts: FontSystem,
    pub atlas: GlyphAtlas,
    cache: HashMap<u64, CachedBuffer>,
    frame: u64,
}

impl TextSystem {
    pub fn new() -> Self {
        let mut fonts = FontSystem::new();
        for data in FONTS {
            fonts.db_mut().load_font_data(data.to_vec());
        }
        Self { fonts, atlas: GlyphAtlas::new(), cache: HashMap::new(), frame: 0 }
    }

    fn key(text: &str, style: &TextStyle, width: Option<f32>, truncate: bool, tabular: bool) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        tabular.hash(&mut h);
        style.size.to_bits().hash(&mut h);
        style.line_height.to_bits().hash(&mut h);
        style.weight.hash(&mut h);
        width.map(|w| w.round() as i32).hash(&mut h);
        truncate.hash(&mut h);
        h.finish()
    }

    /// Shapes and lays out `text`, reusing the result from earlier frames when possible.
    pub fn layout(&mut self, text: &str, style: &TextStyle, width: Option<f32>, truncate: bool) -> &Buffer {
        self.layout_with(text, style, width, truncate, false)
    }

    /// `layout`, optionally with tabular (fixed-width) figures so columns of
    /// numbers line up and counters don't jitter.
    pub fn layout_with(&mut self, text: &str, style: &TextStyle, width: Option<f32>, truncate: bool, tabular: bool) -> &Buffer {
        let key = Self::key(text, style, width, truncate, tabular);
        let frame = self.frame;
        let fonts = &mut self.fonts;
        let entry = self.cache.entry(key).or_insert_with(|| {
            let mut buffer = Buffer::new(fonts, style.metrics());
            if truncate {
                buffer.set_wrap(Wrap::None);
                buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
            } else {
                buffer.set_wrap(Wrap::WordOrGlyph);
            }
            buffer.set_size(width.map(|w| w.round()), None);
            let mut attrs = style.attrs();
            if tabular {
                let mut f = FontFeatures::new();
                f.enable(FeatureTag::new(b"tnum"));
                attrs = attrs.font_features(f);
            }
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(fonts, false);
            CachedBuffer { buffer, used: frame }
        });
        entry.used = frame;
        &entry.buffer
    }

    pub fn measure(&mut self, text: &str, style: &TextStyle, width: Option<f32>, truncate: bool) -> (f32, f32) {
        self.measure_with(text, style, width, truncate, false)
    }

    pub fn measure_with(&mut self, text: &str, style: &TextStyle, width: Option<f32>, truncate: bool, tabular: bool) -> (f32, f32) {
        let line_h = style.metrics().line_height;
        let buffer = self.layout_with(text, style, width, truncate, tabular);
        buffer_size(buffer, line_h)
    }

    pub fn paint(
        &mut self,
        scene: &mut Scene,
        text: &str,
        style: &TextStyle,
        width: Option<f32>,
        truncate: bool,
        origin: Point,
        scale: f32,
    ) {
        self.paint_with(scene, text, style, width, truncate, false, origin, scale)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn paint_with(
        &mut self,
        scene: &mut Scene,
        text: &str,
        style: &TextStyle,
        width: Option<f32>,
        truncate: bool,
        tabular: bool,
        origin: Point,
        scale: f32,
    ) {
        let key = Self::key(text, style, width, truncate, tabular);
        self.layout_with(text, style, width, truncate, tabular);
        let buffer = &self.cache[&key].buffer;
        draw_buffer(&mut self.fonts, &mut self.atlas, scene, buffer, origin, style.color, scale);
    }

    pub fn end_frame(&mut self) {
        let frame = self.frame;
        self.cache.retain(|_, c| frame.saturating_sub(c.used) < 3);
        self.frame += 1;
    }
}

pub fn buffer_size(buffer: &Buffer, fallback_line_h: f32) -> (f32, f32) {
    let mut w: f32 = 0.0;
    let mut h: f32 = 0.0;
    for run in buffer.layout_runs() {
        w = w.max(run.line_w);
        h = h.max(run.line_top + run.line_height);
    }
    (w.ceil(), if h > 0.0 { h } else { fallback_line_h })
}

pub fn draw_buffer(
    fonts: &mut FontSystem,
    atlas: &mut GlyphAtlas,
    scene: &mut Scene,
    buffer: &Buffer,
    origin: Point,
    color: Color,
    scale: f32,
) {
    let clip = scene.clip();
    for run in buffer.layout_runs() {
        let top = origin.y + run.line_top;
        if top > clip.bottom() || top + run.line_height < clip.y {
            continue;
        }
        for g in run.glyphs {
            let phys = g.physical((origin.x * scale, (origin.y + run.line_y) * scale), scale);
            let Some(e) = atlas.get(fonts, phys.cache_key) else { continue };
            let c = g
                .color_opt
                .map(|c| Color { r: c.r() as f32 / 255.0, g: c.g() as f32 / 255.0, b: c.b() as f32 / 255.0, a: c.a() as f32 / 255.0 })
                .unwrap_or(color);
            let b = Bounds::new(
                (phys.x + e.left) as f32 / scale,
                (phys.y - e.top) as f32 / scale,
                e.w as f32 / scale,
                e.h as f32 / scale,
            );
            scene.glyph(b, e.uv, c, e.color);
        }
    }
}
