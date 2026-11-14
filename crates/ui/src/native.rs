//! Shared macOS metrics and system colors (see `docs/design.md`), so every
//! app's toolbar, sidebar, selection and focus ring come out the same.

use crate::cx::Cx;
use crate::element::{Styled, Text, text};
use crate::geom::{Color, rgb, rgba};
use crate::text::Weight;

/// Unified toolbar height.
pub const TOOLBAR_H: f32 = 52.0;
/// Traffic lights start this far from the window's left edge.
pub const TRAFFIC_LIGHTS_X: f32 = 20.0;
pub const SIDEBAR_W: f32 = 220.0;
/// Sidebar rows: height, inset of the selection from the sidebar's edges,
/// padding inside the row, icon size and the selection's corner radius.
pub const SIDEBAR_ROW_H: f32 = 28.0;
pub const SIDEBAR_INSET: f32 = 10.0;
pub const SIDEBAR_ROW_PAD: f32 = 8.0;
pub const SIDEBAR_ICON: f32 = 16.0;
pub const SIDEBAR_RADIUS: f32 = 6.0;
/// Width of a keyboard focus ring, drawn outside the control.
pub const FOCUS_RING_W: f32 = 3.0;

/// System blue.
pub fn accent(cx: &Cx) -> Color {
    if cx.dark { rgb(0x0a84ff) } else { rgb(0x007aff) }
}

/// Keyboard focus ring around fields and controls.
pub fn focus_ring(cx: &Cx) -> Color {
    accent(cx).alpha(if cx.dark { 0.55 } else { 0.45 })
}

/// A selected row: `accent` while its list has keyboard focus in the active
/// window, gray otherwise (as macOS dims selections in inactive windows).
pub fn selection(cx: &Cx, list_focused: bool) -> Color {
    if list_focused && cx.window_focused { accent(cx) } else { unemphasized_selection(cx) }
}

/// Text on a `selection` background.
pub fn selection_text(cx: &Cx, list_focused: bool, normal: Color) -> Color {
    if list_focused && cx.window_focused { Color::WHITE } else { normal }
}

pub fn unemphasized_selection(cx: &Cx) -> Color {
    if cx.dark { rgba(0xffffff, 0.10) } else { rgba(0x000000, 0.08) }
}

/// The gray pill behind the selected sidebar row; lighter in inactive windows.
pub fn sidebar_selection(cx: &Cx) -> Color {
    match (cx.dark, cx.window_focused) {
        (false, true) => rgba(0x000000, 0.09),
        (false, false) => rgba(0x000000, 0.05),
        (true, true) => rgba(0xffffff, 0.10),
        (true, false) => rgba(0xffffff, 0.06),
    }
}

/// Text and icons in toolbars: lighter while the window is inactive.
pub fn toolbar_ink(cx: &Cx, ink: Color) -> Color {
    if cx.window_focused { ink } else { ink.alpha(ink.a * 0.55) }
}

pub fn secondary_label(cx: &Cx) -> Color {
    if cx.dark { rgba(0xffffff, 0.55) } else { rgba(0x000000, 0.50) }
}

pub fn tertiary_label(cx: &Cx) -> Color {
    if cx.dark { rgba(0xffffff, 0.25) } else { rgba(0x000000, 0.26) }
}

/// A sidebar section header ("Favorites", "On My Computer"): 11pt semibold,
/// secondary color, lined up with the row icons below it.
pub fn sidebar_header(label: impl Into<String>, cx: &Cx) -> Text {
    text(label)
        .size(11.0)
        .weight(Weight::Semibold)
        .color(secondary_label(cx))
        .truncate()
        .pl(SIDEBAR_INSET + SIDEBAR_ROW_PAD)
        .pt(14.0)
        .pb(5.0)
}
