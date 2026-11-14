use ui::{Color, rgb, rgba};

/// Colors sampled to match macOS Calendar in light and dark appearance.
#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub sidebar: Color,
    pub content: Color,
    pub divider: Color,
    /// Hour lines and day separators.
    pub grid: Color,
    /// Weekend columns and days outside the month.
    pub shade: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    /// Today, the current-time line.
    pub red: Color,
    pub accent: Color,
    pub hover: Color,
    pub icon: Color,
    pub icon_hover_bg: Color,
    pub icon_pressed_bg: Color,
    pub field: Color,
    pub text_selection: Color,
    pub menu_bg: Color,
    pub menu_border: Color,
    pub window_border: Color,
    pub shadow: Color,
    pub button_bg: Color,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            dark: false,
            sidebar: rgb(0xf2f2f4),
            content: rgb(0xffffff),
            divider: rgba(0x000000, 0.10),
            grid: rgba(0x000000, 0.09),
            shade: rgba(0x000000, 0.025),
            text: rgba(0x000000, 0.85),
            text_secondary: rgba(0x000000, 0.50),
            text_tertiary: rgba(0x000000, 0.26),
            red: rgb(0xff3b30),
            accent: rgb(0x007aff),
            hover: rgba(0x000000, 0.05),
            icon: rgba(0x000000, 0.55),
            icon_hover_bg: rgba(0x000000, 0.06),
            icon_pressed_bg: rgba(0x000000, 0.11),
            field: rgba(0x000000, 0.05),
            text_selection: rgba(0x007aff, 0.25),
            menu_bg: rgba(0xf6f6f6, 0.98),
            menu_border: rgba(0x000000, 0.12),
            window_border: rgba(0x000000, 0.22),
            shadow: rgba(0x000000, 0.18),
            button_bg: rgba(0x000000, 0.06),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            sidebar: rgb(0x2a2a2c),
            content: rgb(0x1e1e1e),
            divider: rgb(0x0b0b0b),
            grid: rgba(0xffffff, 0.09),
            shade: rgba(0xffffff, 0.025),
            text: rgba(0xffffff, 0.86),
            text_secondary: rgba(0xffffff, 0.55),
            text_tertiary: rgba(0xffffff, 0.25),
            red: rgb(0xff453a),
            accent: rgb(0x0a84ff),
            hover: rgba(0xffffff, 0.05),
            icon: rgba(0xffffff, 0.58),
            icon_hover_bg: rgba(0xffffff, 0.08),
            icon_pressed_bg: rgba(0xffffff, 0.14),
            field: rgba(0xffffff, 0.08),
            text_selection: rgba(0x0a84ff, 0.45),
            menu_bg: rgba(0x2c2c2e, 0.98),
            menu_border: rgba(0xffffff, 0.14),
            window_border: rgba(0xffffff, 0.14),
            shadow: rgba(0x000000, 0.45),
            button_bg: rgba(0xffffff, 0.10),
        }
    }

    /// A calendar's color, slightly brighter in dark mode like macOS.
    pub fn cal(&self, name: &str) -> Color {
        let c = rgb(calendar_core::color_rgb(name));
        if self.dark { mix(c, Color::WHITE, 0.08) } else { c }
    }

    /// Event block colors for a calendar: (fill, bar, text).
    pub fn event_colors(&self, name: &str, selected: bool) -> (Color, Color, Color) {
        let c = self.cal(name);
        if selected {
            return (c, c, Color::WHITE);
        }
        if self.dark {
            (mix(self.content, c, 0.32), c, mix(c, Color::WHITE, 0.55))
        } else {
            (mix(Color::WHITE, c, 0.18), c, mix(c, Color::BLACK, 0.45))
        }
    }
}

/// `a` blended toward `b` by `t`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}
