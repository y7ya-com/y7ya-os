// One instanced pipeline draws everything: rounded rects (with border),
// soft shadows, line segments (icons) and glyphs from the atlas.
// Coordinates are logical pixels; `globals.scale` converts to physical for AA.

struct Globals {
    viewport: vec2<f32>,
    scale: f32,
    // Corner radius of the (transparent, undecorated) window; 0 = square.
    window_radius: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Inst {
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) border_color: vec4<f32>,
    // radius, border width, blur/thickness, kind
    @location(3) params: vec4<f32>,
    // atlas uv rect, or segment endpoints
    @location(4) data: vec4<f32>,
    @location(5) clip: vec4<f32>,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) p: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) rect: vec4<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) border_color: vec4<f32>,
    @location(5) @interpolate(flat) params: vec4<f32>,
    @location(6) @interpolate(flat) data: vec4<f32>,
    @location(7) @interpolate(flat) clip: vec4<f32>,
};

const KIND_RECT: u32 = 0u;
const KIND_GLYPH: u32 = 1u;
const KIND_COLOR_GLYPH: u32 = 2u;
const KIND_SHADOW: u32 = 3u;
const KIND_SEGMENT: u32 = 4u;

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: Inst) -> VOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let kind = u32(inst.params.w + 0.5);
    var pad = 1.0;
    if kind == KIND_SHADOW {
        pad = inst.params.z * 2.0;
    }
    if kind == KIND_GLYPH || kind == KIND_COLOR_GLYPH {
        pad = 0.0;
    }
    var origin = inst.rect.xy - vec2<f32>(pad);
    var size = inst.rect.zw + vec2<f32>(2.0 * pad);
    if kind == KIND_SEGMENT {
        let t = inst.params.z;
        let lo = min(inst.data.xy, inst.data.zw) - vec2<f32>(t + 1.0);
        let hi = max(inst.data.xy, inst.data.zw) + vec2<f32>(t + 1.0);
        origin = lo;
        size = hi - lo;
    }
    let p = origin + corner * size;

    var out: VOut;
    out.pos = vec4<f32>(p.x / globals.viewport.x * 2.0 - 1.0, 1.0 - p.y / globals.viewport.y * 2.0, 0.0, 1.0);
    out.p = p;
    out.uv = mix(inst.data.xy, inst.data.zw, corner);
    out.rect = inst.rect;
    out.color = inst.color;
    out.border_color = inst.border_color;
    out.params = inst.params;
    out.data = inst.data;
    out.clip = inst.clip;
    return out;
}

fn sd_round_rect(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    if in.p.x < in.clip.x || in.p.y < in.clip.y || in.p.x >= in.clip.z || in.p.y >= in.clip.w {
        discard;
    }
    var mask = 1.0;
    if globals.window_radius > 0.0 {
        let half = globals.viewport * 0.5;
        let d = sd_round_rect(in.p - half, half, globals.window_radius);
        mask = clamp(0.5 - d * globals.scale, 0.0, 1.0);
    }
    return shade(in) * mask;
}

fn shade(in: VOut) -> vec4<f32> {
    let kind = u32(in.params.w + 0.5);
    let s = globals.scale;
    let tex = textureSampleLevel(atlas, samp, in.uv, 0.0);

    if kind == KIND_GLYPH {
        return in.color * tex.a;
    }
    if kind == KIND_COLOR_GLYPH {
        return tex * in.color.a;
    }
    if kind == KIND_SEGMENT {
        let d = sd_segment(in.p, in.data.xy, in.data.zw) - in.params.z * 0.5;
        return in.color * clamp(0.5 - d * s, 0.0, 1.0);
    }

    let half = in.rect.zw * 0.5;
    let local = in.p - (in.rect.xy + half);
    let radius = min(in.params.x, min(half.x, half.y));
    let d = sd_round_rect(local, half, radius);

    if kind == KIND_SHADOW {
        let blur = max(in.params.z, 0.001);
        return in.color * (1.0 - smoothstep(-blur, blur, d));
    }

    let outer = clamp(0.5 - d * s, 0.0, 1.0);
    var col = in.color;
    let bw = in.params.y;
    if bw > 0.0 {
        let inner = clamp(0.5 - (d + bw) * s, 0.0, 1.0);
        col = mix(in.border_color, in.color, inner);
    }
    return col * outer;
}
