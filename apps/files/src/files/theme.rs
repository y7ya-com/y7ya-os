use ui::{Color, rgb, rgba};

/// Finder's colors in light and dark appearance (see docs/design.md).
#[derive(Clone, Copy)]
pub struct Theme {
    pub sidebar: Color,
    pub content: Color,
    pub divider: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub accent: Color,
    /// Behind a selected icon in icon view.
    pub icon_selection: Color,
    pub stripe: Color,
    pub hover: Color,
    pub icon: Color,
    pub icon_disabled: Color,
    pub icon_hover_bg: Color,
    pub icon_pressed_bg: Color,
    pub field: Color,
    pub segment_selected: Color,
    pub text_selection: Color,
    pub menu_border: Color,
    pub window_border: Color,
    pub shadow: Color,
    pub destructive: Color,
    pub panel: Color,
    pub preview_bg: Color,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            sidebar: rgb(0xf2f2f4),
            content: rgb(0xffffff),
            divider: rgba(0x000000, 0.10),
            text: rgba(0x000000, 0.85),
            text_secondary: rgba(0x000000, 0.50),
            text_tertiary: rgba(0x000000, 0.26),
            accent: rgb(0x007aff),
            icon_selection: rgba(0x000000, 0.08),
            stripe: rgba(0x000000, 0.03),
            hover: rgba(0x000000, 0.05),
            icon: rgba(0x000000, 0.62),
            icon_disabled: rgba(0x000000, 0.22),
            icon_hover_bg: rgba(0x000000, 0.06),
            icon_pressed_bg: rgba(0x000000, 0.11),
            field: rgba(0x000000, 0.05),
            segment_selected: rgb(0xffffff),
            text_selection: rgba(0x007aff, 0.25),
            menu_border: rgba(0x000000, 0.12),
            window_border: rgba(0x000000, 0.22),
            shadow: rgba(0x000000, 0.18),
            destructive: rgb(0xff3b30),
            panel: rgb(0xffffff),
            preview_bg: rgb(0xf5f5f7),
        }
    }

    pub fn dark() -> Self {
        Self {
            sidebar: rgb(0x2a2a2c),
            content: rgb(0x1e1e1e),
            divider: rgba(0xffffff, 0.10),
            text: rgba(0xffffff, 0.85),
            text_secondary: rgba(0xffffff, 0.55),
            text_tertiary: rgba(0xffffff, 0.25),
            accent: rgb(0x0a84ff),
            icon_selection: rgba(0xffffff, 0.12),
            stripe: rgba(0xffffff, 0.035),
            hover: rgba(0xffffff, 0.06),
            icon: rgba(0xffffff, 0.66),
            icon_disabled: rgba(0xffffff, 0.22),
            icon_hover_bg: rgba(0xffffff, 0.08),
            icon_pressed_bg: rgba(0xffffff, 0.14),
            field: rgba(0xffffff, 0.08),
            segment_selected: rgba(0xffffff, 0.20),
            text_selection: rgba(0x0a84ff, 0.45),
            menu_border: rgba(0xffffff, 0.14),
            window_border: rgba(0xffffff, 0.14),
            shadow: rgba(0x000000, 0.45),
            destructive: rgb(0xff453a),
            panel: rgb(0x2c2c2e),
            preview_bg: rgb(0x262628),
        }
    }
}
