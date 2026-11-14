use super::geom::{Bounds, Color, Point};

const KIND_RECT: f32 = 0.0;
const KIND_GLYPH: f32 = 1.0;
const KIND_COLOR_GLYPH: f32 = 2.0;
const KIND_SHADOW: f32 = 3.0;
const KIND_SEGMENT: f32 = 4.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub border_color: [f32; 4],
    pub params: [f32; 4],
    pub data: [f32; 4],
    pub clip: [f32; 4],
}

/// A flat list of primitives in paint order, split into layers so that
/// deferred content (menus, popovers) always lands on top.
#[derive(Default)]
pub struct Scene {
    layers: Vec<Vec<Instance>>,
    layer: usize,
    clips: Vec<Bounds>,
}

impl Scene {
    pub fn clear(&mut self) {
        self.layers.iter_mut().for_each(Vec::clear);
        self.layer = 0;
        self.clips.clear();
    }

    pub fn set_layer(&mut self, layer: usize) {
        self.layer = layer;
        while self.layers.len() <= layer {
            self.layers.push(Vec::new());
        }
    }

    pub fn layer(&self) -> usize {
        self.layer
    }

    pub fn clip(&self) -> Bounds {
        self.clips.last().copied().unwrap_or(Bounds::EVERYTHING)
    }

    pub fn push_clip(&mut self, b: Bounds) {
        let c = self.clip().intersect(&b);
        self.clips.push(c);
    }

    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    pub fn take_clips(&mut self) -> Vec<Bounds> {
        std::mem::take(&mut self.clips)
    }

    pub fn restore_clips(&mut self, clips: Vec<Bounds>) {
        self.clips = clips;
    }

    fn push(&mut self, mut inst: Instance) {
        let c = self.clip();
        if c.w <= 0.0 || c.h <= 0.0 {
            return;
        }
        inst.clip = [c.x, c.y, c.right(), c.bottom()];
        self.set_layer(self.layer);
        self.layers[self.layer].push(inst);
    }

    pub fn rect(&mut self, b: Bounds, fill: Color, radius: f32) {
        self.rect_bordered(b, fill, radius, Color::TRANSPARENT, 0.0);
    }

    pub fn rect_bordered(&mut self, b: Bounds, fill: Color, radius: f32, border: Color, border_w: f32) {
        if fill.a <= 0.0 && (border.a <= 0.0 || border_w <= 0.0) {
            return;
        }
        self.push(Instance {
            rect: [b.x, b.y, b.w, b.h],
            color: fill.premultiplied(),
            border_color: border.premultiplied(),
            params: [radius, border_w, 0.0, KIND_RECT],
            ..Default::default()
        });
    }

    pub fn shadow(&mut self, b: Bounds, color: Color, radius: f32, blur: f32) {
        self.push(Instance {
            rect: [b.x, b.y, b.w, b.h],
            color: color.premultiplied(),
            params: [radius, 0.0, blur, KIND_SHADOW],
            ..Default::default()
        });
    }

    pub fn segment(&mut self, a: Point, b: Point, thickness: f32, color: Color) {
        self.push(Instance {
            color: color.premultiplied(),
            params: [0.0, 0.0, thickness, KIND_SEGMENT],
            data: [a.x, a.y, b.x, b.y],
            ..Default::default()
        });
    }

    pub fn glyph(&mut self, b: Bounds, uv: [f32; 4], color: Color, is_color: bool) {
        self.push(Instance {
            rect: [b.x, b.y, b.w, b.h],
            color: color.premultiplied(),
            params: [0.0, 0.0, 0.0, if is_color { KIND_COLOR_GLYPH } else { KIND_GLYPH }],
            data: uv,
            ..Default::default()
        });
    }

    pub fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.layers.iter().flatten()
    }
}
