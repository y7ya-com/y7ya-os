//! A minimal GPU UI framework in the spirit of GPUI: winit for windows,
//! wgpu for drawing, cosmic-text for text and taffy for flexbox layout.

mod app;
pub mod chrome;
mod cx;
mod element;
mod geom;
mod input;
mod platform;
mod renderer;
mod scene;
mod scroll;
mod text;
mod widget;
pub mod widgets;

pub use app::{App, WindowOptions, run, snapshot};
pub use cx::{Cx, Waker};
pub use element::{AnyElement, Div, ElementId, Styled, canvas, div, eid, input, text};
pub use geom::{Bounds, Color, Point, pt, rgb, rgba};
pub use input::{KeyPress, TextInput};
pub use platform::{WindowButton, is_y7ya, window_button_layout};
pub use scene::Scene;
pub use text::{TextStyle, Weight};
pub use widget::{Widget, WidgetEl, widget};
pub use widgets::image::{Fit, Image, ImageStyle, fit_rect, image};
pub use widgets::text_view::TextView;
pub use winit::keyboard::{Key, ModifiersState, NamedKey};
pub use winit::window::CursorIcon;
