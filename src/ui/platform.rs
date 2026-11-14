//! Desktop integration: which window buttons the user wants, and where.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

/// Window buttons for the left and right side of the header bar, read from
/// KDE's decoration settings (`kwinrc`), falling back to KDE's defaults.
pub fn window_button_layout() -> (Vec<WindowButton>, Vec<WindowButton>) {
    let mut left = "MS".to_string();
    let mut right = "HIAX".to_string();
    let config = dirs::config_dir().map(|d| d.join("kwinrc"));
    if let Some(s) = config.and_then(|p| std::fs::read_to_string(p).ok()) {
        let mut in_section = false;
        for line in s.lines().map(str::trim) {
            if line.starts_with('[') {
                in_section = line == "[org.kde.kdecoration2]";
            } else if in_section {
                if let Some(v) = line.strip_prefix("ButtonsOnLeft=") {
                    left = v.to_string();
                } else if let Some(v) = line.strip_prefix("ButtonsOnRight=") {
                    right = v.to_string();
                }
            }
        }
    }
    let parse = |s: &str| {
        s.chars()
            .filter_map(|c| match c {
                'I' => Some(WindowButton::Minimize),
                'A' => Some(WindowButton::Maximize),
                'X' => Some(WindowButton::Close),
                _ => None,
            })
            .collect()
    };
    (parse(&left), parse(&right))
}
