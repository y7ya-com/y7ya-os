use ui::{Color, rgb, rgba};

/// docs/design.md colors, plus the graph and heat-map palette.
#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub content: Color,
    pub sidebar: Color,
    pub divider: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub accent: Color,
    pub hover: Color,
    /// Toolbar icon buttons and gray push buttons: hovered and pressed fills.
    pub control_hover: Color,
    pub control_pressed: Color,
    pub icon: Color,
    pub field: Color,
    pub text_selection: Color,
    pub menu_border: Color,
    pub window_border: Color,
    pub shadow: Color,
    pub destructive: Color,
    pub success: Color,
    pub cpu: Color,
    pub memory: Color,
    pub disk: Color,
    pub network: Color,
    pub gpu: Color,
    /// Heat-map tint (Windows' yellow, as macOS's warm amber).
    heat: Color,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            dark: false,
            content: rgb(0xffffff),
            sidebar: rgb(0xf2f2f4),
            divider: rgba(0x000000, 0.10),
            text: rgba(0x000000, 0.85),
            text_secondary: rgba(0x000000, 0.50),
            text_tertiary: rgba(0x000000, 0.26),
            accent: rgb(0x007aff),
            hover: rgba(0x000000, 0.04),
            control_hover: rgba(0x000000, 0.07),
            control_pressed: rgba(0x000000, 0.12),
            icon: rgba(0x000000, 0.55),
            field: rgba(0x000000, 0.05),
            text_selection: rgba(0x007aff, 0.25),
            menu_border: rgba(0x000000, 0.12),
            window_border: rgba(0x000000, 0.22),
            shadow: rgba(0x000000, 0.18),
            destructive: rgb(0xff3b30),
            success: rgb(0x34c759),
            cpu: rgb(0x007aff),
            memory: rgb(0xaf52de),
            disk: rgb(0x28a745),
            network: rgb(0xff9500),
            gpu: rgb(0x30b0c7),
            heat: rgb(0xff9f0a),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            content: rgb(0x1e1e1e),
            sidebar: rgb(0x2a2a2c),
            divider: rgba(0xffffff, 0.10),
            text: rgba(0xffffff, 0.85),
            text_secondary: rgba(0xffffff, 0.55),
            text_tertiary: rgba(0xffffff, 0.25),
            accent: rgb(0x0a84ff),
            hover: rgba(0xffffff, 0.05),
            control_hover: rgba(0xffffff, 0.11),
            control_pressed: rgba(0xffffff, 0.17),
            icon: rgba(0xffffff, 0.60),
            field: rgba(0xffffff, 0.08),
            text_selection: rgba(0x0a84ff, 0.40),
            menu_border: rgba(0xffffff, 0.14),
            window_border: rgba(0xffffff, 0.14),
            shadow: rgba(0x000000, 0.45),
            destructive: rgb(0xff453a),
            success: rgb(0x30d158),
            cpu: rgb(0x0a84ff),
            memory: rgb(0xbf5af2),
            disk: rgb(0x30d158),
            network: rgb(0xff9f0a),
            gpu: rgb(0x40c8e0),
            heat: rgb(0xff9f0a),
        }
    }

    /// Cell shading for a load from 0 (idle) to 1 (hot). Like Windows, every
    /// cell gets a faint tint, deepening with load; the curve favors the low
    /// end, where most processes sit.
    pub fn heat(&self, load: f32) -> Color {
        let t = load.clamp(0.0, 1.0).sqrt();
        let (lo, hi) = if self.dark { (0.05, 0.42) } else { (0.07, 0.55) };
        self.heat.alpha(lo + (hi - lo) * t)
    }

    pub fn graph_fill(&self, c: Color) -> Color {
        c.alpha(if self.dark { 0.22 } else { 0.14 })
    }

    pub fn graph_grid(&self, c: Color) -> Color {
        c.alpha(if self.dark { 0.16 } else { 0.12 })
    }
}
