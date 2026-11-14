//! Turns the desktop into a list of things to draw: wallpaper, shadows, title
//! bars, window contents (with rounded corners), popups, Mission Control and
//! the app switcher.
//!
//! Everything is damage-tracked: elements keep their identity between frames
//! and only report changes when something actually changed, so an idle screen
//! costs nothing to "redraw".

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use cosmic_text::{Attrs, Buffer as TextBuffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree};
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement, RescaleRenderElement};
use smithay::backend::renderer::element::{AsRenderElements, Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::desktop::{PopupManager, Window, layer_map_for_output};
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::Layer;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

use crate::overview::SPACES_BAR_H;
use crate::state::{Desk, LIGHT_GAP, LIGHT_R, LIGHT_X, RADIUS, TITLEBAR_H, frame_of, is_ssd, win_data};

const OPEN_ANIM: Duration = Duration::from_millis(220);
/// How far shadows reach past a window frame: the soft layer's blur (3σ)
/// plus its drop, with room to spare so it always fades out fully.
const SHADOW_MARGIN: i32 = 120;
const ACCENT: [f32; 4] = [0.039, 0.518, 1.0, 1.0];

type Moved<E> = RelocateRenderElement<RescaleRenderElement<E>>;

smithay::backend::renderer::element::render_elements! {
    pub DeskElement<=GlesRenderer>;
    Surface=Moved<WaylandSurfaceRenderElement<GlesRenderer>>,
    Rounded=Moved<RoundedSurface>,
    Shader=PixelShaderElement,
    Text=Moved<MemoryRenderBufferRenderElement<GlesRenderer>>,
}

pub struct Shaders {
    titlebar: GlesPixelProgram,
    shadow: GlesPixelProgram,
    wallpaper: GlesPixelProgram,
    rect: GlesPixelProgram,
    rounded: GlesTexProgram,
}

impl Shaders {
    pub fn compile(r: &mut GlesRenderer) -> Result<Self, GlesError> {
        let f = |n: &'static str| UniformName::new(n, UniformType::_1f);
        let f2 = |n: &'static str| UniformName::new(n, UniformType::_2f);
        let f3 = |n: &'static str| UniformName::new(n, UniformType::_3f);
        let f4 = |n: &'static str| UniformName::new(n, UniformType::_4f);
        Ok(Shaders {
            titlebar: r.compile_custom_pixel_shader(
                include_str!("shaders/titlebar.frag"),
                &[
                    f("scale"),
                    f("radius"),
                    f4("bg"),
                    f4("hairline"),
                    f("focused"),
                    f("hover"),
                    f("pressed"),
                    f("dark"),
                    f3("lights"),
                ],
            )?,
            shadow: r.compile_custom_pixel_shader(
                include_str!("shaders/shadow.frag"),
                &[f("scale"), f4("rect"), f("radius"), f3("edge"), f3("mid"), f3("far")],
            )?,
            wallpaper: r.compile_custom_pixel_shader(include_str!("shaders/wallpaper.frag"), &[f("dark")])?,
            rect: r.compile_custom_pixel_shader(
                include_str!("shaders/rect.frag"),
                &[f("scale"), f("radius"), f4("fill"), f4("border"), f("border_w")],
            )?,
            rounded: r.compile_custom_texture_shader(
                include_str!("shaders/rounded.frag"),
                &[f2("elem_size"), f4("geo"), f4("radii"), f("flip")],
            )?,
        })
    }
}

/// A pixel-shader element that keeps its identity and only reports damage
/// when its area or uniforms change.
struct ShaderSlot {
    elem: PixelShaderElement,
    key: Vec<f32>,
}

impl ShaderSlot {
    fn get(
        slot: &mut Option<ShaderSlot>,
        program: &GlesPixelProgram,
        area: Rectangle<i32, Logical>,
        alpha: f32,
        uniforms: Vec<(&'static str, Vec<f32>)>,
    ) -> PixelShaderElement {
        let mut key = vec![area.loc.x as f32, area.loc.y as f32, area.size.w as f32, area.size.h as f32, alpha];
        key.extend(uniforms.iter().flat_map(|(_, v)| v.iter().copied()));
        let to_uniforms = || {
            uniforms
                .iter()
                .map(|(name, v)| match v.as_slice() {
                    [a] => Uniform::new(*name, *a),
                    [a, b] => Uniform::new(*name, (*a, *b)),
                    [a, b, c] => Uniform::new(*name, (*a, *b, *c)),
                    [a, b, c, d] => Uniform::new(*name, (*a, *b, *c, *d)),
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>()
        };
        match slot {
            Some(s) if s.key == key => {}
            Some(s) if s.key[4] == alpha => {
                s.elem.resize(area, None);
                s.elem.update_uniforms(to_uniforms());
                s.key = key;
            }
            _ => {
                *slot = Some(ShaderSlot {
                    elem: PixelShaderElement::new(program.clone(), area, None, alpha, to_uniforms(), Kind::Unspecified),
                    key,
                });
            }
        }
        slot.as_ref().unwrap().elem.clone()
    }
}

/// Rendering state that belongs to one window.
#[derive(Default)]
struct WinLook {
    shadow: Option<ShaderSlot>,
    titlebar: Option<ShaderSlot>,
    title: Option<(String, bool, bool, i32, MemoryRenderBuffer, (i32, i32))>,
}

/// A scale about `pivot` followed by a move, applied to a whole window.
#[derive(Clone, Copy)]
struct Xf {
    pivot: Point<f64, Logical>,
    scale: f64,
    offset: Point<f64, Logical>,
}

impl Xf {
    fn identity() -> Self {
        Xf { pivot: Point::from((0.0, 0.0)), scale: 1.0, offset: Point::from((0.0, 0.0)) }
    }

    /// Maps rectangle `from` onto `to` (same aspect ratio assumed).
    fn fit(from: Rectangle<f64, Logical>, to: Rectangle<f64, Logical>) -> Self {
        Xf { pivot: from.loc, scale: to.size.w / from.size.w, offset: to.loc - from.loc }
    }

    fn rect(&self, r: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let loc = (r.loc.to_f64() - self.pivot).upscale(self.scale) + self.pivot + self.offset;
        Rectangle::new(loc.to_i32_round(), r.size.to_f64().upscale(self.scale).to_i32_round())
    }

    fn wrap<E: Element>(&self, e: E, out_scale: f64) -> Moved<E> {
        let pivot: Point<i32, Physical> = self.pivot.to_physical_precise_round(out_scale);
        let offset: Point<i32, Physical> = self.offset.to_physical_precise_round(out_scale);
        RelocateRenderElement::from_element(RescaleRenderElement::from_element(e, pivot, self.scale), offset, Relocate::Relative)
    }
}

pub struct Look {
    shaders: Shaders,
    wallpaper: Option<ShaderSlot>,
    /// Shader elements for system UI (Mission Control, the switcher), by name.
    slots: HashMap<String, Option<ShaderSlot>>,
    labels: HashMap<String, (MemoryRenderBuffer, (i32, i32))>,
    icons: HashMap<String, Option<MemoryRenderBuffer>>,
    fonts: FontSystem,
    swash: SwashCache,
}

impl Look {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let mut db = cosmic_text::fontdb::Database::new();
        for font in [
            include_bytes!("../../crates/ui/assets/fonts/Inter-Regular.ttf").as_slice(),
            include_bytes!("../../crates/ui/assets/fonts/Inter-Medium.ttf").as_slice(),
            include_bytes!("../../crates/ui/assets/fonts/Inter-SemiBold.ttf").as_slice(),
            include_bytes!("../../crates/ui/assets/fonts/Inter-Bold.ttf").as_slice(),
        ] {
            db.load_font_data(font.to_vec());
        }
        Ok(Look {
            shaders: Shaders::compile(renderer)?,
            wallpaper: None,
            slots: HashMap::new(),
            labels: HashMap::new(),
            icons: HashMap::new(),
            fonts: FontSystem::new_with_locale_and_db("en-US".into(), db),
            swash: SwashCache::new(),
        })
    }

    /// Renders text to a premultiplied BGRA buffer at `scale`. Returns it with
    /// its logical size.
    fn text_buffer(&mut self, text: &str, pt: f32, weight: Weight, color: [f32; 4], scale: i32) -> (MemoryRenderBuffer, (i32, i32)) {
        let size = pt * scale as f32;
        let mut buffer = TextBuffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        buffer.set_size(None, None);
        let attrs = Attrs::new().family(Family::Name("Inter")).weight(weight);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let w = buffer.layout_runs().map(|r| r.line_w).fold(0.0f32, f32::max).ceil() as i32;
        let h = (size * 1.25).ceil() as i32;
        let (w, h) = (w.max(1), h.max(1));
        let mut px = vec![0u8; (w * h * 4) as usize];
        let ink = TextColor::rgba(255, 255, 255, 255);
        buffer.draw(&mut self.fonts, &mut self.swash, ink, |x, y, bw, bh, c| {
            let cov = c.a() as f32 / 255.0;
            for yy in y.max(0)..(y + bh as i32).min(h) {
                for xx in x.max(0)..(x + bw as i32).min(w) {
                    let i = ((yy * w + xx) * 4) as usize;
                    let a = cov * color[3];
                    // Premultiplied BGRA, "over" the existing pixel.
                    let keep = 1.0 - a;
                    px[i] = (color[2] * a * 255.0 + px[i] as f32 * keep) as u8;
                    px[i + 1] = (color[1] * a * 255.0 + px[i + 1] as f32 * keep) as u8;
                    px[i + 2] = (color[0] * a * 255.0 + px[i + 2] as f32 * keep) as u8;
                    px[i + 3] = (a * 255.0 + px[i + 3] as f32 * keep) as u8;
                }
            }
        });
        let buf = MemoryRenderBuffer::from_slice(&px, Fourcc::Argb8888, (w, h), scale, Transform::Normal, None);
        (buf, (w / scale, h / scale))
    }

    /// A cached text label.
    fn label(&mut self, text: &str, pt: f32, weight: Weight, color: [f32; 4], scale: i32) -> (MemoryRenderBuffer, (i32, i32)) {
        let key = format!("{text}\u{1}{pt}\u{1}{}\u{1}{color:?}\u{1}{scale}", weight.0);
        if let Some(l) = self.labels.get(&key) {
            return l.clone();
        }
        if self.labels.len() > 256 {
            self.labels.clear();
        }
        let l = self.text_buffer(text, pt, weight, color, scale);
        self.labels.insert(key, l.clone());
        l
    }

    /// The app's icon, `pt` logical pixels square, rendered for `scale`.
    fn icon(&mut self, app_id: &str, pt: u32, scale: i32) -> Option<MemoryRenderBuffer> {
        let key = format!("{app_id}@{pt}x{scale}");
        self.icons
            .entry(key)
            .or_insert_with(|| {
                crate::icons::load(app_id, pt * scale as u32).map(|icon| {
                    let s = icon.size as i32;
                    MemoryRenderBuffer::from_slice(&icon.px, Fourcc::Argb8888, (s, s), scale, Transform::Normal, None)
                })
            })
            .clone()
    }

    fn rect(&mut self, name: &str, area: Rectangle<i32, Logical>, alpha: f32, scale: f64, radius: f32, fill: [f32; 4], border: [f32; 4], border_w: f32) -> PixelShaderElement {
        let program = self.shaders.rect.clone();
        let slot = self.slots.entry(name.to_string()).or_default();
        ShaderSlot::get(
            slot,
            &program,
            area,
            alpha,
            vec![
                ("scale", vec![scale as f32]),
                ("radius", vec![radius]),
                ("fill", fill.to_vec()),
                ("border", border.to_vec()),
                ("border_w", vec![border_w]),
            ],
        )
    }
}

fn win_look(window: &Window) -> &RefCell<WinLook> {
    window.user_data().insert_if_missing(|| RefCell::new(WinLook::default()));
    window.user_data().get::<RefCell<WinLook>>().unwrap()
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

pub fn app_id_and_title(window: &Window) -> (String, String) {
    window
        .toplevel()
        .map(|t| {
            with_states(t.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .map(|d| {
                        let d = d.lock().unwrap();
                        (d.app_id.clone().unwrap_or_default(), d.title.clone().unwrap_or_default())
                    })
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default()
}

struct Ctx<'a> {
    look: &'a mut Look,
    renderer: &'a mut GlesRenderer,
    scale: f64,
    int_scale: i32,
    dark: bool,
}

impl Ctx<'_> {
    /// Places a text label with its top-left at `at` (logical).
    fn text(&mut self, out: &mut Vec<DeskElement>, text: &str, pt: f32, weight: Weight, color: [f32; 4], at: Point<f64, Logical>, alpha: f32) -> (i32, i32) {
        let (buf, size) = self.look.label(text, pt, weight, color, self.int_scale);
        let loc: Point<f64, Physical> = at.to_physical_precise_round::<f64, i32>(self.scale).to_f64();
        if let Ok(elem) = MemoryRenderBufferRenderElement::from_buffer(self.renderer, loc, &buf, Some(alpha), None, Some(size.into()), Kind::Unspecified) {
            out.push(DeskElement::Text(Xf::identity().wrap(elem, self.scale)));
        }
        size
    }
}

/// Everything to draw on the output, front to back.
pub fn elements(desk: &mut Desk, look: &mut Look, renderer: &mut GlesRenderer) -> Vec<DeskElement> {
    let out_geo = desk.output_geometry();
    let scale = desk.output.as_ref().map_or(1.0, |o| o.current_scale().fractional_scale());
    let dark = desk.dark;
    let mut cx = Ctx { look, renderer, scale, int_scale: scale.ceil() as i32, dark };
    let mut out: Vec<DeskElement> = Vec::new();
    if desk.tick_spaces() {
        desk.needs_redraw = true;
    }

    switcher(desk, &mut cx, &mut out);
    layer_elements(desk, cx.renderer, &[Layer::Overlay, Layer::Top], scale, &mut out);

    let mc = desk.spaces.mc.value();
    let layout = if mc > 0.0 { desk.mc_layout() } else { Vec::new() };
    if mc > 0.0 {
        mission_control_chrome(desk, &mut cx, mc, &layout, &mut out);
    }

    let focused = desk.focused_window();
    let windows = desk.visible_windows();
    for (window, loc, slide) in windows.iter().rev() {
        let frame = frame_of(window, *loc).to_f64();
        let opening = win_data(window).borrow().opened.map_or(1.0, |t| {
            (t.elapsed().as_secs_f32() / OPEN_ANIM.as_secs_f32()).min(1.0)
        });
        if opening < 1.0 {
            desk.needs_redraw = true;
        }
        let e = ease_out(opening) as f64;
        let mut xf = Xf {
            pivot: frame.loc + frame.size.to_point().downscale(2.0),
            scale: 0.92 + 0.08 * e,
            offset: Point::from((*slide, 0.0)),
        };
        if let Some((_, target)) = layout.iter().find(|(w, _)| w == window) {
            let to = Rectangle::new(
                Point::from((lerp(frame.loc.x, target.loc.x, mc), lerp(frame.loc.y, target.loc.y, mc))),
                Size::from((lerp(frame.size.w, target.size.w, mc), lerp(frame.size.h, target.size.h, mc))),
            );
            xf = Xf::fit(frame, to);
        }
        let active = focused.as_ref() == Some(window) && mc < 0.5;
        draw_window(desk, &mut cx, window, *loc, xf, e as f32, active, &mut out);
    }

    if mc > 0.0 {
        // Dim the desktop behind Mission Control.
        let dim = cx.look.rect("mc-dim", out_geo, (mc * 0.45) as f32, scale, 0.0, [0.0, 0.0, 0.0, 1.0], [0.0; 4], 0.0);
        out.push(DeskElement::Shader(dim));
    }
    layer_elements(desk, cx.renderer, &[Layer::Bottom, Layer::Background], scale, &mut out);
    let wallpaper = ShaderSlot::get(&mut cx.look.wallpaper, &cx.look.shaders.wallpaper, out_geo, 1.0, vec![("dark", vec![dark as u8 as f32])]);
    out.push(DeskElement::Shader(wallpaper));
    out
}

/// One window: popups, title bar, contents and shadow, under transform `xf`.
#[allow(clippy::too_many_arguments)]
fn draw_window(desk: &Desk, cx: &mut Ctx, window: &Window, loc: Point<i32, Logical>, xf: Xf, alpha: f32, active: bool, out: &mut Vec<DeskElement>) {
    let scale = cx.scale;
    let dark = cx.dark;
    let ssd = is_ssd(window);
    let geo = window.geometry();
    let frame = frame_of(window, loc);
    let s = xf.scale as f32;

    // Popups (menus) sit above the window and aren't clipped.
    let toplevel = window.toplevel().unwrap();
    let surface = toplevel.wl_surface();
    let surface_loc = loc - geo.loc;
    for (popup, offset) in PopupManager::popups_for_surface(surface) {
        let at = (surface_loc + geo.loc + offset - popup.geometry().loc).to_physical_precise_round(scale);
        let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(cx.renderer, popup.wl_surface(), at, scale, alpha, Kind::Unspecified);
        out.extend(elems.into_iter().map(|e| DeskElement::Surface(xf.wrap(e, scale))));
    }

    let mut wl = win_look(window).borrow_mut();

    if ssd {
        // Title text.
        let (_, title) = app_id_and_title(window);
        let title = if title.is_empty() { "Untitled".to_string() } else { title };
        let ink = match (dark, active) {
            (true, true) => [1.0, 1.0, 1.0, 0.85],
            (true, false) => [1.0, 1.0, 1.0, 0.38],
            (false, true) => [0.0, 0.0, 0.0, 0.85],
            (false, false) => [0.0, 0.0, 0.0, 0.32],
        };
        let stale = wl.title.as_ref().is_none_or(|(t, a, d, sc, _, _)| *t != title || *a != active || *d != dark || *sc != cx.int_scale);
        if stale {
            let (buf, size) = cx.look.text_buffer(&title, 13.0, Weight::SEMIBOLD, ink, cx.int_scale);
            wl.title = Some((title, active, dark, cx.int_scale, buf, size));
        }
        let (_, _, _, _, buf, (tw, th)) = wl.title.as_ref().unwrap();
        let max_w = frame.size.w - 160;
        let x = (frame.loc.x + (frame.size.w - tw) / 2).max(frame.loc.x + 80);
        let y = frame.loc.y + (TITLEBAR_H - th) / 2;
        if max_w > 20 {
            let src = Rectangle::<f64, Logical>::from_size(((*tw).min(max_w) as f64, *th as f64).into());
            let at: Point<i32, Physical> = Point::<i32, Logical>::from((x, y)).to_physical_precise_round(scale);
            if let Ok(elem) = MemoryRenderBufferRenderElement::from_buffer(
                cx.renderer,
                at.to_f64(),
                buf,
                Some(alpha),
                Some(src),
                Some(((*tw).min(max_w), *th).into()),
                Kind::Unspecified,
            ) {
                out.push(DeskElement::Text(xf.wrap(elem, scale)));
            }
        }

        // Title bar background and traffic lights.
        let bar = Rectangle::new(frame.loc, (frame.size.w, TITLEBAR_H).into());
        let (bg, hairline) = match (dark, active) {
            (true, true) => ([0.200, 0.200, 0.208, 1.0], [0.0, 0.0, 0.0, 0.55]),
            (true, false) => ([0.161, 0.161, 0.165, 1.0], [0.0, 0.0, 0.0, 0.45]),
            (false, true) => ([0.918, 0.918, 0.922, 1.0], [0.0, 0.0, 0.0, 0.11]),
            (false, false) => ([0.965, 0.965, 0.965, 1.0], [0.0, 0.0, 0.0, 0.07]),
        };
        let hover = desk.hovered_lights.as_ref() == Some(window);
        let pressed = match &desk.pressed {
            Some((w, b)) if w == window => 1.0 + *b as u8 as f32,
            _ => 0.0,
        };
        let elem = ShaderSlot::get(
            &mut wl.titlebar,
            &cx.look.shaders.titlebar,
            xf.rect(bar),
            alpha,
            vec![
                ("scale", vec![(scale * xf.scale) as f32]),
                ("radius", vec![RADIUS * s]),
                ("bg", bg.to_vec()),
                ("hairline", hairline.to_vec()),
                ("focused", vec![active as u8 as f32]),
                ("hover", vec![hover as u8 as f32]),
                ("pressed", vec![pressed]),
                ("dark", vec![dark as u8 as f32]),
                ("lights", vec![LIGHT_X as f32 * s, LIGHT_GAP as f32 * s, LIGHT_R as f32 * s]),
            ],
        );
        out.push(DeskElement::Shader(elem));
    }

    // Window contents with rounded corners. Clients that draw their own
    // shadow (their geometry is inset in the buffer) are left alone.
    let own_shadow = geo.loc != Point::from((0, 0));
    let r = (RADIUS as f64 * scale) as f32;
    let radii = if ssd { [0.0, 0.0, r, r] } else { [r, r, r, r] };
    let surf_at = surface_loc.to_physical_precise_round(scale);
    let geo_phys: Rectangle<i32, Physical> = Rectangle::new(loc, geo.size).to_physical_precise_round(scale);
    let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(cx.renderer, surface, surf_at, scale, alpha, Kind::Unspecified);
    for elem in elems {
        if own_shadow {
            out.push(DeskElement::Surface(xf.wrap(elem, scale)));
        } else {
            let rounded = RoundedSurface::new(elem, cx.look.shaders.rounded.clone(), scale, geo_phys, radii);
            out.push(DeskElement::Rounded(xf.wrap(rounded, scale)));
        }
    }

    // Shadow.
    if !own_shadow {
        let area = Rectangle::new(
            frame.loc - Point::from((SHADOW_MARGIN, SHADOW_MARGIN)),
            (frame.size.w + 2 * SHADOW_MARGIN, frame.size.h + 2 * SHADOW_MARGIN).into(),
        );
        let m = SHADOW_MARGIN as f32 * s;
        let (fw, fh) = (frame.size.w as f32 * s, frame.size.h as f32 * s);
        // (sigma, drop, opacity) per layer; the active window floats higher.
        let (edge, mid, far) = match (active, dark) {
            (true, false) => ([0.6, 0.0, 0.22], [5.0, 2.0, 0.12], [26.0, 20.0, 0.30]),
            (false, false) => ([0.6, 0.0, 0.16], [4.0, 1.0, 0.08], [16.0, 10.0, 0.16]),
            (true, true) => ([0.6, 0.0, 0.55], [5.0, 2.0, 0.25], [26.0, 20.0, 0.50]),
            (false, true) => ([0.6, 0.0, 0.45], [4.0, 1.0, 0.18], [16.0, 10.0, 0.30]),
        };
        let k = |l: [f32; 3]| vec![l[0] * s, l[1] * s, l[2]];
        let elem = ShaderSlot::get(
            &mut wl.shadow,
            &cx.look.shaders.shadow,
            xf.rect(area),
            alpha,
            vec![
                ("scale", vec![scale as f32]),
                ("rect", vec![m, m, fw, fh]),
                ("radius", vec![RADIUS * s]),
                ("edge", k(edge)),
                ("mid", k(mid)),
                ("far", k(far)),
            ],
        );
        out.push(DeskElement::Shader(elem));
    }
}

/// The spaces bar, the hovered window's outline and its name.
fn mission_control_chrome(desk: &Desk, cx: &mut Ctx, mc: f64, layout: &[(Window, Rectangle<f64, Logical>)], out: &mut Vec<DeskElement>) {
    let scale = cx.scale;
    let a = mc as f32;
    let white = [1.0, 1.0, 1.0, 0.92];

    // Hovered window: blue outline and its name below it.
    if let Some((w, r)) = layout.iter().find(|(w, _)| desk.spaces.mc_hover.as_ref() == Some(w)) {
        if mc > 0.85 {
            let (app_id, title) = app_id_and_title(w);
            let name = crate::icons::desktop_entry(&app_id).map(|(_, n)| n).unwrap_or(if title.is_empty() { app_id } else { title });
            let pad = 5.0;
            let rect = Rectangle::new((r.loc.x - pad, r.loc.y - pad).into(), (r.size.w + 2.0 * pad, r.size.h + 2.0 * pad).into());
            let ring = cx.look.rect("mc-ring", rect.to_i32_round(), a, scale, RADIUS * (r.size.w / desk.frame(w).map_or(r.size.w, |f| f.size.w as f64)) as f32 + pad as f32, [0.0; 4], ACCENT, 3.5);
            out.insert(0, DeskElement::Shader(ring));
            let (lw, lh) = cx.look.label(&name, 13.0, Weight::MEDIUM, white, cx.int_scale).1;
            let pill = Rectangle::<f64, Logical>::new(
                (r.loc.x + r.size.w / 2.0 - lw as f64 / 2.0 - 12.0, r.loc.y + r.size.h + 14.0).into(),
                ((lw + 24) as f64, (lh + 8) as f64).into(),
            );
            let mut texts = Vec::new();
            cx.text(&mut texts, &name, 13.0, Weight::MEDIUM, white, (pill.loc.x + 12.0, pill.loc.y + 4.0).into(), a);
            out.splice(0..0, texts);
            let bg = cx.look.rect("mc-pill", pill.to_i32_round(), a, scale, 999.0, [0.12, 0.12, 0.13, 0.82], [1.0, 1.0, 1.0, 0.12], 0.5);
            out.insert(1, DeskElement::Shader(bg));
        }
    }

    // The spaces bar slides down from the top.
    let drop = -(1.0 - mc) * SPACES_BAR_H;
    let (tiles, add) = desk.space_tiles();
    for (i, t) in tiles.iter().enumerate() {
        let t = Rectangle::<f64, Logical>::new((t.loc.x, t.loc.y + drop).into(), t.size);
        let label = format!("Desktop {}", i + 1);
        let (lw, _) = cx.look.label(&label, 11.0, Weight::MEDIUM, white, cx.int_scale).1;
        cx.text(out, &label, 11.0, Weight::MEDIUM, white, (t.loc.x + t.size.w / 2.0 - lw as f64 / 2.0, t.loc.y + t.size.h + 6.0).into(), a);
        let current = i == desk.spaces.active;
        let border = if current { [1.0, 1.0, 1.0, 0.95] } else { [1.0, 1.0, 1.0, 0.25] };
        let frame = cx.look.rect(&format!("space-frame-{i}"), t.to_i32_round(), a, scale, 8.0, [0.0; 4], border, if current { 2.5 } else { 1.0 });
        out.push(DeskElement::Shader(frame));
        let program = cx.look.shaders.wallpaper.clone();
        let slot = cx.look.slots.entry(format!("space-wall-{i}")).or_default();
        let wall = ShaderSlot::get(slot, &program, t.to_i32_round(), a, vec![("dark", vec![cx.dark as u8 as f32])]);
        out.push(DeskElement::Shader(wall));
    }
    let add = Rectangle::<f64, Logical>::new((add.loc.x, add.loc.y + drop).into(), add.size);
    let (pw, ph) = cx.look.label("+", 22.0, Weight::MEDIUM, white, cx.int_scale).1;
    cx.text(out, "+", 22.0, Weight::MEDIUM, white, (add.loc.x + add.size.w / 2.0 - pw as f64 / 2.0, add.loc.y + add.size.h / 2.0 - ph as f64 / 2.0).into(), a);
    let plus = cx.look.rect("space-add", add.to_i32_round(), a, scale, 999.0, [1.0, 1.0, 1.0, 0.16], [1.0, 1.0, 1.0, 0.2], 0.5);
    out.push(DeskElement::Shader(plus));
}

/// The Super+Tab switcher: app icons in a panel, the chosen one highlighted.
fn switcher(desk: &Desk, cx: &mut Ctx, out: &mut Vec<DeskElement>) {
    let Some(sw) = &desk.spaces.switcher else { return };
    let scale = cx.scale;
    let out_geo = desk.output_geometry().to_f64();
    let icon = 96.0;
    let pad = 22.0;
    let gap = 18.0;
    let n = sw.windows.len() as f64;
    let w = n * icon + (n - 1.0) * gap + 2.0 * pad;
    let h = icon + 2.0 * pad + 26.0;
    let panel = Rectangle::<f64, Logical>::new(
        (out_geo.loc.x + (out_geo.size.w - w) / 2.0, out_geo.loc.y + (out_geo.size.h - h) / 2.0).into(),
        (w, h).into(),
    );
    let ink = if cx.dark { [1.0, 1.0, 1.0, 0.9] } else { [0.0, 0.0, 0.0, 0.85] };
    let mut front = Vec::new();
    for (i, window) in sw.windows.iter().enumerate() {
        let x = panel.loc.x + pad + i as f64 * (icon + gap);
        let y = panel.loc.y + pad;
        let (app_id, title) = app_id_and_title(window);
        match cx.look.icon(&app_id, icon as u32, cx.int_scale) {
            Some(buf) => {
                let loc: Point<f64, Physical> = Point::<f64, Logical>::from((x, y)).to_physical_precise_round::<f64, i32>(scale).to_f64();
                if let Ok(elem) = MemoryRenderBufferRenderElement::from_buffer(cx.renderer, loc, &buf, None, None, None, Kind::Unspecified) {
                    front.push(DeskElement::Text(Xf::identity().wrap(elem, scale)));
                }
            }
            None => {
                // No icon: a rounded tile with the app's initial.
                let initial: String = app_id.rsplit('.').next().unwrap_or("?").chars().take(1).collect::<String>().to_uppercase();
                let (lw, lh) = cx.look.label(&initial, 40.0, Weight::BOLD, [1.0; 4], cx.int_scale).1;
                cx.text(&mut front, &initial, 40.0, Weight::BOLD, [1.0; 4], (x + icon / 2.0 - lw as f64 / 2.0, y + icon / 2.0 - lh as f64 / 2.0).into(), 1.0);
                let tile = Rectangle::<f64, Logical>::new((x + 8.0, y + 8.0).into(), (icon - 16.0, icon - 16.0).into());
                front.push(DeskElement::Shader(cx.look.rect(&format!("sw-tile-{i}"), tile.to_i32_round(), 1.0, scale, 18.0, [0.36, 0.38, 0.44, 1.0], [0.0; 4], 0.0)));
            }
        }
        if i == sw.index {
            let name = crate::icons::desktop_entry(&app_id).map(|(_, n)| n).unwrap_or(if title.is_empty() { app_id.clone() } else { title });
            let (lw, _) = cx.look.label(&name, 12.0, Weight::MEDIUM, ink, cx.int_scale).1;
            let lx = (x + icon / 2.0 - lw as f64 / 2.0).clamp(panel.loc.x + 8.0, panel.loc.x + panel.size.w - lw as f64 - 8.0);
            cx.text(&mut front, &name, 12.0, Weight::MEDIUM, ink, (lx, y + icon + 8.0).into(), 1.0);
            let hl = Rectangle::<f64, Logical>::new((x - 8.0, y - 8.0).into(), (icon + 16.0, icon + 16.0).into());
            let fill = if cx.dark { [1.0, 1.0, 1.0, 0.16] } else { [0.0, 0.0, 0.0, 0.10] };
            front.push(DeskElement::Shader(cx.look.rect("sw-hl", hl.to_i32_round(), 1.0, scale, 16.0, fill, [0.0; 4], 0.0)));
        }
    }
    let (fill, border) = if cx.dark {
        ([0.17, 0.17, 0.18, 0.94], [1.0, 1.0, 1.0, 0.14])
    } else {
        ([0.96, 0.96, 0.97, 0.94], [0.0, 0.0, 0.0, 0.12])
    };
    front.push(DeskElement::Shader(cx.look.rect("sw-panel", panel.to_i32_round(), 1.0, scale, 24.0, fill, border, 0.5)));
    out.extend(front);
}

/// Layer surfaces (menu bar, panels) on the given layers, top first.
fn layer_elements(desk: &Desk, renderer: &mut GlesRenderer, layers: &[Layer], scale: f64, out: &mut Vec<DeskElement>) {
    let Some(output) = &desk.output else { return };
    let map = layer_map_for_output(output);
    let origin = desk.output_geometry().loc;
    for &kind in layers {
        for layer in map.layers_on(kind).rev() {
            let Some(geo) = map.layer_geometry(layer) else { continue };
            let at = (geo.loc + origin).to_physical_precise_round(scale);
            let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                layer.render_elements(renderer, at, Scale::from(scale), 1.0);
            out.extend(elems.into_iter().map(|e| DeskElement::Surface(Xf::identity().wrap(e, scale))));
        }
    }
}

/// A window surface drawn through the corner-rounding shader.
#[derive(Debug)]
pub struct RoundedSurface {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
}

impl RoundedSurface {
    fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        program: GlesTexProgram,
        scale: f64,
        window_geo: Rectangle<i32, Physical>,
        radii: [f32; 4],
    ) -> Self {
        let elem = inner.geometry(Scale::from(scale));
        let rel = window_geo.loc - elem.loc;
        let uniforms = vec![
            Uniform::new("elem_size", (elem.size.w as f32, elem.size.h as f32)),
            Uniform::new("geo", (rel.x as f32, rel.y as f32, window_geo.size.w as f32, window_geo.size.h as f32)),
            Uniform::new("radii", (radii[0], radii[1], radii[2], radii[3])),
            Uniform::new("flip", 0.0f32),
        ];
        RoundedSurface { inner, program, uniforms }
    }
}

impl Element for RoundedSurface {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        // The rounded corners are see-through.
        OpaqueRegions::default()
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for RoundedSurface {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(self.program.clone(), self.uniforms.clone());
        let result = self.inner.draw(frame, src, dst, damage, opaque_regions);
        frame.clear_tex_program_override();
        result
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// Frame callbacks for everything visible, so clients draw their next frame.
pub fn send_frames(desk: &Desk) {
    let Some(output) = desk.output.clone() else { return };
    let time = desk.start.elapsed();
    for window in desk.space.elements() {
        window.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
}
