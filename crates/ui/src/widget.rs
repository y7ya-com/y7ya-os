//! Custom interactive leaf elements. Like `TextInput`, a widget's state is
//! owned by the app (behind `Rc<RefCell<..>>`), but it paints itself and gets
//! focus, keys, mouse drags and the scroll wheel directly. Its size comes from
//! layout alone (use `flex_1`, `w`/`h` and friends).

use std::cell::RefCell;
use std::hash::Hash;
use std::rc::Rc;
use std::time::Instant;

use taffy::Style;
use winit::window::CursorIcon;

use super::cx::Cx;
use super::element::{AnyElement, ElementId, Styled, eid};
use super::geom::{Bounds, Point};
use super::input::KeyPress;
use super::scene::Scene;

pub trait Widget {
    /// Paints into `bounds`; returns the caret rectangle (for IME placement).
    fn paint(&mut self, cx: &mut Cx, scene: &mut Scene, bounds: Bounds, focused: bool) -> Option<Bounds>;
    /// A key while focused (after `App::on_key`). Return true if the content changed.
    fn key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool;
    /// `local` is relative to the widget's top-left corner.
    fn mouse_down(&mut self, _local: Point, _clicks: u32, _shift: bool, _cx: &mut Cx) {}
    fn mouse_drag(&mut self, _local: Point, _cx: &mut Cx) {}
    /// Wheel scrolling by `dy` points (positive scrolls down). Return true if handled.
    fn scroll(&mut self, _dy: f32, _cx: &mut Cx) -> bool {
        false
    }
    /// Last keyboard or mouse activity, which restarts the caret blink.
    fn last_activity(&self) -> Option<Instant> {
        None
    }
}

pub struct WidgetEl {
    pub(crate) id: ElementId,
    pub(crate) state: Rc<RefCell<dyn Widget>>,
    pub(crate) cursor: Option<CursorIcon>,
    pub(crate) layout: Style,
}

pub fn widget<W: Widget + 'static>(id: impl Hash, state: &Rc<RefCell<W>>) -> WidgetEl {
    WidgetEl { id: eid(id), state: state.clone(), cursor: None, layout: Style::default() }
}

impl WidgetEl {
    pub fn cursor(mut self, c: CursorIcon) -> Self {
        self.cursor = Some(c);
        self
    }
}

impl Styled for WidgetEl {
    fn layout_mut(&mut self) -> &mut Style {
        &mut self.layout
    }
}

impl<S> From<WidgetEl> for AnyElement<S> {
    fn from(w: WidgetEl) -> Self {
        AnyElement::Widget(w)
    }
}
