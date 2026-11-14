//! Runs the desktop in a window inside another desktop, for development.

use std::cell::RefCell;
use std::rc::Rc;

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Scale};
use smithay::reexports::calloop::EventLoop;
use smithay::utils::Transform;

use crate::render::{self, Look};
use crate::state::Desk;

/// Starts the nested backend. Returns a hook to call after each loop
/// iteration; it schedules a frame when something changed.
pub fn init(event_loop: &mut EventLoop<'static, Desk>, desk: &mut Desk) -> anyhow::Result<Box<dyn FnMut(&mut Desk)>> {
    let (mut backend, winit) = winit::init::<GlesRenderer>().map_err(|e| anyhow::anyhow!("{e}"))?;
    backend.window().set_title("y7ya-os");
    let size = backend.window_size();
    let scale = backend.window().scale_factor();
    let output = super::add_output(desk, "nested", (size.w, size.h), scale, Transform::Flipped180);
    let mut look = Look::new(backend.renderer())?;
    desk.init_dmabuf(backend.renderer());
    let mut damage = OutputDamageTracker::from_output(&output);
    let backend: Rc<RefCell<WinitGraphicsBackend<GlesRenderer>>> = Rc::new(RefCell::new(backend));

    let b = backend.clone();
    event_loop
        .handle()
        .insert_source(winit, move |event, _, desk: &mut Desk| match event {
            WinitEvent::Resized { size, scale_factor } => {
                output.change_current_state(
                    Some(Mode { size, refresh: 60_000 }),
                    None,
                    Some(Scale::Fractional(scale_factor)),
                    None,
                );
                desk.redraw();
            }
            WinitEvent::Input(event) => desk.process_input(event),
            WinitEvent::Redraw => {
                desk.needs_redraw = false;
                let mut backend = b.borrow_mut();
                let age = backend.buffer_age().unwrap_or(0);
                let submitted = match backend.bind() {
                    Ok((renderer, mut fb)) => {
                        let elements = render::elements(desk, &mut look, renderer);
                        match damage.render_output(renderer, &mut fb, age, &elements, [0.0, 0.0, 0.0, 1.0]) {
                            Ok(result) => result.damage.cloned(),
                            Err(e) => {
                                tracing::warn!("render failed: {e:?}");
                                None
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("bind failed: {e:?}");
                        None
                    }
                };
                if let Some(rects) = submitted {
                    if let Err(e) = backend.submit(Some(&rects)) {
                        tracing::warn!("submit failed: {e:?}");
                    }
                }
                render::send_frames(desk);
                desk.space.refresh();
                desk.popups.cleanup();
                let _ = desk.display_handle.flush_clients();
            }
            WinitEvent::CloseRequested => desk.loop_signal.stop(),
            _ => {}
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(Box::new(move |desk: &mut Desk| {
        if desk.needs_redraw {
            backend.borrow().window().request_redraw();
        }
    }))
}
