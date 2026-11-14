//! Protocols for the rest of the system: layer surfaces (the menu bar,
//! Spotlight, notifications draw above or below windows), clipboard managers,
//! and GPU buffer sharing so apps don't copy frames through the CPU.

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::egl::EGLDevice;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::{LayerSurface, WindowSurfaceType, layer_map_for_output};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface};
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::with_states;
use smithay::wayland::dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::selection::wlr_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::fractional_scale::{FractionalScaleHandler, with_fractional_scale};
use smithay::wayland::xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData};
use smithay::{
    delegate_data_control, delegate_dmabuf, delegate_fractional_scale, delegate_layer_shell, delegate_xdg_activation,
};

use crate::state::Desk;

impl WlrLayerShellHandler for Desk {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, output: Option<WlOutput>, _layer: Layer, namespace: String) {
        let Some(output) = output.as_ref().and_then(Output::from_resource).or_else(|| self.output.clone()) else { return };
        let mut map = layer_map_for_output(&output);
        if let Err(e) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            tracing::warn!("couldn't map layer surface: {e}");
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let Some(output) = self.output.clone() else { return };
        let mut map = layer_map_for_output(&output);
        let layer = map.layers().find(|l| l.layer_surface() == &surface).cloned();
        if let Some(layer) = layer {
            map.unmap_layer(&layer);
        }
        drop(map);
        if self.layer_focus.as_ref().is_some_and(|s| s == surface.wl_surface()) {
            self.layer_focus = None;
            self.focus_top();
        }
        self.redraw();
    }
}

impl Desk {
    /// Handles a commit if it belongs to a layer surface. Returns true if so.
    pub fn commit_layer(&mut self, surface: &WlSurface) -> bool {
        let Some(output) = self.output.clone() else { return false };
        let mut map = layer_map_for_output(&output);
        let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).cloned() else { return false };
        let initial_sent = with_states(surface, |states| {
            states.data_map.get::<LayerSurfaceData>().unwrap().lock().unwrap().initial_configure_sent
        });
        map.arrange();
        if !initial_sent {
            layer.layer_surface().send_configure();
        }
        drop(map);
        // Panels like Spotlight ask for the keyboard while they're up.
        let wants_keys = layer.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive;
        if wants_keys && initial_sent && self.layer_focus.as_ref() != Some(surface) {
            self.layer_focus = Some(surface.clone());
            let keyboard = self.seat.get_keyboard().unwrap();
            keyboard.set_focus(self, Some(surface.clone()), smithay::utils::SERIAL_COUNTER.next_serial());
        }
        self.redraw();
        true
    }

    /// The part of the screen windows may use (minus the menu bar and such).
    pub fn usable_area(&self) -> Rectangle<i32, Logical> {
        let out = self.output_geometry();
        let Some(output) = &self.output else { return out };
        let mut zone = layer_map_for_output(output).non_exclusive_zone();
        zone.loc += out.loc;
        zone
    }

    /// A layer surface on `layers` under `pos`, with its location.
    pub fn layer_under(&self, layers: &[Layer], pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.output.as_ref()?;
        let map = layer_map_for_output(output);
        let out = self.output_geometry();
        for &kind in layers {
            if let Some(layer) = map.layer_under(kind, pos - out.loc.to_f64()) {
                let geo = map.layer_geometry(layer)?;
                let loc = (geo.loc + out.loc).to_f64();
                if let Some((s, p)) = layer.surface_under(pos - loc, WindowSurfaceType::ALL) {
                    return Some((s, p.to_f64() + loc));
                }
            }
        }
        None
    }

    /// Sets up GPU buffer sharing with the renderer's formats.
    pub fn init_dmabuf(&mut self, renderer: &GlesRenderer) {
        let formats = renderer.dmabuf_formats();
        let node = EGLDevice::device_for_display(renderer.egl_context().display())
            .ok()
            .and_then(|d| d.try_get_render_node().ok().flatten());
        let mut state = DmabufState::new();
        let global = match node.and_then(|n| DmabufFeedbackBuilder::new(n.dev_id(), formats.clone()).build().ok()) {
            Some(feedback) => state.create_global_with_default_feedback::<Desk>(&self.display_handle, &feedback),
            None => state.create_global::<Desk>(&self.display_handle, formats),
        };
        self.dmabuf = Some((state, global));
    }
}

impl DmabufHandler for Desk {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf.as_mut().expect("dmabuf initialized by the backend").0
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, _dmabuf: Dmabuf, notifier: ImportNotifier) {
        // The renderer imports buffers when it first draws them.
        let _ = notifier.successful::<Desk>();
    }
}

impl DataControlHandler for Desk {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
    }
}

delegate_layer_shell!(Desk);
delegate_dmabuf!(Desk);
delegate_data_control!(Desk);

impl FractionalScaleHandler for Desk {}

impl XdgActivationHandler for Desk {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
    }

    /// An app asks to come forward (e.g. a link opened in a running browser).
    /// Allow it when the request is recent, like macOS does for user actions.
    fn request_activation(&mut self, token: XdgActivationToken, data: XdgActivationTokenData, surface: WlSurface) {
        if data.timestamp.elapsed() < std::time::Duration::from_secs(10) {
            if let Some(window) = self.window_for_surface(&surface) {
                self.focus(Some(window));
            }
        }
        self.activation_state.remove_token(&token);
    }
}

impl Desk {
    /// Tells a surface the output's (possibly fractional) scale so it can
    /// render crisply instead of being resampled.
    pub fn send_preferred_scale(&self, surface: &WlSurface) {
        let Some(output) = &self.output else { return };
        let scale = output.current_scale().fractional_scale();
        with_states(surface, |states| with_fractional_scale(states, |fs| fs.set_preferred_scale(scale)));
    }
}

delegate_fractional_scale!(Desk);
delegate_xdg_activation!(Desk);
