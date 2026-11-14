//! Layer-shell surfaces: menu bars, panels and overlays that the desktop
//! stacks above or below windows (wlr-layer-shell). winit can't create them,
//! so `run_layer` is a second runner built on smithay-client-toolkit. It
//! drives the same [`App`] trait, elements, renderer and input code as `run`.
//!
//! One app can own several surfaces. Each has a namespace; `render` is called
//! once per surface with `cx.layer()` naming the one being drawn, and the app
//! shows, hides and resizes surfaces with `cx.show_layer` / `cx.hide_layer`.

#[cfg(target_os = "linux")]
mod keys;
#[cfg(target_os = "linux")]
mod wayland;

use std::collections::HashSet;
use std::ops::BitOr;

#[cfg(target_os = "linux")]
pub use wayland::run_layer;

use crate::app::App;
use crate::cx::Cx;

/// Where a layer surface sits relative to windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layer {
    Background,
    Bottom,
    #[default]
    Top,
    Overlay,
}

/// Screen edges a layer surface is attached to. Anchored to both opposite
/// edges with a size of 0 on that axis, it stretches across the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Anchor(u8);

impl Anchor {
    pub const NONE: Anchor = Anchor(0);
    pub const TOP: Anchor = Anchor(1);
    pub const BOTTOM: Anchor = Anchor(2);
    pub const LEFT: Anchor = Anchor(4);
    pub const RIGHT: Anchor = Anchor(8);
    pub const ALL: Anchor = Anchor(15);

    pub fn contains(self, other: Anchor) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Anchor {
    type Output = Anchor;
    fn bitor(self, o: Anchor) -> Anchor {
        Anchor(self.0 | o.0)
    }
}

/// Whether the surface takes keyboard input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardInteractivity {
    #[default]
    None,
    /// Grabs the keyboard while shown (Spotlight).
    Exclusive,
    /// Gets the keyboard when clicked.
    OnDemand,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerOptions {
    pub layer: Layer,
    pub anchor: Anchor,
    /// Logical size; 0 on an axis anchored at both ends means "stretch".
    pub size: (u32, u32),
    /// Space reserved at the anchored edge (windows stay out of it); 0 to
    /// respect other surfaces' zones, -1 to cover them.
    pub exclusive_zone: i32,
    pub keyboard_interactivity: KeyboardInteractivity,
    /// Names the surface; also the key for `show_layer` / `hide_layer`.
    pub namespace: String,
    /// Top, right, bottom, left.
    pub margin: (i32, i32, i32, i32),
}

pub(crate) enum LayerCommand {
    Show(LayerOptions),
    Hide(String),
}

/// Layer state kept in `Cx`.
#[derive(Default)]
pub(crate) struct LayerCx {
    /// The surface being rendered or receiving input.
    pub(crate) current: String,
    pub(crate) commands: Vec<LayerCommand>,
    pub(crate) shown: HashSet<String>,
}

impl Cx {
    /// The namespace of the layer surface being rendered or handling input
    /// (empty in a normal window).
    pub fn layer(&self) -> &str {
        &self.layers.current
    }

    /// Shows a layer surface, or updates it (size, margins, keyboard...) if one
    /// with this namespace is already up.
    pub fn show_layer(&mut self, opts: LayerOptions) {
        self.layers.shown.insert(opts.namespace.clone());
        self.layers.commands.push(LayerCommand::Show(opts));
        self.redraw = true;
    }

    /// Destroys the surface, freeing its buffers until it's shown again.
    pub fn hide_layer(&mut self, namespace: &str) {
        if self.layers.shown.remove(namespace) {
            self.layers.commands.push(LayerCommand::Hide(namespace.to_string()));
        }
    }

    pub fn layer_shown(&self, namespace: &str) -> bool {
        self.layers.shown.contains(namespace)
    }
}

/// Like [`crate::snapshot`], rendering the layer surface `namespace` of a
/// multi-surface app.
pub fn snapshot_layer<A: App>(
    namespace: &str,
    size: (f32, f32),
    scale: f32,
    dark: bool,
    path: &std::path::Path,
    make: impl FnOnce(&mut Cx) -> A,
    setup: impl FnOnce(&mut A, &mut Cx),
) {
    crate::app::snapshot(size, scale, dark, path, make, |app, cx| {
        cx.layers.current = namespace.to_string();
        setup(app, cx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_and_bookkeeping() {
        let top_bar = Anchor::TOP | Anchor::LEFT | Anchor::RIGHT;
        assert!(top_bar.contains(Anchor::TOP | Anchor::RIGHT));
        assert!(!top_bar.contains(Anchor::BOTTOM));
        assert!(Anchor::ALL.contains(top_bar));

        let mut cx = Cx::new();
        let opts = LayerOptions { namespace: "panel".into(), ..Default::default() };
        cx.show_layer(opts.clone());
        assert!(cx.layer_shown("panel"));
        cx.hide_layer("panel");
        cx.hide_layer("panel");
        assert!(!cx.layer_shown("panel"));
        // One show and one hide: hiding twice doesn't queue twice.
        assert!(matches!(cx.layers.commands.as_slice(), [LayerCommand::Show(o), LayerCommand::Hide(n)] if *o == opts && n == "panel"));
    }
}

/// The desktop's light/dark preference: `Y7_THEME`, then KDE's color scheme.
pub fn system_dark() -> bool {
    if let Ok(theme) = std::env::var("Y7_THEME") {
        return theme == "dark";
    }
    let path = dirs::config_dir().map(|d| d.join("kdeglobals"));
    let Some(s) = path.and_then(|p| std::fs::read_to_string(p).ok()) else { return false };
    s.lines().find_map(|l| l.strip_prefix("ColorScheme=")).is_some_and(|v| v.to_lowercase().contains("dark"))
}
