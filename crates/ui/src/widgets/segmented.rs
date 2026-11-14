//! A macOS segmented control (Calendar's Day | Week | Month | Year).

use std::rc::Rc;

use crate::cx::Cx;
use crate::element::{Div, Styled, div, text};
use crate::geom::{Color, rgba};
use crate::text::Weight;

/// Segments `labels` with `selected` raised; clicking one calls `on_select`
/// with its index. 24pt tall, each segment sized to fit its label.
pub fn segmented<S: 'static>(
    cx: &Cx,
    id: &'static str,
    labels: &[&str],
    selected: usize,
    on_select: impl Fn(&mut S, usize, &mut Cx) + 'static,
) -> Div<S> {
    let (track, knob, ink, divider) = if cx.dark {
        (rgba(0xffffff, 0.08), rgba(0xffffff, 0.24), rgba(0xffffff, 0.85), rgba(0xffffff, 0.12))
    } else {
        (rgba(0x000000, 0.06), Color::WHITE, rgba(0x000000, 0.85), rgba(0x000000, 0.12))
    };
    let on_select = Rc::new(on_select);
    let mut row = div().h(24.0).p(2.0).rounded(7.0).bg(track).flex_row().items_center();
    for (i, label) in labels.iter().enumerate() {
        // Hairlines between unselected neighbours, like AppKit.
        if i > 0 {
            let show = i != selected && i - 1 != selected;
            row = row.child(div().w(1.0).h(12.0).bg(if show { divider } else { Color::TRANSPARENT }));
        }
        let on_select = on_select.clone();
        let mut seg = div()
            .id((id, i))
            .h(20.0)
            .px(11.0)
            .rounded(5.0)
            .items_center()
            .justify_center()
            .on_mouse_down(move |s: &mut S, cx: &mut Cx| on_select(s, i, cx))
            .child(text(label.to_string()).size(12.5).weight(if i == selected { Weight::Medium } else { Weight::Regular }).color(ink));
        if i == selected {
            seg = seg.bg(knob).shadow(rgba(0x000000, if cx.dark { 0.3 } else { 0.12 }), 2.0, 0.5);
        }
        row = row.child(seg);
    }
    row
}
