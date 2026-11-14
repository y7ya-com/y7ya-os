use ui::{Color, rgb, rgba};

/// Colors for macOS Reminders in light and dark appearance.
#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub sidebar: Color,
    pub content: Color,
    pub divider: Color,
    pub row_divider: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub accent: Color,
    pub destructive: Color,
    pub flag: Color,
    /// Smart-list tiles when not selected.
    pub tile: Color,
    pub row_selection: Color,
    pub hover: Color,
    pub icon: Color,
    pub icon_hover_bg: Color,
    pub icon_pressed_bg: Color,
    pub field: Color,
    pub text_selection: Color,
    /// The empty completion circle.
    pub ring: Color,
    pub menu_border: Color,
    pub popover: Color,
    pub window_border: Color,
    pub shadow: Color,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            dark: false,
            sidebar: rgb(0xebeaee),
            content: rgb(0xffffff),
            divider: rgba(0x000000, 0.10),
            row_divider: rgba(0x000000, 0.08),
            text: rgba(0x000000, 0.85),
            text_secondary: rgba(0x000000, 0.50),
            text_tertiary: rgba(0x000000, 0.26),
            accent: rgb(0x007aff),
            destructive: rgb(0xff3b30),
            flag: rgb(0xff9500),
            tile: rgba(0xffffff, 0.92),
            row_selection: rgba(0x000000, 0.055),
            hover: rgba(0x000000, 0.05),
            icon: rgba(0x000000, 0.55),
            icon_hover_bg: rgba(0x000000, 0.06),
            icon_pressed_bg: rgba(0x000000, 0.11),
            field: rgba(0x000000, 0.05),
            text_selection: rgba(0x007aff, 0.25),
            ring: rgba(0x000000, 0.26),
            menu_border: rgba(0x000000, 0.12),
            popover: rgb(0xf6f6f6),
            window_border: rgba(0x000000, 0.22),
            shadow: rgba(0x000000, 0.18),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            sidebar: rgb(0x2a2a2c),
            content: rgb(0x1e1e1e),
            divider: rgb(0x0b0b0b),
            row_divider: rgba(0xffffff, 0.08),
            text: rgba(0xffffff, 0.85),
            text_secondary: rgba(0xffffff, 0.55),
            text_tertiary: rgba(0xffffff, 0.25),
            accent: rgb(0x0a84ff),
            destructive: rgb(0xff453a),
            flag: rgb(0xff9f0a),
            tile: rgba(0xffffff, 0.08),
            row_selection: rgba(0xffffff, 0.07),
            hover: rgba(0xffffff, 0.05),
            icon: rgba(0xffffff, 0.58),
            icon_hover_bg: rgba(0xffffff, 0.08),
            icon_pressed_bg: rgba(0xffffff, 0.14),
            field: rgba(0xffffff, 0.08),
            text_selection: rgba(0x0a84ff, 0.45),
            ring: rgba(0xffffff, 0.30),
            menu_border: rgba(0xffffff, 0.14),
            popover: rgb(0x2c2c2e),
            window_border: rgba(0xffffff, 0.14),
            shadow: rgba(0x000000, 0.45),
        }
    }
}
