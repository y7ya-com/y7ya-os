use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

use super::element::{ElementId, eid};
use super::geom::{Bounds, Point};
use super::input::TextInput;
use super::text::TextSystem;

pub const CARET_BLINK_MS: u128 = 530;

pub(crate) enum WindowCommand {
    Close,
    Minimize,
    ToggleMaximize,
}

/// Window-wide state shared with the app during render and event handling.
pub struct Cx {
    pub text: TextSystem,
    /// Logical size of the window.
    pub window_size: (f32, f32),
    pub scale: f32,
    pub dark: bool,
    pub maximized: bool,
    /// True when the window can have transparent (rounded) corners.
    pub transparent: bool,
    /// Rounded window corners (only applied when `transparent`).
    pub window_radius: f32,
    pub mouse: Point,
    pub focused: Option<ElementId>,
    pub clipboard: Option<arboard::Clipboard>,
    pub(crate) hovered: Vec<ElementId>,
    pub(crate) ime_area: Option<Bounds>,
    pub(crate) commands: Vec<WindowCommand>,
    pub(crate) redraw: bool,
    scroll: HashMap<ElementId, f32>,
}

impl Cx {
    pub(crate) fn new() -> Self {
        Self {
            text: TextSystem::new(),
            window_size: (0.0, 0.0),
            scale: 1.0,
            dark: false,
            maximized: false,
            transparent: false,
            window_radius: 0.0,
            mouse: Point { x: -1.0, y: -1.0 },
            focused: None,
            clipboard: arboard::Clipboard::new().ok(),
            hovered: Vec::new(),
            ime_area: None,
            commands: Vec::new(),
            redraw: false,
            scroll: HashMap::new(),
        }
    }

    pub fn focus(&mut self, id: impl Hash) {
        self.focused = Some(eid(id));
        self.redraw = true;
    }

    pub fn blur(&mut self) {
        self.focused = None;
        self.redraw = true;
    }

    pub fn is_focused(&self, id: impl Hash) -> bool {
        self.focused == Some(eid(id))
    }

    pub fn is_hovered(&self, id: ElementId) -> bool {
        self.hovered.contains(&id)
    }

    pub fn hovered(&self, id: impl Hash) -> bool {
        self.is_hovered(eid(id))
    }

    pub(crate) fn scroll_offset(&self, id: ElementId) -> f32 {
        self.scroll.get(&id).copied().unwrap_or(0.0)
    }

    pub(crate) fn set_scroll_offset(&mut self, id: ElementId, v: f32) {
        self.scroll.insert(id, v.max(0.0));
    }

    pub(crate) fn clamp_scroll(&mut self, id: ElementId, max: f32) -> f32 {
        let v = self.scroll_offset(id).clamp(0.0, max);
        self.scroll.insert(id, v);
        v
    }

    pub fn scroll_to_top(&mut self, id: impl Hash) {
        self.scroll.insert(eid(id), 0.0);
    }

    pub fn caret_visible(&self, input: &TextInput) -> bool {
        (Instant::now() - input.last_activity).as_millis() / CARET_BLINK_MS % 2 == 0
    }

    pub(crate) fn effective_window_radius(&self) -> f32 {
        if self.transparent && !self.maximized { self.window_radius } else { 0.0 }
    }

    pub fn request_redraw(&mut self) {
        self.redraw = true;
    }

    pub fn close_window(&mut self) {
        self.commands.push(WindowCommand::Close);
    }

    pub fn minimize_window(&mut self) {
        self.commands.push(WindowCommand::Minimize);
    }

    pub fn toggle_maximize(&mut self) {
        self.commands.push(WindowCommand::ToggleMaximize);
    }
}
