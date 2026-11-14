//! Fading a whole subtree, for panels that fade in and out: everything a div
//! paints (its own background, shadow, text, images, children) has its alpha
//! scaled. Deferred children paint on their own layers and aren't affected.

use crate::element::Div;

impl<S> Div<S> {
    /// 0 is invisible (it still takes clicks), 1 is unchanged.
    pub fn opacity(mut self, v: f32) -> Self {
        self.opacity = v.clamp(0.0, 1.0);
        self
    }
}
