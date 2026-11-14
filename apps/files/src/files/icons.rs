//! Icons. Line icons in the SF Symbols spirit for the sidebar and toolbar,
//! drawn on an 18-unit grid scaled to their box; and the big, filled Finder
//! icons: the blue folder (with an embossed symbol on special folders) and
//! the white page with a folded corner for documents.

use media::Kind;
use ui::{Bounds, Color, Scene, pt, rgb, rgba};

const STROKE: f32 = 1.5;

/// A line icon: draws into the square centered in its bounds.
pub type Icon = fn(&mut Scene, Bounds, Color);

fn frame(b: Bounds) -> (f32, f32, f32, f32) {
    let size = b.w.min(b.h);
    let c = b.center();
    let s = size / 18.0;
    (c.x - size / 2.0, c.y - size / 2.0, s, STROKE * s.max(1.0))
}

fn outline(scene: &mut Scene, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color, stroke: f32) {
    scene.rect_bordered(Bounds::new(x, y, w, h), Color::TRANSPARENT, r, color, stroke);
}

fn dot(scene: &mut Scene, cx: f32, cy: f32, r: f32, color: Color) {
    scene.rect(Bounds::new(cx - r, cy - r, 2.0 * r, 2.0 * r), color, r);
}

fn poly(scene: &mut Scene, pts: &[(f32, f32)], x: f32, y: f32, s: f32, w: f32, color: Color) {
    for p in pts.windows(2) {
        scene.segment(pt(x + p[0].0 * s, y + p[0].1 * s), pt(x + p[1].0 * s, y + p[1].1 * s), w, color);
    }
}

fn arc(scene: &mut Scene, cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32, color: Color, stroke: f32) {
    let steps = ((to_deg - from_deg).abs() / 15.0).ceil().max(2.0) as usize;
    let mut prev = None;
    for i in 0..=steps {
        let a = (from_deg + (to_deg - from_deg) * i as f32 / steps as f32).to_radians();
        let p = pt(cx + r * a.cos(), cy + r * a.sin());
        if let Some(q) = prev {
            scene.segment(q, p, stroke, color);
        }
        prev = Some(p);
    }
}

/// Fills a triangle with scanlines (only for opaque colors: lines overlap).
pub fn fill_triangle(scene: &mut Scene, a: (f32, f32), b: (f32, f32), c: (f32, f32), color: Color) {
    let (y0, y1) = (a.1.min(b.1).min(c.1), a.1.max(b.1).max(c.1));
    let step = 0.5;
    let t = 0.7;
    let edges = [(a, b), (b, c), (c, a)];
    let mut y = y0 + step / 2.0;
    while y < y1 {
        let mut xs = edges.iter().filter_map(|&(p, q)| {
            let (lo, hi) = if p.1 < q.1 { (p, q) } else { (q, p) };
            (y >= lo.1 && y <= hi.1 && hi.1 > lo.1).then(|| lo.0 + (y - lo.1) / (hi.1 - lo.1) * (hi.0 - lo.0))
        });
        if let Some(first) = xs.next() {
            let (mut l, mut r) = (first, first);
            for x in xs {
                l = l.min(x);
                r = r.max(x);
            }
            if r - l > t {
                scene.segment(pt(l + t / 2.0, y), pt(r - t / 2.0, y), t, color);
            }
        }
        y += step;
    }
}

// -- line icons ---------------------------------------------------------------

pub fn clock(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    let r = 7.2 * s;
    outline(scene, x + 9.0 * s - r, y + 9.0 * s - r, 2.0 * r, 2.0 * r, r, color, w);
    poly(scene, &[(9.0, 4.8), (9.0, 9.2), (12.0, 11.0)], x, y, s, w, color);
}

pub fn house(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(1.8, 9.2), (9.0, 2.6), (16.2, 9.2)], x, y, s, w, color);
    poly(scene, &[(4.0, 7.4), (4.0, 15.6), (14.0, 15.6), (14.0, 7.4)], x, y, s, w, color);
    poly(scene, &[(7.4, 15.6), (7.4, 11.2), (10.6, 11.2), (10.6, 15.6)], x, y, s, w, color);
}

pub fn desktop(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    outline(scene, x + 1.5 * s, y + 3.0 * s, 15.0 * s, 10.2 * s, 2.0 * s, color, w);
    poly(scene, &[(9.0, 13.2), (9.0, 15.6)], x, y, s, w, color);
    poly(scene, &[(5.8, 15.8), (12.2, 15.8)], x, y, s, w, color);
}

pub fn document(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(10.6, 1.8), (4.2, 1.8), (4.2, 16.2), (13.8, 16.2), (13.8, 5.0), (10.6, 1.8), (10.6, 5.0), (13.8, 5.0)], x, y, s, w, color);
    for yy in [8.5, 11.0, 13.5] {
        poly(scene, &[(6.6, yy), (11.4, yy)], x, y, s, w * 0.8, color);
    }
}

pub fn download(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    let r = 7.2 * s;
    outline(scene, x + 9.0 * s - r, y + 9.0 * s - r, 2.0 * r, 2.0 * r, r, color, w);
    poly(scene, &[(9.0, 4.8), (9.0, 12.6)], x, y, s, w, color);
    poly(scene, &[(5.9, 9.6), (9.0, 12.8), (12.1, 9.6)], x, y, s, w, color);
}

pub fn photo(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    outline(scene, x + 1.5 * s, y + 3.2 * s, 15.0 * s, 11.6 * s, 2.2 * s, color, w);
    poly(scene, &[(3.6, 12.6), (7.4, 8.2), (10.4, 11.4), (12.2, 9.4), (14.6, 12.2)], x, y, s, w, color);
    dot(scene, x + 12.4 * s, y + 6.6 * s, 1.3 * s, color);
}

pub fn music(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    dot(scene, x + 6.4 * s, y + 13.6 * s, 2.4 * s, color);
    dot(scene, x + 13.4 * s, y + 11.8 * s, 2.4 * s, color);
    poly(scene, &[(8.6, 13.4), (8.6, 4.0), (15.6, 2.4), (15.6, 11.6)], x, y, s, w, color);
}

pub fn film(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    outline(scene, x + 2.0 * s, y + 2.4 * s, 14.0 * s, 13.2 * s, 2.0 * s, color, w);
    poly(scene, &[(5.6, 2.6), (5.6, 15.4)], x, y, s, w, color);
    poly(scene, &[(12.4, 2.6), (12.4, 15.4)], x, y, s, w, color);
    for yy in [6.4, 9.0, 11.6] {
        poly(scene, &[(2.4, yy), (5.4, yy)], x, y, s, w * 0.8, color);
        poly(scene, &[(12.6, yy), (15.6, yy)], x, y, s, w * 0.8, color);
    }
}

pub fn drive(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    outline(scene, x + 1.5 * s, y + 5.0 * s, 15.0 * s, 8.4 * s, 2.4 * s, color, w);
    dot(scene, x + 13.0 * s, y + 9.2 * s, 1.0 * s, color);
}

pub fn external_drive(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    outline(scene, x + 1.5 * s, y + 5.0 * s, 15.0 * s, 8.4 * s, 2.4 * s, color, w);
    poly(scene, &[(4.6, 9.2), (9.0, 9.2)], x, y, s, w, color);
    dot(scene, x + 13.0 * s, y + 9.2 * s, 1.0 * s, color);
}

pub fn network(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    let r = 7.2 * s;
    outline(scene, x + 9.0 * s - r, y + 9.0 * s - r, 2.0 * r, 2.0 * r, r, color, w);
    poly(scene, &[(1.8, 9.0), (16.2, 9.0)], x, y, s, w * 0.8, color);
    outline(scene, x + 6.0 * s, y + 1.8 * s, 6.0 * s, 14.4 * s, 3.0 * s, color, w * 0.8);
}


pub fn trash(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(2.5, 4.5), (15.5, 4.5)], x, y, s, w, color);
    poly(scene, &[(6.8, 4.5), (7.2, 2.2), (10.8, 2.2), (11.2, 4.5)], x, y, s, w, color);
    poly(scene, &[(4.2, 4.8), (5.0, 15.6), (13.0, 15.6), (13.8, 4.8)], x, y, s, w, color);
    poly(scene, &[(7.4, 7.2), (7.6, 13.2)], x, y, s, w * 0.8, color);
    poly(scene, &[(10.6, 7.2), (10.4, 13.2)], x, y, s, w * 0.8, color);
}

pub fn chevron_left(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(11.2, 3.6), (5.8, 9.0), (11.2, 14.4)], x, y, s, w * 1.1, color);
}

pub fn chevron_right(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(6.8, 3.6), (12.2, 9.0), (6.8, 14.4)], x, y, s, w * 1.1, color);
}

pub fn chevron_down(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(4.5, 6.8), (9.0, 11.3), (13.5, 6.8)], x, y, s, w, color);
}

pub fn chevron_up(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(4.5, 11.3), (9.0, 6.8), (13.5, 11.3)], x, y, s, w, color);
}

pub fn grid(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    for (gx, gy) in [(2.4, 2.4), (10.0, 2.4), (2.4, 10.0), (10.0, 10.0)] {
        outline(scene, x + gx * s, y + gy * s, 5.6 * s, 5.6 * s, 1.6 * s, color, w * 0.9);
    }
}

pub fn list(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    for yy in [4.4, 9.0, 13.6] {
        dot(scene, x + 3.2 * s, y + yy * s, 1.1 * s, color);
        poly(scene, &[(6.8, yy), (15.8, yy)], x, y, s, w, color);
    }
}

pub fn columns(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    outline(scene, x + 1.5 * s, y + 3.2 * s, 15.0 * s, 11.6 * s, 2.2 * s, color, w * 0.9);
    poly(scene, &[(6.5, 3.4), (6.5, 14.6)], x, y, s, w * 0.9, color);
    poly(scene, &[(11.5, 3.4), (11.5, 14.6)], x, y, s, w * 0.9, color);
}

pub fn ellipsis_circle(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    let r = 7.4 * s;
    outline(scene, x + 9.0 * s - r, y + 9.0 * s - r, 2.0 * r, 2.0 * r, r, color, w);
    for xx in [5.6, 9.0, 12.4] {
        dot(scene, x + xx * s, y + 9.0 * s, 1.05 * s, color);
    }
}

pub fn magnifier(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    let r = 5.6 * s;
    outline(scene, x + 1.8 * s, y + 1.8 * s, 2.0 * r, 2.0 * r, r, color, w);
    poly(scene, &[(11.4, 11.4), (16.0, 16.0)], x, y, s, w * 1.2, color);
}

pub fn xmark(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(4.5, 4.5), (13.5, 13.5)], x, y, s, w, color);
    poly(scene, &[(13.5, 4.5), (4.5, 13.5)], x, y, s, w, color);
}

pub fn check(scene: &mut Scene, b: Bounds, color: Color) {
    let (x, y, s, w) = frame(b);
    poly(scene, &[(3.5, 9.5), (7.5, 13.5), (14.5, 4.5)], x, y, s, w * 1.1, color);
}



/// The disclosure triangle in column view rows.
pub fn chevron_small(scene: &mut Scene, b: Bounds, color: Color) {
    let c = b.center();
    let h = b.h.min(b.w) * 0.22;
    scene.segment(pt(c.x - h * 0.5, c.y - h), pt(c.x + h * 0.5, c.y), 1.3, color);
    scene.segment(pt(c.x + h * 0.5, c.y), pt(c.x - h * 0.5, c.y + h), 1.3, color);
}

// -- Finder icons ---------------------------------------------------------------

/// Fills a rounded rect with a vertical gradient, in clipped bands.
fn gradient(scene: &mut Scene, b: Bounds, radius: f32, top: Color, bottom: Color) {
    const BANDS: usize = 10;
    let mix = |t: f32| Color {
        r: top.r + (bottom.r - top.r) * t,
        g: top.g + (bottom.g - top.g) * t,
        b: top.b + (bottom.b - top.b) * t,
        a: top.a + (bottom.a - top.a) * t,
    };
    for i in 0..BANDS {
        let y0 = b.y + b.h * i as f32 / BANDS as f32;
        let y1 = if i == BANDS - 1 { b.bottom() + 1.0 } else { b.y + b.h * (i + 1) as f32 / BANDS as f32 };
        let y0 = if i == 0 { b.y - 1.0 } else { y0 };
        scene.push_clip(Bounds::new(b.x - 1.0, y0, b.w + 2.0, y1 - y0));
        scene.rect(b, mix((i as f32 + 0.5) / BANDS as f32), radius);
        scene.pop_clip();
    }
}

/// The macOS folder: a darker back with a tab, a lighter front, and for
/// special folders their symbol embossed on the front.
pub fn folder(scene: &mut Scene, b: Bounds, emblem: Option<Icon>) {
    let size = b.w.min(b.h);
    let c = b.center();
    let (w, h) = (size * 0.88, size * 0.70);
    let (x, y) = (c.x - w / 2.0, c.y - h / 2.0 + size * 0.02);
    let r = (size * 0.055).max(1.2);
    let tab_h = size * 0.08;
    let tab_w = w * 0.40;
    let back = rgb(0x4ba3ea);
    // Back panel and tab, the tab's right edge sloping down.
    scene.rect(Bounds::new(x, y, tab_w, tab_h * 2.0), back, r * 0.8);
    fill_triangle(scene, (x + tab_w - r * 0.6, y), (x + tab_w + tab_h * 1.1, y + tab_h + 0.5), (x + tab_w - r * 0.6, y + tab_h + 0.5), back);
    scene.rect(Bounds::new(x, y + tab_h, w, h - tab_h), back, r);
    // Front panel, lit from above.
    let front = Bounds::new(x, y + tab_h + size * 0.075, w, h - tab_h - size * 0.075);
    gradient(scene, front, r, rgb(0x86cdfb), rgb(0x5db4f4));
    let hl = (size / 64.0).max(0.6);
    scene.segment(pt(front.x + r, front.y + hl * 0.6), pt(front.right() - r, front.y + hl * 0.6), hl, rgba(0xffffff, 0.45));
    if let Some(icon) = emblem {
        let g = size * 0.34;
        let gc = pt(front.center().x, front.center().y + size * 0.01);
        icon(scene, Bounds::new(gc.x - g / 2.0, gc.y - g / 2.0, g, g), rgba(0x1f78c8, 0.75));
    }
}

/// Where a document page sits in an icon box of `b`.
pub fn page_rect(b: Bounds) -> Bounds {
    let size = b.w.min(b.h);
    let c = b.center();
    let (w, h) = (size * 0.64, size * 0.82);
    Bounds::new(c.x - w / 2.0, c.y - h / 2.0, w, h)
}

/// A white page with a folded top-right corner, marked by kind.
pub fn page(scene: &mut Scene, b: Bounds, kind: Kind) {
    let size = b.w.min(b.h);
    let p = page_rect(b);
    let (x, y, w, h) = (p.x, p.y, p.w, p.h);
    let fold = w * 0.26;
    let r = (size * 0.035).max(0.8);
    let line = (size / 80.0).max(0.6);
    let edge = rgba(0x000000, 0.2);
    let paper = rgb(0xffffff);

    scene.shadow(Bounds::new(x, y + size * 0.012, w, h), rgba(0x000000, 0.12), r, size * 0.02);
    scene.rect(Bounds::new(x, y, w - fold, h), paper, r);
    scene.rect(Bounds::new(x + w - fold - r, y + fold, fold + r, h - fold), paper, r);
    // The folded flap.
    fill_triangle(scene, (x + w - fold, y), (x + w - fold, y + fold), (x + w, y + fold), rgb(0xe4e4e6));

    // Outline, corner by corner.
    scene.segment(pt(x, y + r), pt(x, y + h - r), line, edge);
    scene.segment(pt(x + r, y + h), pt(x + w - r, y + h), line, edge);
    scene.segment(pt(x + w, y + fold), pt(x + w, y + h - r), line, edge);
    scene.segment(pt(x + r, y), pt(x + w - fold, y), line, edge);
    scene.segment(pt(x + w - fold, y), pt(x + w, y + fold), line, edge);
    scene.segment(pt(x + w - fold, y), pt(x + w - fold, y + fold), line * 0.8, edge);
    scene.segment(pt(x + w - fold, y + fold), pt(x + w, y + fold), line * 0.8, edge);
    arc(scene, x + r, y + r, r, 180.0, 270.0, edge, line);
    arc(scene, x + r, y + h - r, r, 90.0, 180.0, edge, line);
    arc(scene, x + w - r, y + h - r, r, 0.0, 90.0, edge, line);

    if size < 24.0 {
        return;
    }
    // A hint of the content, in the upper part (the extension label goes below).
    let inner = Bounds::new(x + w * 0.16, y + fold + h * 0.06, w * 0.68, h * 0.42);
    let thick = (size / 40.0).max(0.8);
    match kind {
        Kind::Text | Kind::Document | Kind::Other | Kind::Font | Kind::Pdf => {
            let lens = [1.0, 0.86, 0.94, 0.7, 0.9, 0.6];
            for (i, l) in lens.iter().enumerate() {
                let yy = inner.y + i as f32 * inner.h / 6.0;
                scene.segment(pt(inner.x, yy), pt(inner.x + inner.w * l, yy), thick, rgba(0x000000, 0.14));
            }
        }
        Kind::Code => {
            let rows: [(f32, f32, u32); 6] = [(0.0, 0.5, 0xaf52de), (0.12, 0.8, 0x007aff), (0.24, 0.55, 0x8e8e93), (0.24, 0.7, 0x007aff), (0.12, 0.4, 0xff9500), (0.0, 0.2, 0xaf52de)];
            for (i, (indent, l, col)) in rows.iter().enumerate() {
                let yy = inner.y + i as f32 * inner.h / 6.0;
                let x0 = inner.x + inner.w * indent;
                scene.segment(pt(x0, yy), pt(x0 + inner.w * l, yy), thick, rgba(*col, 0.55));
            }
        }
        Kind::Image | Kind::OtherImage => {
            let a = Bounds::new(inner.x, inner.y, inner.w, inner.h * 0.95);
            scene.rect(a, rgb(0x8fd0ff), size * 0.03);
            scene.push_clip(a);
            fill_triangle(scene, (a.x - a.w * 0.1, a.bottom()), (a.x + a.w * 0.35, a.y + a.h * 0.35), (a.x + a.w * 0.8, a.bottom()), rgb(0x34a853));
            fill_triangle(scene, (a.x + a.w * 0.4, a.bottom()), (a.x + a.w * 0.72, a.y + a.h * 0.5), (a.right() + a.w * 0.1, a.bottom()), rgb(0x2a8c46));
            scene.pop_clip();
            dot(scene, a.x + a.w * 0.76, a.y + a.h * 0.26, a.h * 0.12, rgb(0xffd60a));
        }
        Kind::Audio => {
            let g = inner.h * 1.1;
            music(scene, Bounds::new(inner.center().x - g / 2.0, inner.y - g * 0.05, g, g), rgb(0xfc3c44));
        }
        Kind::Video => {
            let a = Bounds::new(inner.x, inner.y, inner.w, inner.h * 0.95);
            scene.rect(a, rgb(0x2c2c2e), size * 0.03);
            let c = a.center();
            let t = a.h * 0.26;
            fill_triangle(scene, (c.x - t * 0.7, c.y - t), (c.x - t * 0.7, c.y + t), (c.x + t, c.y), rgb(0xffffff));
        }
        Kind::Archive => {
            let cx = x + w / 2.0;
            let zw = w * 0.07;
            let mut yy = y + size * 0.01;
            let mut left = true;
            while yy < y + h * 0.55 {
                let xx = if left { cx - zw } else { cx };
                scene.rect(Bounds::new(xx, yy, zw, zw * 0.8), rgb(0x8e8e93), zw * 0.2);
                yy += zw * 0.9;
                left = !left;
            }
            scene.rect(Bounds::new(cx - zw * 1.2, yy, zw * 2.4, zw * 3.0), rgb(0x8e8e93), zw * 0.4);
        }
        Kind::Spreadsheet => {
            let a = inner;
            scene.rect_bordered(a, rgba(0x34c759, 0.10), 0.0, rgba(0x34c759, 0.7), thick * 0.8);
            for i in 1..4 {
                let yy = a.y + a.h * i as f32 / 4.0;
                scene.segment(pt(a.x, yy), pt(a.right(), yy), thick * 0.7, rgba(0x34c759, 0.7));
            }
            let xx = a.x + a.w * 0.35;
            scene.segment(pt(xx, a.y), pt(xx, a.bottom()), thick * 0.7, rgba(0x34c759, 0.7));
        }
        Kind::Presentation => {
            let a = Bounds::new(inner.x, inner.y + inner.h * 0.1, inner.w, inner.h * 0.7);
            scene.rect(a, rgb(0xff9500), size * 0.02);
            scene.rect(Bounds::new(a.x + a.w * 0.15, a.y + a.h * 0.25, a.w * 0.5, a.h * 0.14), rgba(0xffffff, 0.9), 0.0);
            scene.rect(Bounds::new(a.x + a.w * 0.15, a.y + a.h * 0.52, a.w * 0.7, a.h * 0.1), rgba(0xffffff, 0.7), 0.0);
        }
        Kind::Executable => {
            let a = Bounds::new(inner.x, inner.y, inner.w, inner.h * 0.95);
            scene.rect(a, rgb(0x2c2c2e), size * 0.03);
            let s = a.h / 10.0;
            scene.segment(pt(a.x + 2.0 * s, a.y + 3.0 * s), pt(a.x + 4.0 * s, a.y + 5.0 * s), thick, rgb(0x30d158));
            scene.segment(pt(a.x + 4.0 * s, a.y + 5.0 * s), pt(a.x + 2.0 * s, a.y + 7.0 * s), thick, rgb(0x30d158));
            scene.segment(pt(a.x + 5.0 * s, a.y + 7.0 * s), pt(a.x + 8.0 * s, a.y + 7.0 * s), thick, rgb(0x30d158));
        }
        Kind::DiskImage => {
            let g = inner.h * 1.1;
            drive(scene, Bounds::new(inner.center().x - g / 2.0, inner.y - g * 0.05, g, g), rgb(0x8e8e93));
        }
        Kind::Folder => {}
    }
}

/// Finder's alias badge: a small curved arrow in the bottom-left corner.
pub fn alias_badge(scene: &mut Scene, b: Bounds) {
    let size = b.w.min(b.h);
    let k = (size * 0.3).max(7.0);
    let r = Bounds::new(b.x + size * 0.08, b.bottom() - k - size * 0.06, k, k);
    scene.rect_bordered(r, rgb(0xffffff), k * 0.2, rgba(0x000000, 0.35), 0.6);
    let ink = rgb(0x1c1c1e);
    let w = (k / 7.0).max(1.0);
    let (x, y, s) = (r.x, r.y, k / 10.0);
    arc(scene, x + 7.2 * s, y + 7.6 * s, 4.4 * s, 180.0, 270.0, ink, w);
    scene.segment(pt(x + 7.2 * s, y + 3.2 * s), pt(x + 8.4 * s, y + 3.2 * s), w, ink);
    fill_triangle(scene, (x + 7.6 * s, y + 1.4 * s), (x + 9.6 * s, y + 3.2 * s), (x + 7.6 * s, y + 5.0 * s), ink);
}

/// The color of the extension label under a page's content.
pub fn label_color(kind: Kind) -> Color {
    match kind {
        Kind::Pdf => rgb(0xe0443e),
        Kind::Code => rgb(0x007aff),
        Kind::Audio => rgb(0xfc3c44),
        Kind::Spreadsheet => rgb(0x28a745),
        Kind::Presentation => rgb(0xf08c00),
        _ => rgb(0x8e8e93),
    }
}
