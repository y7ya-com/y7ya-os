//! Turns the desktop into a list of things to draw: wallpaper, shadows, title
//! bars, window contents (with rounded corners) and popups.
//!
//! Everything is damage-tracked: elements keep their identity between frames
//! and only report changes when something actually changed, so an idle screen
//! costs nothing to "redraw".

use std::cell::RefCell;
use std::time::Duration;

use cosmic_text::{Attrs, Buffer as TextBuffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree};
use smithay::backend::renderer::element::utils::RescaleRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::desktop::{PopupManager, Window};
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Transform};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

use crate::state::{Desk, LIGHT_GAP, LIGHT_R, LIGHT_X, RADIUS, TITLEBAR_H, frame_of, is_ssd, win_data};

const OPEN_ANIM: Duration = Duration::from_millis(220);
/// How far shadows reach past a window frame.
const SHADOW_MARGIN: i32 = 64;

smithay::backend::renderer::element::render_elements! {
    pub DeskElement<=GlesRenderer>;
    Surface=RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Rounded=RescaleRenderElement<RoundedSurface>,
    Shader=PixelShaderElement,
    Text=RescaleRenderElement<MemoryRenderBufferRenderElement<GlesRenderer>>,
}

pub struct Shaders {
    titlebar: GlesPixelProgram,
    shadow: GlesPixelProgram,
    wallpaper: GlesPixelProgram,
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
                &[f("scale"), f4("rect"), f("radius"), f4("near"), f4("far"), f2("sigmas"), f("far_dy")],
            )?,
            wallpaper: r.compile_custom_pixel_shader(include_str!("shaders/wallpaper.frag"), &[f("dark")])?,
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

pub struct Look {
    shaders: Shaders,
    wallpaper: Option<ShaderSlot>,
    fonts: FontSystem,
    swash: SwashCache,
}

impl Look {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let mut db = cosmic_text::fontdb::Database::new();
        for font in [
            include_bytes!("../../crates/ui/assets/fonts/Inter-Medium.ttf").as_slice(),
            include_bytes!("../../crates/ui/assets/fonts/Inter-SemiBold.ttf").as_slice(),
        ] {
            db.load_font_data(font.to_vec());
        }
        Ok(Look {
            shaders: Shaders::compile(renderer)?,
            wallpaper: None,
            fonts: FontSystem::new_with_locale_and_db("en-US".into(), db),
            swash: SwashCache::new(),
        })
    }

    /// Renders a window title to a premultiplied BGRA buffer at `scale`.
    fn title_buffer(&mut self, title: &str, color: [f32; 4], scale: i32) -> (MemoryRenderBuffer, (i32, i32)) {
        let size = 13.0 * scale as f32;
        let mut buffer = TextBuffer::new(&mut self.fonts, Metrics::new(size, size * 1.25));
        buffer.set_size(None, None);
        let attrs = Attrs::new().family(Family::Name("Inter")).weight(Weight::SEMIBOLD);
        buffer.set_text(title, &attrs, Shaping::Advanced, None);
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
}

fn win_look(window: &Window) -> &RefCell<WinLook> {
    window.user_data().insert_if_missing(|| RefCell::new(WinLook::default()));
    window.user_data().get::<RefCell<WinLook>>().unwrap()
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Everything to draw on the output, front to back.
pub fn elements(desk: &mut Desk, look: &mut Look, renderer: &mut GlesRenderer) -> Vec<DeskElement> {
    let out_geo = desk.output_geometry();
    let scale = desk.output.as_ref().map_or(1.0, |o| o.current_scale().fractional_scale());
    let int_scale = scale.ceil() as i32;
    let focused = desk.focused_window();
    let dark = desk.dark;
    let mut out: Vec<DeskElement> = Vec::new();
    let windows: Vec<Window> = desk.space.elements().rev().cloned().collect();

    for window in &windows {
        let Some(loc) = desk.space.element_location(window) else { continue };
        let ssd = is_ssd(window);
        let geo = window.geometry();
        let frame = frame_of(window, loc);
        let active = focused.as_ref() == Some(window);

        // Opening: fade in while growing from 92%.
        let progress = win_data(window).borrow().opened.map_or(1.0, |t| {
            (t.elapsed().as_secs_f32() / OPEN_ANIM.as_secs_f32()).min(1.0)
        });
        if progress < 1.0 {
            desk.needs_redraw = true;
        }
        let e = ease_out(progress);
        let alpha = e;
        let zoom = 0.92 + 0.08 * e;
        let center = frame.loc.to_f64() + frame.size.to_f64().to_point().downscale(2.0);
        let origin: Point<i32, Physical> = center.to_physical_precise_round(scale);
        let scaled = |r: Rectangle<i32, Logical>| -> Rectangle<i32, Logical> {
            let loc = (r.loc.to_f64() - center).upscale(zoom as f64) + center;
            Rectangle::new(loc.to_i32_round(), r.size.to_f64().upscale(zoom as f64).to_i32_round())
        };

        // Popups (menus) sit above the window and aren't clipped.
        let toplevel = window.toplevel().unwrap();
        let surface = toplevel.wl_surface();
        let surface_loc = loc - geo.loc;
        for (popup, offset) in PopupManager::popups_for_surface(surface) {
            let at = (surface_loc + geo.loc + offset - popup.geometry().loc).to_physical_precise_round(scale);
            let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(renderer, popup.wl_surface(), at, scale, alpha, Kind::Unspecified);
            out.extend(elems.into_iter().map(|e| DeskElement::Surface(RescaleRenderElement::from_element(e, origin, zoom as f64))));
        }

        let mut wl = win_look(window).borrow_mut();

        // Title text.
        if ssd {
            let title = with_states(surface, |states| {
                states.data_map.get::<XdgToplevelSurfaceData>().and_then(|d| d.lock().unwrap().title.clone())
            })
            .unwrap_or_default();
            let title = if title.is_empty() { "Untitled".to_string() } else { title };
            let ink = match (dark, active) {
                (true, true) => [1.0, 1.0, 1.0, 0.85],
                (true, false) => [1.0, 1.0, 1.0, 0.38],
                (false, true) => [0.0, 0.0, 0.0, 0.85],
                (false, false) => [0.0, 0.0, 0.0, 0.32],
            };
            let stale = wl.title.as_ref().is_none_or(|(t, a, d, s, _, _)| *t != title || *a != active || *d != dark || *s != int_scale);
            if stale {
                let (buf, size) = look.title_buffer(&title, ink, int_scale);
                wl.title = Some((title, active, dark, int_scale, buf, size));
            }
            let (_, _, _, _, buf, (tw, th)) = wl.title.as_ref().unwrap();
            let min_x = frame.loc.x + 80;
            let max_w = frame.size.w - 160;
            let x = (frame.loc.x + (frame.size.w - tw) / 2).max(min_x);
            let y = frame.loc.y + (TITLEBAR_H - th) / 2;
            if max_w > 20 {
                let src = Rectangle::<f64, Logical>::from_size(((*tw).min(max_w) as f64, *th as f64).into());
                let at: Point<i32, Physical> = Point::<i32, Logical>::from((x, y)).to_physical_precise_round(scale);
                if let Ok(elem) = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    at.to_f64(),
                    buf,
                    Some(alpha),
                    Some(src),
                    Some(((*tw).min(max_w), *th).into()),
                    Kind::Unspecified,
                ) {
                    out.push(DeskElement::Text(RescaleRenderElement::from_element(elem, origin, zoom as f64)));
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
                &look.shaders.titlebar,
                scaled(bar),
                alpha,
                vec![
                    ("scale", vec![(scale * zoom as f64) as f32]),
                    ("radius", vec![RADIUS]),
                    ("bg", bg.to_vec()),
                    ("hairline", hairline.to_vec()),
                    ("focused", vec![active as u8 as f32]),
                    ("hover", vec![hover as u8 as f32]),
                    ("pressed", vec![pressed]),
                    ("dark", vec![dark as u8 as f32]),
                    ("lights", vec![LIGHT_X as f32, LIGHT_GAP as f32, LIGHT_R as f32]),
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
            render_elements_from_surface_tree(renderer, surface, surf_at, scale, alpha, Kind::Unspecified);
        for elem in elems {
            if own_shadow {
                out.push(DeskElement::Surface(RescaleRenderElement::from_element(elem, origin, zoom as f64)));
            } else {
                let rounded = RoundedSurface::new(elem, look.shaders.rounded.clone(), scale, geo_phys, radii);
                out.push(DeskElement::Rounded(RescaleRenderElement::from_element(rounded, origin, zoom as f64)));
            }
        }

        // Shadow.
        if !own_shadow {
            let area = Rectangle::new(
                frame.loc - Point::from((SHADOW_MARGIN, SHADOW_MARGIN)),
                (frame.size.w + 2 * SHADOW_MARGIN, frame.size.h + 2 * SHADOW_MARGIN).into(),
            );
            let area = scaled(area);
            let m = SHADOW_MARGIN as f32 * zoom;
            let (fw, fh) = (frame.size.w as f32 * zoom, frame.size.h as f32 * zoom);
            let (near_a, far_a, far_sigma) = if active { (0.30, 0.42, 22.0) } else { (0.22, 0.20, 16.0) };
            let near_a = if dark { near_a + 0.25 } else { near_a };
            let elem = ShaderSlot::get(
                &mut wl.shadow,
                &look.shaders.shadow,
                area,
                alpha,
                vec![
                    ("scale", vec![scale as f32]),
                    ("rect", vec![m, m, fw, fh]),
                    ("radius", vec![RADIUS * zoom]),
                    ("near", vec![0.0, 0.0, 0.0, near_a]),
                    ("far", vec![0.0, 0.0, 0.0, far_a]),
                    ("sigmas", vec![1.2, far_sigma]),
                    ("far_dy", vec![if active { 14.0 } else { 8.0 }]),
                ],
            );
            out.push(DeskElement::Shader(elem));
        }
    }

    let wallpaper = ShaderSlot::get(&mut look.wallpaper, &look.shaders.wallpaper, out_geo, 1.0, vec![("dark", vec![dark as u8 as f32])]);
    out.push(DeskElement::Shader(wallpaper));
    out
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
