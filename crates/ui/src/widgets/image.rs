//! Raster images (thumbnails and the like), packed into the glyph atlas.

use std::sync::Arc;

use crate::cx::Cx;
use crate::element::{Canvas, canvas};
use crate::geom::Color;

/// Straight-alpha RGBA pixels. `key` identifies the pixels: two images with the
/// same key are assumed identical, so derive it from the content or its source.
#[derive(Clone)]
pub struct Image {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

impl Image {
    /// Box-filters the image down so neither side exceeds `max` pixels.
    pub fn downscaled(&self, max: u32) -> Image {
        let (w, h) = (self.width.max(1), self.height.max(1));
        let f = (w.max(h) as f32 / max as f32).max(1.0);
        if f <= 1.0 {
            return self.clone();
        }
        let (nw, nh) = (((w as f32 / f).round() as u32).max(1), ((h as f32 / f).round() as u32).max(1));
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            let (sy0, sy1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
            for x in 0..nw {
                let (sx0, sx1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
                // Average in premultiplied space so transparent pixels don't darken edges.
                let mut acc = [0u64; 4];
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        let i = ((sy * w + sx) * 4) as usize;
                        let a = self.rgba[i + 3] as u64;
                        acc[0] += self.rgba[i] as u64 * a;
                        acc[1] += self.rgba[i + 1] as u64 * a;
                        acc[2] += self.rgba[i + 2] as u64 * a;
                        acc[3] += a;
                    }
                }
                let n = ((sy1 - sy0) * (sx1 - sx0)) as u64;
                let o = ((y * nw + x) * 4) as usize;
                if acc[3] > 0 {
                    out[o] = (acc[0] / acc[3]) as u8;
                    out[o + 1] = (acc[1] / acc[3]) as u8;
                    out[o + 2] = (acc[2] / acc[3]) as u8;
                }
                out[o + 3] = (acc[3] / n) as u8;
            }
        }
        Image { key: self.key ^ (max as u64).rotate_left(32), width: nw, height: nh, rgba: Arc::new(out) }
    }
}

/// An element that draws `img` stretched to its layout box. Keep images small
/// (thumbnails, at most 512×512); see [`downscaled`](Image::downscaled).
pub fn image(cx: &mut Cx, img: &Image) -> Canvas {
    let uv = cx.text.atlas.image(img.key, img.width, img.height, &img.rgba);
    canvas(move |scene, b| {
        if let Some(uv) = uv {
            scene.glyph(b, uv, Color::WHITE, true);
        }
    })
}
