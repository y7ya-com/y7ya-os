//! Programmatic scrolling beyond `scroll_to_top`.

use std::hash::Hash;

use super::cx::Cx;
use super::element::eid;

impl Cx {
    /// Scrolls the `scroll_y` container `id` to its end on the next frame
    /// (the offset is clamped to the content height while painting). Chats
    /// call this when new messages arrive.
    pub fn scroll_to_bottom(&mut self, id: impl Hash) {
        self.set_scroll_offset(eid(id), f32::MAX);
        self.request_redraw();
    }
}
