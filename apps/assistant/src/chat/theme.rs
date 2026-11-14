use ui::{Color, rgb, rgba};

/// Colors from docs/design.md, plus the chat's own surfaces.
#[derive(Clone, Copy)]
pub struct Theme {
    pub window: Color,
    /// The Spotlight-style quick panel.
    pub panel: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub separator: Color,
    pub field: Color,
    pub field_border: Color,
    pub accent: Color,
    pub card: Color,
    pub card_border: Color,
    pub hover: Color,
    pub success: Color,
    pub destructive: Color,
    pub warning: Color,
    pub menu_bg: Color,
    pub menu_border: Color,
    pub shadow: Color,
    pub window_border: Color,
    pub text_selection: Color,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            window: rgb(0xffffff),
            panel: rgb(0xf6f6f6),
            text: rgba(0x000000, 0.85),
            text_secondary: rgba(0x000000, 0.50),
            text_tertiary: rgba(0x000000, 0.26),
            separator: rgba(0x000000, 0.10),
            field: rgba(0x000000, 0.05),
            field_border: rgba(0x000000, 0.08),
            accent: rgb(0x007aff),
            card: rgb(0xf5f5f7),
            card_border: rgba(0x000000, 0.07),
            hover: rgba(0x000000, 0.06),
            success: rgb(0x34c759),
            destructive: rgb(0xff3b30),
            warning: rgb(0xff9f0a),
            menu_bg: rgba(0xf6f6f6, 0.98),
            menu_border: rgba(0x000000, 0.12),
            shadow: rgba(0x000000, 0.18),
            window_border: rgba(0x000000, 0.22),
            text_selection: rgba(0x007aff, 0.25),
        }
    }

    pub fn dark() -> Self {
        Self {
            window: rgb(0x1e1e1e),
            panel: rgb(0x2c2c2e),
            text: rgba(0xffffff, 0.85),
            text_secondary: rgba(0xffffff, 0.55),
            text_tertiary: rgba(0xffffff, 0.25),
            separator: rgba(0xffffff, 0.10),
            field: rgba(0xffffff, 0.08),
            field_border: rgba(0xffffff, 0.06),
            accent: rgb(0x0a84ff),
            card: rgb(0x2a2a2c),
            card_border: rgba(0xffffff, 0.07),
            hover: rgba(0xffffff, 0.08),
            success: rgb(0x30d158),
            destructive: rgb(0xff453a),
            warning: rgb(0xff9f0a),
            menu_bg: rgba(0x2c2c2e, 0.98),
            menu_border: rgba(0xffffff, 0.14),
            shadow: rgba(0x000000, 0.45),
            window_border: rgba(0xffffff, 0.14),
            text_selection: rgba(0x0a84ff, 0.45),
        }
    }
}
