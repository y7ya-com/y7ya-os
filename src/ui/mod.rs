//! A minimal GPU UI framework in the spirit of GPUI: winit for windows,
//! wgpu for drawing, cosmic-text for text and taffy for flexbox layout.

mod app;
mod cx;
mod element;
mod geom;
mod input;
mod platform;
mod renderer;
mod scene;
mod text;

pub use app::{App, WindowOptions, run, snapshot};
pub use cx::Cx;
pub use element::{AnyElement, Div, ElementId, Styled, canvas, div, eid, input, text};
pub use geom::{Bounds, Color, Point, pt, rgb, rgba};
pub use input::{KeyPress, TextInput};
pub use platform::{WindowButton, window_button_layout};
pub use scene::Scene;
pub use text::{TextStyle, Weight};
pub use winit::keyboard::{Key, NamedKey};
pub use winit::window::CursorIcon;
