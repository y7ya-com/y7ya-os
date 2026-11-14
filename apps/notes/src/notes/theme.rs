use ui::{Color, rgb, rgba};

/// Colors sampled to match macOS Notes in light and dark appearance.
#[derive(Clone, Copy)]
pub struct Theme {
    pub sidebar: Color,
    pub content: Color,
    pub divider: Color,
    pub row_divider: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub accent: Color,
    pub selection_focused: Color,
    pub selection_text_focused: Color,
    pub selection_unfocused: Color,
    pub hover: Color,
    pub icon: Color,
    pub icon_hover_bg: Color,
    pub icon_pressed_bg: Color,
    pub field: Color,
    pub text_selection: Color,
    pub caret: Color,
    pub menu_border: Color,
    pub window_border: Color,
    pub shadow: Color,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            sidebar: rgb(0xe9e7e6),
            content: rgb(0xffffff),
            divider: rgba(0x000000, 0.10),
            row_divider: rgba(0x000000, 0.08),
            text: rgba(0x000000, 0.85),
            text_secondary: rgba(0x000000, 0.50),
            text_tertiary: rgba(0x000000, 0.26),
            accent: rgb(0xe2a800),
            selection_focused: rgb(0xfdd85d),
            selection_text_focused: rgba(0x000000, 0.85),
            selection_unfocused: rgb(0xe4e4e4),
            hover: rgba(0x000000, 0.05),
            icon: rgba(0x000000, 0.55),
            icon_hover_bg: rgba(0x000000, 0.06),
            icon_pressed_bg: rgba(0x000000, 0.11),
            field: rgba(0x000000, 0.05),
            text_selection: rgba(0xfdd85d, 0.65),
            caret: rgb(0xe2a800),
            menu_border: rgba(0x000000, 0.12),
            window_border: rgba(0x000000, 0.22),
            shadow: rgba(0x000000, 0.18),
        }
    }

    pub fn dark() -> Self {
        Self {
            sidebar: rgb(0x2c2b2b),
            content: rgb(0x1e1e1e),
            divider: rgb(0x0b0b0b),
            row_divider: rgba(0xffffff, 0.07),
            text: rgba(0xffffff, 0.86),
            text_secondary: rgba(0xffffff, 0.52),
            text_tertiary: rgba(0xffffff, 0.25),
            accent: rgb(0xf5c242),
            selection_focused: rgb(0x7a6114),
            selection_text_focused: rgba(0xffffff, 0.92),
            selection_unfocused: rgb(0x3a3a3a),
            hover: rgba(0xffffff, 0.05),
            icon: rgba(0xffffff, 0.58),
            icon_hover_bg: rgba(0xffffff, 0.08),
            icon_pressed_bg: rgba(0xffffff, 0.14),
            field: rgba(0xffffff, 0.08),
            text_selection: rgba(0xb08a1c, 0.75),
            caret: rgb(0xf5c242),
            menu_border: rgba(0xffffff, 0.14),
            window_border: rgba(0xffffff, 0.14),
            shadow: rgba(0x000000, 0.45),
        }
    }
}
