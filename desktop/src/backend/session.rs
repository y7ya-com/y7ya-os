//! The real thing: runs as a login session, drawing to the built-in screen
//! through DRM/KMS and reading keyboards, mice and touchpads through libinput.
//!
//! Frames are paced by the display's vblank and only rendered when something
//! changed; the pointer lives on the hardware cursor plane, so moving it
//! doesn't redraw the screen.

use std::cell::RefCell;
use std::rc::Rc;

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportMemWl;

use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{all_gpus, primary_gpu};
use smithay::input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData};
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::{Device as _, ModeTypeFlags, connector, crtc};
use smithay::reexports::input::{self, Libinput};
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::{DeviceFd, Logical, Point, Transform};
use smithay::wayland::compositor::with_states;

use crate::render::{self, DeskElement, Look};
use crate::state::Desk;

type Output_ = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

struct Screen {
    renderer: GlesRenderer,
    output: Output_,
    look: Look,
    cursors: Cursors,
    /// A frame is on its way to the screen; wait for its vblank.
    pending: bool,
    active: bool,
}

pub fn init(event_loop: &mut EventLoop<'static, Desk>, desk: &mut Desk) -> anyhow::Result<Box<dyn FnMut(&mut Desk)>> {
    let (mut session, notifier) = LibSeatSession::new().map_err(|e| anyhow::anyhow!("no session: {e}"))?;
    let seat = session.seat();

    // The GPU that drives the built-in display.
    let path = primary_gpu(&seat)?.or_else(|| all_gpus(&seat).ok()?.into_iter().next()).ok_or_else(|| anyhow::anyhow!("no GPU"))?;
    let fd = session
        .open(&path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
        .map_err(|e| anyhow::anyhow!("can't open {}: {e}", path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_events) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd.clone())?;
    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&display)?;
    let mut renderer = unsafe { GlesRenderer::new(context)? };
    let render_node = DrmNode::from_file(&fd).ok().and_then(|n| n.node_with_type(NodeType::Render)?.ok());

    let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), render_node);
    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
    let mut manager = DrmOutputManager::new(
        drm,
        allocator,
        exporter,
        Some(gbm),
        [Fourcc::Abgr8888, Fourcc::Argb8888],
        render_formats,
    );

    // The built-in panel if there is one, else the first connected screen.
    let (conn, crtc) = pick_connector(manager.device()).ok_or_else(|| anyhow::anyhow!("no connected display"))?;
    let mode = conn.modes().iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).copied().unwrap_or(conn.modes()[0]);
    let (w, h) = mode.size();
    let scale = std::env::var("Y7_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| guess_scale(&conn, w as i32));
    let name = format!("{}-{}", conn.interface().as_str(), conn.interface_id());
    let (mm_w, mm_h) = conn.size().unwrap_or((0, 0));
    let output = Output::new(
        name,
        PhysicalProperties { size: (mm_w as i32, mm_h as i32).into(), subpixel: Subpixel::Unknown, make: "y7ya-os".into(), model: "display".into() },
    );
    let _global = output.create_global::<Desk>(&desk.display_handle);
    let wl_mode = Mode::from(mode);
    output.change_current_state(Some(wl_mode), Some(Transform::Normal), Some(Scale::Fractional(scale)), Some((0, 0).into()));
    output.set_preferred(wl_mode);
    desk.space.map_output(&output, (0, 0));
    desk.output = Some(output.clone());
    let _ = (w, h);

    let drm_output = manager
        .initialize_output::<_, DeskElement>(
            crtc,
            mode,
            &[conn.handle()],
            &output,
            None,
            &mut renderer,
            &DrmOutputRenderElements::default(),
        )
        .map_err(|e| anyhow::anyhow!("can't drive the display: {e:?}"))?;

    desk.init_dmabuf(&renderer);
    desk.shm_formats(renderer.shm_formats());
    let look = Look::new(&mut renderer)?;
    let screen = Rc::new(RefCell::new(Screen { renderer, output: drm_output, look, cursors: Cursors::load(), pending: false, active: true }));

    // Vblank: the last frame is on screen, so clients may draw their next one.
    let s = screen.clone();
    event_loop
        .handle()
        .insert_source(drm_events, move |event, _, desk: &mut Desk| match event {
            DrmEvent::VBlank(_) => {
                let mut sc = s.borrow_mut();
                if let Err(e) = sc.output.frame_submitted() {
                    tracing::warn!("frame not submitted: {e:?}");
                }
                sc.pending = false;
                render::send_frames(desk);
                drop(sc);
                if desk.needs_redraw {
                    s.borrow_mut().render(desk);
                }
            }
            DrmEvent::Error(e) => tracing::error!("display error: {e:?}"),
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Input.
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|_| anyhow::anyhow!("libinput can't take seat {seat}"))?;
    event_loop
        .handle()
        .insert_source(LibinputInputBackend::new(libinput.clone()), |mut event, _, desk: &mut Desk| {
            if let InputEvent::DeviceAdded { device } = &mut event {
                configure_device(device);
            }
            desk.process_input(event);
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Switching to another VT pauses us; coming back resumes.
    let s = screen.clone();
    let mut li = libinput.clone();
    event_loop
        .handle()
        .insert_source(notifier, move |event, _, desk: &mut Desk| match event {
            SessionEvent::PauseSession => {
                li.suspend();
                manager.pause();
                s.borrow_mut().active = false;
            }
            SessionEvent::ActivateSession => {
                if li.resume().is_err() {
                    tracing::warn!("couldn't resume input");
                }
                if let Err(e) = manager.activate(false) {
                    tracing::error!("couldn't resume the display: {e:?}");
                }
                let mut sc = s.borrow_mut();
                sc.active = true;
                sc.pending = false;
                drop(sc);
                desk.redraw();
            }
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    desk.vt_switch = Some(Box::new(move |vt| {
        if let Err(e) = session.change_vt(vt) {
            tracing::warn!("couldn't switch to VT {vt}: {e}");
        }
    }));
    desk.draws_cursor = true;
    desk.redraw();

    Ok(Box::new(move |desk: &mut Desk| {
        if desk.needs_redraw {
            screen.borrow_mut().render(desk);
        }
    }))
}

impl Screen {
    fn render(&mut self, desk: &mut Desk) {
        if !self.active || self.pending {
            return;
        }
        desk.needs_redraw = false;
        let scale = desk.output.as_ref().map_or(1.0, |o| o.current_scale().fractional_scale());
        let mut elements = cursor_elements(desk, &mut self.renderer, &mut self.cursors, scale);
        elements.extend(render::elements(desk, &mut self.look, &mut self.renderer));
        match self.output.render_frame(&mut self.renderer, &elements, [0.0, 0.0, 0.0, 1.0], FrameFlags::DEFAULT) {
            Ok(result) if !result.is_empty => match self.output.queue_frame(()) {
                Ok(()) => self.pending = true,
                Err(e) => tracing::warn!("couldn't queue frame: {e:?}"),
            },
            Ok(_) => render::send_frames(desk),
            Err(e) => tracing::warn!("render failed: {e:?}"),
        }
    }
}

/// The internal panel (eDP/LVDS/DSI) if connected, else any connected screen,
/// with a CRTC that can drive it.
fn pick_connector(drm: &DrmDevice) -> Option<(connector::Info, crtc::Handle)> {
    let res = drm.resource_handles().ok()?;
    let mut connected: Vec<connector::Info> = res
        .connectors()
        .iter()
        .filter_map(|c| drm.get_connector(*c, true).ok())
        .filter(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
        .collect();
    connected.sort_by_key(|c| {
        !matches!(c.interface(), connector::Interface::EmbeddedDisplayPort | connector::Interface::LVDS | connector::Interface::DSI)
    });
    for conn in connected {
        for enc in conn.encoders().iter().filter_map(|e| drm.get_encoder(*e).ok()) {
            if let Some(crtc) = res.filter_crtcs(enc.possible_crtcs()).first() {
                return Some((conn, *crtc));
            }
        }
    }
    None
}

/// Picks a scale from pixel density: Retina-like panels get 2x.
fn guess_scale(conn: &connector::Info, width_px: i32) -> f64 {
    let Some((mm, _)) = conn.size().filter(|(w, _)| *w > 0) else { return 1.0 };
    let dpi = width_px as f64 / (mm as f64 / 25.4);
    match dpi {
        d if d >= 190.0 => 2.0,
        d if d >= 150.0 => 1.5,
        d if d >= 125.0 => 1.25,
        _ => 1.0,
    }
}

/// Mac-like touchpad defaults: tap to click, natural scrolling, two-finger
/// right click, and no clicking while typing.
fn configure_device(device: &mut input::Device) {
    if device.config_tap_finger_count() > 0 {
        let _ = device.config_tap_set_enabled(true);
        let _ = device.config_dwt_set_enabled(true);
        let _ = device.config_scroll_set_natural_scroll_enabled(true);
        let _ = device.config_click_set_method(input::ClickMethod::Clickfinger);
    }
}

/// The pointer: a client's own cursor surface, or one from the cursor theme.
fn cursor_elements(desk: &Desk, renderer: &mut GlesRenderer, cursors: &mut Cursors, scale: f64) -> Vec<DeskElement> {
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
struct Cursors {
    theme: xcursor::CursorTheme,
    size: u32,
    cache: std::collections::HashMap<(CursorIcon, i32), Option<(MemoryRenderBuffer, Point<i32, Logical>)>>,
}

impl Cursors {
    fn load() -> Self {
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
