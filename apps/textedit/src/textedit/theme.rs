use ui::{Color, rgb, rgba};

/// The system colors from `docs/design.md`, plus TextEdit's text highlights.
#[derive(Clone, Copy)]
pub struct Theme {
    pub window: Color,
    pub bar: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub separator: Color,
    pub field: Color,
    pub hover: Color,
    pub toggled: Color,
    /// A control held down.
    pub pressed: Color,
    pub accent: Color,
    pub destructive: Color,
    pub icon: Color,
    pub selection: Color,
    pub selection_inactive: Color,
    pub find_match: Color,
    pub find_current: Color,
    pub scrollbar: Color,
    pub sheet: Color,
    pub sheet_border: Color,
    pub shadow: Color,
    pub window_border: Color,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            window: rgb(0xffffff),
            bar: rgb(0xf2f2f4),
            text: rgba(0x000000, 0.85),
            text_secondary: rgba(0x000000, 0.50),
            text_tertiary: rgba(0x000000, 0.26),
            separator: rgba(0x000000, 0.10),
            field: rgba(0x000000, 0.05),
            hover: rgba(0x000000, 0.06),
            toggled: rgba(0x000000, 0.10),
            pressed: rgba(0x000000, 0.16),
            accent: rgb(0x007aff),
            destructive: rgb(0xff3b30),
            icon: rgba(0x000000, 0.55),
            selection: rgb(0xb3d7ff),
            selection_inactive: rgb(0xdcdcdc),
            find_match: rgba(0xffe000, 0.35),
            find_current: rgb(0xffe23a),
            scrollbar: rgba(0x000000, 0.28),
            sheet: rgb(0xf6f6f6),
            sheet_border: rgba(0x000000, 0.12),
            shadow: rgba(0x000000, 0.22),
            window_border: rgba(0x000000, 0.22),
        }
    }

    pub fn dark() -> Self {
        Self {
            window: rgb(0x1e1e1e),
            bar: rgb(0x2a2a2c),
            text: rgba(0xffffff, 0.85),
            text_secondary: rgba(0xffffff, 0.55),
            text_tertiary: rgba(0xffffff, 0.25),
            separator: rgba(0xffffff, 0.10),
            field: rgba(0xffffff, 0.08),
            hover: rgba(0xffffff, 0.08),
            toggled: rgba(0xffffff, 0.14),
            pressed: rgba(0xffffff, 0.22),
            accent: rgb(0x0a84ff),
            destructive: rgb(0xff453a),
            icon: rgba(0xffffff, 0.58),
            selection: rgb(0x3f638b),
            selection_inactive: rgb(0x464646),
            find_match: rgba(0xffd60a, 0.28),
            find_current: rgba(0xffd60a, 0.62),
            scrollbar: rgba(0xffffff, 0.32),
            sheet: rgb(0x2c2c2e),
            sheet_border: rgba(0xffffff, 0.14),
            shadow: rgba(0x000000, 0.5),
            window_border: rgba(0xffffff, 0.14),
        }
    }
}
