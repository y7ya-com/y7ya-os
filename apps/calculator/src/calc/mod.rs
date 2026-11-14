pub mod decimal;
pub mod engine;
mod icons;

use std::time::{Duration, Instant};

use engine::{Const, Engine, Func, Key, Op, key_for_char};
use ui::Key as WinitKey;
use ui::widgets::{Menu, MenuItem, MenuKey};
use ui::*;

/// macOS Calculator keeps its dark look in both appearances.
const WINDOW_BG: u32 = 0x1f1f21;
const DIGIT: u32 = 0x3a3a3c;
const DIGIT_PRESSED: u32 = 0x6e6e70;
const FUNCTION: u32 = 0x5c5c5f;
const FUNCTION_PRESSED: u32 = 0x8f8f92;
const SCIENTIFIC: u32 = 0x2c2c2e;
const SCIENTIFIC_PRESSED: u32 = 0x515154;
const OPERATOR: u32 = 0xff9f0a;
const OPERATOR_PRESSED: u32 = 0xfcc78d;

/// Space above the display at the default size; the traffic lights sit in
/// it (and over the display's empty top), placed as in a toolbar window.
const HEADER_H: f32 = 32.0;
const PAD: f32 = 12.0;
const GAP: f32 = 9.0;
const BUTTON: f32 = 48.0;
/// The display's height at the default window size.
const DISPLAY_H: f32 = 104.0;
const FLASH: Duration = Duration::from_millis(160);

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Basic,
    Scientific,
}

impl Mode {
    fn columns(self) -> usize {
        match self {
            Mode::Basic => 4,
            Mode::Scientific => 10,
        }
    }

    /// The window size that fits the keypad at its natural size.
    pub fn window_size(self) -> (f32, f32) {
        let n = self.columns() as f32;
        let w = 2.0 * PAD + n * BUTTON + (n - 1.0) * GAP;
        (w, HEADER_H + DISPLAY_H + 5.0 * BUTTON + 4.0 * GAP + PAD)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Digit,
    Function,
    Operator,
    Scientific,
}

/// What a key shows: text, or a drawn glyph.
#[derive(Clone, Copy)]
enum Label {
    Text(&'static str),
    Negate,
}

pub struct CalculatorApp {
    pub engine: Engine,
    mode: Mode,
    /// The Basic / Scientific menu from the calculator key.
    menu: Option<Menu<Mode>>,
    /// A key pressed on the keyboard, shown pressed for a moment.
    flash: Option<(Key, Instant)>,
}

impl CalculatorApp {
    pub fn new(mode: Mode, cx: &mut Cx) -> Self {
        cx.window_radius = 10.0;
        let mut app = Self { engine: Engine::new(), mode: Mode::Basic, menu: None, flash: None };
        app.engine.max_digits = digits_for(mode);
        app.mode = mode;
        app
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Cx) {
        self.menu = None;
        if mode != self.mode {
            self.mode = mode;
            self.engine.max_digits = digits_for(mode);
            let (w, h) = mode.window_size();
            cx.resize_window(w, h);
        }
    }

    pub fn press(&mut self, key: Key) {
        self.menu = None;
        self.engine.press(key);
    }

    /// Presses `key` as if typed: it flashes like a click.
    fn press_flash(&mut self, key: Key) {
        self.press(key);
        self.flash = Some((key, Instant::now()));
    }

    fn copy(&mut self, cx: &mut Cx) {
        if let Some(cb) = cx.clipboard.as_mut() {
            let _ = cb.set_text(self.engine.value_text());
        }
    }

    fn paste(&mut self, cx: &mut Cx) {
        if let Some(text) = cx.clipboard.as_mut().and_then(|cb| cb.get_text().ok()) {
            self.menu = None;
            self.engine.paste(&text);
        }
    }

    // -- rendering ----------------------------------------------------------

    fn display(&self, cx: &mut Cx, width: f32) -> Div<Self> {
        let base = match self.mode {
            Mode::Basic => 52.0,
            Mode::Scientific => 58.0,
        };
        let value = self.engine.display();
        let style = TextStyle { size: base, line_height: 1.15, weight: Weight::Regular, color: Color::WHITE };
        // Shrink the digits to fit, like macOS. Figures are fixed-width so
        // the number doesn't jitter as digits change.
        let (w, _) = cx.text.measure_with(&value, &style, None, false, true);
        let size = if w > width { (base * width / w).floor().max(16.0) } else { base };

        let expr_style = TextStyle { size: 17.0, line_height: 1.3, weight: Weight::Regular, color: Color::WHITE.alpha(0.5) };
        let mut expr = self.engine.expression();
        // Long expressions keep their end in view.
        while !expr.is_empty() && cx.text.measure_with(&expr, &expr_style, None, false, true).0 > width {
            let mut chars = expr.chars();
            chars.next();
            chars.next();
            expr = format!("\u{2026}{}", chars.as_str());
        }

        let rad = self.mode == Mode::Scientific && self.engine.radians;
        div()
            .flex_1()
            .flex_col()
            .justify_end()
            .items_end()
            .px(PAD + 6.0)
            .pb(4.0)
            .window_drag()
            .child(
                div()
                    .w_full()
                    .h(22.0)
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(text(if rad { "Rad" } else { "" }).size(13.0).color(Color::WHITE.alpha(0.6)))
                    .child(text(expr).size(17.0).tabular().color(Color::WHITE.alpha(0.5))),
            )
            .child(text(value).size(size).line_height(1.15).tabular().color(Color::WHITE))
    }

    fn key_button(&self, key: Key, label: Label, style: Style, d: f32, cx: &Cx) -> Div<Self> {
        let hovered = cx.hovered(("key", key));
        let pressed = hovered && cx.mouse_down || self.flash.is_some_and(|(k, _)| k == key);
        let active = matches!(key, Key::Op(op) if self.engine.active_op() == Some(op))
            || (key == Key::Second && self.engine.second);
        let (bg, pressed_bg) = match style {
            Style::Digit => (DIGIT, DIGIT_PRESSED),
            Style::Function => (FUNCTION, FUNCTION_PRESSED),
            Style::Operator => (OPERATOR, OPERATOR_PRESSED),
            Style::Scientific => (SCIENTIFIC, SCIENTIFIC_PRESSED),
        };
        let (bg, fg) = match (pressed, active, style) {
            (true, ..) => (rgb(pressed_bg), Color::WHITE),
            (false, true, Style::Operator) => (rgb(0xffffff), rgb(OPERATOR)),
            (false, true, _) => (rgb(SCIENTIFIC_PRESSED), Color::WHITE),
            // Hovered keys light up a little, so the pointer has a target.
            _ if hovered => (lighten(rgb(bg)), Color::WHITE),
            _ => (rgb(bg), Color::WHITE),
        };
        let size = match style {
            Style::Operator => d * 0.56,
            Style::Function => d * 0.4,
            Style::Digit => d * 0.5,
            Style::Scientific => d * 0.3,
        };
        let content: AnyElement<Self> = match label {
            Label::Text(s) => text(s).size(size).weight(Weight::Medium).color(fg).into(),
            Label::Negate => canvas(move |s, b| icons::plus_minus(s, b, fg)).size(d * 0.5).into(),
        };
        div()
            .id(("key", key))
            .size(d)
            .shrink_0()
            .rounded(d / 2.0)
            .bg(bg)
            .items_center()
            .justify_center()
            .on_click(move |s: &mut Self, _| s.press(key))
            .child(content)
    }

    fn mode_button(&self, d: f32, cx: &Cx) -> Div<Self> {
        let hovered = cx.hovered("mode");
        let bg = match (hovered && cx.mouse_down || self.menu.is_some(), hovered) {
            (true, _) => rgb(DIGIT_PRESSED),
            (false, true) => lighten(rgb(DIGIT)),
            _ => rgb(DIGIT),
        };
        div()
            .id("mode")
            .size(d)
            .shrink_0()
            .rounded(d / 2.0)
            .bg(bg)
            .items_center()
            .justify_center()
            .on_click(|s: &mut Self, cx: &mut Cx| s.open_menu(cx))
            .child(canvas(icons::calculator).size(d * 0.5))
    }

    /// Opens the mode menu just above the calculator key.
    pub fn open_menu(&mut self, cx: &Cx) {
        let (w, h) = cx.window_size;
        let d = self.key_size(cx);
        let cols = self.mode.columns() as f32;
        // The keypad is centered; in Scientific the key follows six columns.
        let left = ((w - cols * d - (cols - 1.0) * GAP) / 2.0).max(PAD);
        let x = left + if self.mode == Mode::Scientific { 6.0 * (d + GAP) } else { 0.0 };
        let top = h - PAD - d;
        let items = [(Mode::Basic, "Basic", "Ctrl+1"), (Mode::Scientific, "Scientific", "Ctrl+2")]
            .into_iter()
            .map(|(mode, label, shortcut)| MenuItem::new(label, mode).shortcut(shortcut).checked(self.mode == mode))
            .collect();
        // Opens upward, its bottom 6pt above the key (two 24pt rows, 5pt padding).
        let menu_h = 2.0 * 24.0 + 2.0 * 5.0;
        self.menu = Some(Menu::new(Point { x, y: (top - 6.0 - menu_h).max(0.0) }, items));
    }

    /// Key diameter: keys grow with the window but keep their circles.
    fn key_size(&self, cx: &Cx) -> f32 {
        let (w, h) = cx.window_size;
        let cols = self.mode.columns() as f32;
        let by_width = (w - 2.0 * PAD - (cols - 1.0) * GAP) / cols;
        let by_height = (h - HEADER_H - 70.0 - PAD - 4.0 * GAP) / 5.0;
        by_width.min(by_height).max(24.0).floor()
    }

    fn basic_rows(&self) -> [[(Key, Label, Style); 4]; 5] {
        use Label::Text as T;
        use Style::*;
        let digit = |n: u8| (Key::Digit(n), T(["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"][n as usize]), Digit);
        [
            [
                (Key::Clear, T(self.engine.clear_label()), Function),
                (Key::Negate, Label::Negate, Function),
                (Key::Percent, T("%"), Function),
                (Key::Op(Op::Div), T("\u{f7}"), Operator),
            ],
            [digit(7), digit(8), digit(9), (Key::Op(Op::Mul), T("\u{d7}"), Operator)],
            [digit(4), digit(5), digit(6), (Key::Op(Op::Sub), T("\u{2212}"), Operator)],
            [digit(1), digit(2), digit(3), (Key::Op(Op::Add), T("+"), Operator)],
            // The first slot is the mode button.
            [digit(0), digit(0), (Key::Point, T("."), Digit), (Key::Equals, T("="), Operator)],
        ]
    }

    fn scientific_rows(&self) -> [[(Key, &'static str); 6]; 5] {
        let second = self.engine.second;
        let pick = |a: (Key, &'static str), b: (Key, &'static str)| if second { b } else { a };
        let f = Key::Func;
        [
            [(Key::Open, "("), (Key::Close, ")"), (Key::MemClear, "mc"), (Key::MemAdd, "m+"), (Key::MemSub, "m\u{2212}"), (Key::MemRecall, "mr")],
            [
                (Key::Second, "2\u{207f}\u{1d48}"),
                (f(Func::Square), "x²"),
                (f(Func::Cube), "x³"),
                (Key::Op(Op::Pow), "x\u{02b8}"),
                pick((f(Func::Exp), "e\u{02e3}"), (Key::Op(Op::PowRev), "y\u{02e3}")),
                pick((f(Func::Pow10), "10\u{02e3}"), (f(Func::Pow2), "2\u{02e3}")),
            ],
            [
                (f(Func::Recip), "1/x"),
                (f(Func::Sqrt), "√x"),
                (f(Func::Cbrt), "∛x"),
                (Key::Op(Op::Root), "\u{02b8}√x"),
                pick((f(Func::Ln), "ln"), (Key::Op(Op::LogBase), "log\u{1d67}")),
                pick((f(Func::Log10), "log₁₀"), (f(Func::Log2), "log₂")),
            ],
            [
                (f(Func::Factorial), "x!"),
                pick((f(Func::Sin), "sin"), (f(Func::Asin), "sin⁻¹")),
                pick((f(Func::Cos), "cos"), (f(Func::Acos), "cos⁻¹")),
                pick((f(Func::Tan), "tan"), (f(Func::Atan), "tan⁻¹")),
                (Key::Const(Const::E), "e"),
                (Key::Op(Op::Ee), "EE"),
            ],
            [
                (Key::RadDeg, if self.engine.radians { "Deg" } else { "Rad" }),
                pick((f(Func::Sinh), "sinh"), (f(Func::Asinh), "sinh⁻¹")),
                pick((f(Func::Cosh), "cosh"), (f(Func::Acosh), "cosh⁻¹")),
                pick((f(Func::Tanh), "tanh"), (f(Func::Atanh), "tanh⁻¹")),
                (Key::Const(Const::Pi), "π"),
                (Key::Const(Const::Rand), "Rand"),
            ],
        ]
    }

    fn keypad(&self, d: f32, cx: &Cx) -> Div<Self> {
        let sci = self.scientific_rows();
        let mut pad = div().flex_col().gap(GAP).px(PAD).pb(PAD).items_center();
        for (r, row) in self.basic_rows().into_iter().enumerate() {
            let mut line = div().flex_row().gap(GAP);
            if self.mode == Mode::Scientific {
                for (key, label) in sci[r] {
                    line = line.child(self.key_button(key, Label::Text(label), Style::Scientific, d, cx));
                }
            }
            for (c, (key, label, style)) in row.into_iter().enumerate() {
                line = line.child(if r == 4 && c == 0 { self.mode_button(d, cx) } else { self.key_button(key, label, style, d, cx) });
            }
            pad = pad.child(line);
        }
        pad
    }
}

/// A key's color a little lighter, for hover.
fn lighten(c: Color) -> Color {
    let k = 0.1;
    Color { r: c.r + (1.0 - c.r) * k, g: c.g + (1.0 - c.g) * k, b: c.b + (1.0 - c.b) * k, a: c.a }
}

fn digits_for(mode: Mode) -> u32 {
    match mode {
        Mode::Basic => 9,
        Mode::Scientific => 16,
    }
}

impl App for CalculatorApp {
    fn base_text_style(&self, _cx: &Cx) -> TextStyle {
        TextStyle { size: 13.0, line_height: 1.3, weight: Weight::Regular, color: rgba(0xffffff, 0.85) }
    }

    fn render(&mut self, cx: &mut Cx) -> Div<Self> {
        let (w, h) = cx.window_size;
        let d = self.key_size(cx);

        // The traffic lights sit where a toolbar window has them (20pt in,
        // centered in a 52pt bar), over the display's empty top.
        let mut header = div()
            .absolute(0.0, 0.0)
            .w(w)
            .h(native::TOOLBAR_H)
            .flex_row()
            .items_center()
            .pl(native::TRAFFIC_LIGHTS_X)
            .pr(13.0)
            .window_drag();
        // The keypad stays dark, so the window buttons do too.
        let dark = cx.dark;
        cx.dark = true;
        if let Some(buttons) = ui::chrome::window_buttons(true, cx, rgba(0xffffff, 0.85)) {
            header = header.child(buttons);
        }
        let right = ui::chrome::window_buttons(false, cx, rgba(0xffffff, 0.85));
        cx.dark = dark;
        if let Some(buttons) = right {
            header = header.child(div().flex_1()).child(buttons);
        }

        let display_w = w - 2.0 * (PAD + 6.0);
        let mut root = div()
            .w(w)
            .h(h)
            .flex_col()
            .bg(rgb(WINDOW_BG))
            .child(self.display(cx, display_w))
            .child(self.keypad(d, cx))
            .child(header);
        if let Some(menu) = &self.menu {
            // Dark like the rest of the window, in both appearances.
            cx.dark = true;
            let panel = menu.render(cx, |s: &mut Self, mode, cx| s.set_mode(mode, cx), |s: &mut Self, _| s.menu = None);
            cx.dark = dark;
            root = root.child(panel);
        }
        if cx.transparent && !cx.maximized && !ui::is_y7ya() {
            root = root.child(div().inset_0().rounded(cx.window_radius).border(rgba(0xffffff, 0.14), 1.0));
        }
        root
    }

    fn on_key(&mut self, key: &KeyPress, cx: &mut Cx) -> bool {
        // The open mode menu takes every key (arrows, Return, Escape, letters).
        if let Some(menu) = &mut self.menu {
            match menu.key(key, cx) {
                MenuKey::Pick(mode) => self.set_mode(mode, cx),
                MenuKey::Close => self.menu = None,
                MenuKey::Stay => {}
            }
            return true;
        }
        if key.ctrl && !key.alt {
            match &key.key {
                WinitKey::Character(c) => match c.to_lowercase().as_str() {
                    "c" => self.copy(cx),
                    "v" => self.paste(cx),
                    "1" => self.set_mode(Mode::Basic, cx),
                    "2" => self.set_mode(Mode::Scientific, cx),
                    "w" | "q" => cx.close_window(),
                    _ => return false,
                },
                _ => return false,
            }
            return true;
        }
        let k = match &key.key {
            WinitKey::Named(NamedKey::Enter) => Some(Key::Equals),
            WinitKey::Named(NamedKey::Backspace | NamedKey::Delete) => Some(Key::Backspace),
            WinitKey::Named(NamedKey::Escape) => Some(Key::Clear),
            _ if key.alt && key.is_char('-') => Some(Key::Negate),
            _ => key.text.as_deref().and_then(|t| t.chars().next()).and_then(key_for_char),
        };
        match k {
            Some(k) => {
                self.press_flash(k);
                true
            }
            None => false,
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        self.flash.map(|(_, t)| t + FLASH)
    }

    fn on_wake(&mut self, _cx: &mut Cx) {
        if self.flash.is_some_and(|(_, t)| t.elapsed() >= FLASH) {
            self.flash = None;
        }
    }
}
