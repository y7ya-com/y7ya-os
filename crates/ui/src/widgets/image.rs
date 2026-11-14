//! Raster images. An [`Image`] holds decoded pixels; the renderer uploads it
//! to its own GPU texture (with mipmaps, so it scales down smoothly) the first
//! time it's drawn and drops the texture once the image is gone or unused.
//!
//! Decoding is slow: do it off the UI thread (`Image::load` is `Send`-safe)
//! and hand the result back through a channel and [`Waker`](crate::Waker).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use crate::element::{Canvas, canvas};
use crate::geom::{Bounds, Color};
use crate::scene::{Instance, KIND_IMAGE, Scene};

/// A decoded image, cheap to clone.
#[derive(Clone)]
pub struct Image(Arc<ImageData>);

struct ImageData {
    id: u64,
    width: u32,
    height: u32,
    /// Premultiplied RGBA mip levels, largest first.
    levels: Vec<Vec<u8>>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.0.width, self.0.height)
    }
}

impl Image {
    /// From straight-alpha RGBA8 pixels, row by row.
    pub fn from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Image {
        assert_eq!(rgba.len(), (width * height * 4) as usize, "pixel buffer size");
        for p in rgba.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a < 255 {
                for c in &mut p[..3] {
                    *c = (*c as u32 * a / 255) as u8;
                }
            }
        }
        Self::premultiplied(width, height, rgba)
    }

    fn premultiplied(width: u32, height: u32, rgba: Vec<u8>) -> Image {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let levels = mip_chain(width.max(1), height.max(1), rgba);
        Image(Arc::new(ImageData { id: NEXT.fetch_add(1, Ordering::Relaxed), width, height, levels }))
    }

    /// Decodes an image file (PNG, JPEG, GIF, WebP, BMP), applying EXIF
    /// rotation and shrinking it to fit within `max_side` pixels.
    pub fn load(path: &Path, max_side: u32) -> Option<Image> {
        use image::ImageDecoder;
        let reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
        let mut decoder = reader.into_decoder().ok()?;
        let orientation = decoder.orientation().ok();
        let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
        if let Some(o) = orientation {
            img.apply_orientation(o);
        }
        Some(Self::from_dynamic(img, max_side))
    }

    /// Decodes an in-memory image file, like [`Image::load`].
    pub fn decode(bytes: &[u8], max_side: u32) -> Option<Image> {
        let img = image::load_from_memory(bytes).ok()?;
        Some(Self::from_dynamic(img, max_side))
    }

    fn from_dynamic(img: image::DynamicImage, max_side: u32) -> Image {
        let img = if img.width() > max_side || img.height() > max_side { img.thumbnail(max_side, max_side) } else { img };
        let rgba = img.into_rgba8();
        let (w, h) = rgba.dimensions();
        Image::from_rgba(w, h, rgba.into_raw())
    }

    pub fn width(&self) -> u32 {
        self.0.width
    }

    pub fn height(&self) -> u32 {
        self.0.height
    }

    /// The same pixels turned a quarter turn clockwise `turns` times.
    pub fn rotated(&self, turns: u32) -> Image {
        let (w, h) = (self.0.width as usize, self.0.height as usize);
        let src = &self.0.levels[0];
        let turns = turns % 4;
        if turns == 0 {
            return self.clone();
        }
        let (nw, nh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
        let mut out = vec![0u8; src.len()];
        for y in 0..h {
            for x in 0..w {
                let (nx, ny) = match turns {
                    1 => (h - 1 - y, x),
                    2 => (w - 1 - x, h - 1 - y),
                    _ => (y, w - 1 - x),
                };
                let (s, d) = ((y * w + x) * 4, (ny * nw + nx) * 4);
                out[d..d + 4].copy_from_slice(&src[s..s + 4]);
            }
        }
        Self::premultiplied(nw as u32, nh as u32, out)
    }
}

/// Box-filtered mip levels down to 1×1.
fn mip_chain(mut w: u32, mut h: u32, base: Vec<u8>) -> Vec<Vec<u8>> {
    let mut levels = vec![base];
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let src = levels.last().unwrap();
        let mut dst = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            let (y0, y1) = ((y * 2).min(h - 1), (y * 2 + 1).min(h - 1));
            for x in 0..nw {
                let (x0, x1) = ((x * 2).min(w - 1), (x * 2 + 1).min(w - 1));
                for c in 0..4 {
                    let at = |xx: u32, yy: u32| src[((yy * w + xx) * 4 + c) as usize] as u32;
                    let sum = at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1);
                    dst[((y * nw + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
                }
            }
        }
        levels.push(dst);
        (w, h) = (nw, nh);
    }
    levels
}

/// How an image fills the box it's drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Scaled to fit inside, centered, never enlarged past its own size.
    Contain,
    /// Scaled to fit inside, centered, enlarged if smaller.
    Scale,
    /// Scaled and cropped to cover the whole box.
    Cover,
}

/// Where an image of `size` lands inside `b`, and the part of it that shows
/// (as a uv rect).
pub fn fit_rect(b: Bounds, size: (f32, f32), fit: Fit) -> (Bounds, [f32; 4]) {
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    let full = [0.0, 0.0, 1.0, 1.0];
    match fit {
        Fit::Contain | Fit::Scale => {
            let mut s = (b.w / w).min(b.h / h);
            if fit == Fit::Contain {
                s = s.min(1.0);
            }
            let (dw, dh) = (w * s, h * s);
            (Bounds::new(b.x + (b.w - dw) / 2.0, b.y + (b.h - dh) / 2.0, dw, dh), full)
        }
        Fit::Cover => {
            let s = (b.w / w).max(b.h / h);
            let (cw, ch) = (b.w / s / w, b.h / s / h);
            let (u, v) = ((1.0 - cw) / 2.0, (1.0 - ch) / 2.0);
            (b, [u, v, u + cw, v + ch])
        }
    }
}

/// Drawing options for [`Scene::image_with`].
#[derive(Clone, Copy, Debug)]
pub struct ImageStyle {
    pub radius: f32,
    pub opacity: f32,
    /// Quarter turns clockwise.
    pub turns: u32,
    /// The part of the image to draw, in uv space.
    pub uv: [f32; 4],
}

impl Default for ImageStyle {
    fn default() -> Self {
        Self { radius: 0.0, opacity: 1.0, turns: 0, uv: [0.0, 0.0, 1.0, 1.0] }
    }
}

impl Scene {
    /// Draws `img` stretched over `b`, with rounded corners.
    pub fn image(&mut self, b: Bounds, img: &Image, radius: f32) {
        self.image_with(b, img, ImageStyle { radius, ..Default::default() });
    }

    pub fn image_with(&mut self, b: Bounds, img: &Image, style: ImageStyle) {
        if b.w <= 0.0 || b.h <= 0.0 || style.opacity <= 0.0 {
            return;
        }
        let slot = match self.images.iter().rposition(|i| i.0.id == img.0.id) {
            Some(s) => s,
            None => {
                self.images.push(img.clone());
                self.images.len() - 1
            }
        };
        // Rotation maps the quad's corners onto the texture differently; odd
        // turns also swap its axes (the shader transposes them).
        let [u0, v0, u1, v1] = style.uv;
        let turns = style.turns % 4;
        let data = match turns {
            0 => [u0, v0, u1, v1],
            1 => [u0, v1, u1, v0],
            2 => [u1, v1, u0, v0],
            _ => [u1, v0, u0, v1],
        };
        self.push(Instance {
            rect: [b.x, b.y, b.w, b.h],
            color: Color::WHITE.alpha(style.opacity).premultiplied(),
            params: [style.radius, turns as f32, slot as f32, KIND_IMAGE],
            data,
            ..Default::default()
        });
    }
}

/// An element that paints `img` fitted into its box.
pub fn image(img: &Image, fit: Fit, radius: f32) -> Canvas {
    let img = img.clone();
    canvas(move |scene, b| {
        let (dst, uv) = fit_rect(b, (img.width() as f32, img.height() as f32), fit);
        scene.image_with(dst, &img, ImageStyle { radius, uv, ..Default::default() });
    })
}

// ---------------------------------------------------------------------------
// GPU side

pub(crate) fn image_slot(inst: &Instance) -> Option<usize> {
    (inst.params[3] == KIND_IMAGE).then(|| inst.params[2] as usize)
}

struct GpuImage {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    image: Weak<ImageData>,
    used: u64,
}

/// Textures for the images in recent frames, keyed by image id.
pub(crate) struct ImageCache {
    textures: HashMap<u64, GpuImage>,
    sampler: wgpu::Sampler,
    frame: u64,
}

/// Frames an image may go undrawn before its texture is dropped.
const KEEP_FRAMES: u64 = 240;

impl ImageCache {
    pub fn new(device: &wgpu::Device) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self { textures: HashMap::new(), sampler, frame: 0 }
    }

    /// Uploads images new to this frame and drops stale textures.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        globals: &wgpu::Buffer,
        images: &[Image],
    ) {
        self.frame += 1;
        let max = device.limits().max_texture_dimension_2d;
        for img in images {
            let d = &img.0;
            if let Some(t) = self.textures.get_mut(&d.id) {
                t.used = self.frame;
                continue;
            }
            if d.width == 0 || d.height == 0 || d.width > max || d.height > max {
                continue;
            }
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("image"),
                size: wgpu::Extent3d { width: d.width, height: d.height, depth_or_array_layers: 1 },
                mip_level_count: d.levels.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let (mut w, mut h) = (d.width, d.height);
            for (level, pixels) in d.levels.iter().enumerate() {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    pixels,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
                (w, h) = ((w / 2).max(1), (h / 2).max(1));
            }
            let view = texture.create_view(&Default::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("image"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            let image = Arc::downgrade(&img.0);
            self.textures.insert(d.id, GpuImage { _texture: texture, bind_group, image, used: self.frame });
        }
        let frame = self.frame;
        self.textures.retain(|_, t| t.image.strong_count() > 0 && frame - t.used < KEEP_FRAMES);
    }

    pub fn bind_group(&self, img: &Image) -> Option<&wgpu::BindGroup> {
        self.textures.get(&img.0.id).map(|t| &t.bind_group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mips_and_rotation() {
        let mut px = Vec::new();
        for i in 0..6u8 {
            px.extend_from_slice(&[i * 40, 0, 0, 255]);
        }
        let img = Image::from_rgba(3, 2, px);
        assert_eq!(img.0.levels.len(), 2);
        assert_eq!(img.0.levels[1].len(), 4);
        let r = img.rotated(1);
        assert_eq!((r.width(), r.height()), (2, 3));
        // Clockwise: the bottom-left pixel (index 3) lands top-left.
        assert_eq!(r.0.levels[0][0], 120);
        let fit = fit_rect(Bounds::new(0.0, 0.0, 100.0, 50.0), (200.0, 200.0), Fit::Contain);
        assert_eq!(fit.0, Bounds::new(25.0, 0.0, 50.0, 50.0));
    }
}
