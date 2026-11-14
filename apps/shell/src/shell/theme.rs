use ui::{Color, rgb, rgba};

/// docs/design.md's tokens plus the shell's own surfaces. No blur anywhere:
/// the taskbar is lightly translucent; flyouts are solid (without blur,
/// translucency just shows windows ghosting through).
#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub label: Color,
    pub secondary: Color,
    pub tertiary: Color,
    pub separator: Color,
    pub fill: Color,
    pub accent: Color,
    pub destructive: Color,
    pub success: Color,
    /// The taskbar and its hairline top edge.
    pub bar: Color,
    pub bar_edge: Color,
    /// Taskbar buttons: hovered, pressed, and the active window's pill.
    pub bar_hover: Color,
    pub bar_press: Color,
    pub bar_active: Color,
    /// A running window's quiet underline (the active one's is accent).
    pub bar_running: Color,
    pub menu: Color,
    pub border: Color,
    pub shadow: Color,
    /// Flyouts (Start, Quick Settings, the calendar) and the modules in them.
    pub panel: Color,
    pub module: Color,
    /// Banners, Spotlight's field and results.
    pub card: Color,
    pub hover: Color,
}

impl Theme {
    pub fn new(dark: bool) -> Theme {
        if dark {
            Theme {
                dark,
                label: rgba(0xffffff, 0.85),
                secondary: rgba(0xffffff, 0.55),
                tertiary: rgba(0xffffff, 0.25),
                separator: rgba(0xffffff, 0.10),
                fill: rgba(0xffffff, 0.08),
                accent: rgb(0x0a84ff),
                destructive: rgb(0xff453a),
                success: rgb(0x30d158),
                bar: rgba(0x1e1e20, 0.90),
                bar_edge: rgba(0xffffff, 0.10),
                bar_hover: rgba(0xffffff, 0.07),
                bar_press: rgba(0xffffff, 0.04),
                bar_active: rgba(0xffffff, 0.13),
                bar_running: rgba(0xffffff, 0.32),
                menu: rgb(0x2c2c2e),
                border: rgba(0xffffff, 0.14),
                shadow: rgba(0x000000, 0.45),
                panel: rgb(0x252527),
                module: rgba(0xffffff, 0.06),
                card: rgb(0x2a2a2c),
                hover: rgba(0xffffff, 0.08),
            }
        } else {
            Theme {
                dark,
                label: rgba(0x000000, 0.85),
                secondary: rgba(0x000000, 0.50),
                tertiary: rgba(0x000000, 0.26),
                separator: rgba(0x000000, 0.10),
                fill: rgba(0x000000, 0.05),
                accent: rgb(0x007aff),
                destructive: rgb(0xff3b30),
                success: rgb(0x34c759),
                bar: rgba(0xf2f2f4, 0.90),
                bar_edge: rgba(0x000000, 0.12),
                bar_hover: rgba(0x000000, 0.05),
                bar_press: rgba(0x000000, 0.08),
                bar_active: rgba(0xffffff, 0.95),
                bar_running: rgba(0x000000, 0.28),
                menu: rgb(0xf6f6f6),
                border: rgba(0x000000, 0.12),
                shadow: rgba(0x000000, 0.22),
                panel: rgb(0xf4f4f6),
                module: rgba(0x000000, 0.045),
                card: rgb(0xf7f7f8),
                hover: rgba(0x000000, 0.05),
            }
        }
    }
}
