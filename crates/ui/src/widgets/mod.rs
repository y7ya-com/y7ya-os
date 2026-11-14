//! Ready-made widgets, one per file: some built on [`Widget`](crate::Widget),
//! some from the core elements.

pub mod graph;
pub mod image;
mod spinner;
pub mod table;
pub mod text_view;

pub use spinner::{SPINNER_FRAME, spinner, spinner_phase};
