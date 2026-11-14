use ui::{Color, rgb, rgba};

/// docs/design.md's tokens plus the shell's own surfaces. No blur anywhere:
/// panels are solid or lightly translucent over the wallpaper.
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
    /// The menu bar: wallpaper shows through.
    pub bar: Color,
    pub bar_text: Color,
    /// A menu-bar item while its menu is open.
    pub bar_highlight: Color,
    pub menu: Color,
    pub border: Color,
    pub shadow: Color,
    /// Control Center's background and its modules.
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
                bar: rgba(0x16141c, 0.42),
                bar_text: rgba(0xffffff, 0.92),
                bar_highlight: rgba(0xffffff, 0.18),
                menu: rgba(0x2c2c2e, 0.98),
                border: rgba(0xffffff, 0.14),
                shadow: rgba(0x000000, 0.45),
                panel: rgba(0x232325, 0.97),
                module: rgba(0xffffff, 0.07),
                card: rgba(0x2a2a2c, 0.97),
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
                bar: rgba(0xf6f5f8, 0.62),
                bar_text: rgba(0x000000, 0.85),
                bar_highlight: rgba(0x000000, 0.10),
                menu: rgba(0xf6f6f6, 0.98),
                border: rgba(0x000000, 0.12),
                shadow: rgba(0x000000, 0.22),
                panel: rgba(0xe9e9eb, 0.97),
                module: rgba(0xffffff, 0.78),
                card: rgba(0xf7f7f8, 0.97),
                hover: rgba(0x000000, 0.05),
            }
        }
    }
}
