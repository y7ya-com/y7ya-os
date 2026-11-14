//! A macOS popover: a panel beside something on screen, with an arrow
//! pointing at it. Put it inside an `inset_0().deferred()` overlay.

use crate::cx::Cx;
use crate::element::{Div, Styled, canvas, div};
use crate::geom::{Bounds, Color, pt};

const ARROW: f32 = 10.0;
const MARGIN: f32 = 8.0;

/// `content` in a panel `w` wide (about `est_h` tall) next to `anchor`
/// (window coordinates): on its right if there's room, else on its left,
/// kept inside the window. The panel is drawn opaque, so busy content
/// underneath never shows through.
#[allow(clippy::too_many_arguments)]
pub fn popover<S: 'static>(cx: &Cx, anchor: Bounds, w: f32, est_h: f32, bg: Color, border: Color, shadow: Color, content: Div<S>) -> Div<S> {
    let (win_w, win_h) = cx.window_size;
    let right = anchor.right() + ARROW + w + MARGIN <= win_w || anchor.x - ARROW - w < MARGIN;
    let x = if right { anchor.right() + ARROW } else { anchor.x - ARROW - w };
    let x = x.clamp(MARGIN, (win_w - w - MARGIN).max(MARGIN));
    let target = anchor.y + anchor.h.min(60.0) / 2.0;
    let y = (target - 40.0).clamp(MARGIN, (win_h - est_h - MARGIN).max(MARGIN));
    // The arrow stays on the panel's rounded edge.
    let ay = (target - y).clamp(18.0, est_h - 18.0);
    let solid = Color { a: 1.0, ..bg };
    let arrow = canvas(move |s, b| {
        let cy = b.y + b.h / 2.0;
        let steps = (ARROW * 4.0) as i32;
        for i in 0..=steps {
            // Distance from the tip, so the triangle widens toward the panel.
            let d = i as f32 / 4.0;
            let x = if right { b.x + d } else { b.right() - d };
            let half = d.min(ARROW);
            s.segment(pt(x, cy - half), pt(x, cy + half), 0.5, solid);
        }
        let tip = if right { pt(b.x, cy) } else { pt(b.right(), cy) };
        let base = if right { b.x + ARROW } else { b.right() - ARROW };
        s.segment(tip, pt(base, cy - ARROW), 0.6, border);
        s.segment(tip, pt(base, cy + ARROW), 0.6, border);
    })
    .absolute(if right { -ARROW } else { w - 1.0 }, ay - ARROW)
    .w(ARROW + 1.0)
    .h(ARROW * 2.0);
    div()
        .absolute(x, y)
        .w(w)
        .rounded(10.0)
        .bg(solid)
        .border(border, 0.5)
        .shadow(shadow, 24.0, 8.0)
        .occlude()
        .child(content)
        .child(arrow)
}
