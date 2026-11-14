//! The keypad logic: digit entry, operator precedence (2 + 3 × 4 = 14),
//! parentheses, repeated `=`, percent, memory and the scientific functions.
//! Values are exact decimals (see [`Dec`]).

use super::decimal::{Dec, group};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    /// x^y
    Pow,
    /// y^x (the second operand is the base)
    PowRev,
    /// The y-th root of x
    Root,
    /// log base y of x
    LogBase,
    /// x·10^y
    Ee,
}

impl Op {
    fn precedence(self) -> u8 {
        match self {
            Op::Add | Op::Sub => 1,
            Op::Mul | Op::Div => 2,
            _ => 3,
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "\u{2212}",
            Op::Mul => "\u{d7}",
            Op::Div => "\u{f7}",
            Op::Pow => "^",
            Op::PowRev => " yˣ ",
            Op::Root => " ʸ√ ",
            Op::LogBase => " logᵧ ",
            Op::Ee => "E",
        }
    }

    fn apply(self, a: Dec, b: Dec) -> Option<Dec> {
        match self {
            Op::Add => a.add(b),
            Op::Sub => a.sub(b),
            Op::Mul => a.mul(b),
            Op::Div => a.div(b),
            Op::Pow => a.pow(b),
            Op::PowRev => b.pow(a),
            Op::Root => root(a, b),
            Op::LogBase => {
                let (x, y) = (a.to_f64(), b.to_f64());
                if x <= 0.0 || y <= 0.0 || y == 1.0 {
                    return None;
                }
                Dec::from_f64(x.ln() / y.ln(), 15)
            }
            Op::Ee => a.mul(Dec::parse("10")?.pow(b)?),
        }
    }
}

/// One-operand functions applied to the displayed value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Func {
    Square,
    Cube,
    Recip,
    Sqrt,
    Cbrt,
    Exp,
    Pow10,
    Pow2,
    Ln,
    Log10,
    Log2,
    Factorial,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Const {
    Pi,
    E,
    Rand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Digit(u8),
    Point,
    Op(Op),
    Equals,
    Percent,
    Negate,
    Clear,
    Backspace,
    Open,
    Close,
    Func(Func),
    Const(Const),
    MemClear,
    MemAdd,
    MemSub,
    MemRecall,
    Second,
    RadDeg,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    Bin(Op),
    Paren,
}

pub struct Engine {
    /// Operands and operators waiting for higher-precedence ones to finish.
    vals: Vec<Dec>,
    ops: Vec<Pending>,
    /// The value on display (the operand being built).
    cur: Dec,
    /// Digits as typed, while typing.
    entry: Option<String>,
    /// How `cur` reads in the expression line, or `None` if it's already there
    /// (a closed parenthesis).
    label: Option<String>,
    /// An operator or `(` was just pressed: the next digit starts a new operand.
    awaiting: bool,
    /// `=` was just pressed.
    evaluated: bool,
    /// Last operator and operand, so `=` again repeats them.
    repeat: Option<(Op, Dec)>,
    /// The expression line above the result.
    tokens: Vec<String>,
    /// Where each open parenthesis starts in `tokens`.
    groups: Vec<usize>,
    error: bool,
    memory: Option<Dec>,
    pub radians: bool,
    pub second: bool,
    /// Digits that fit on the display (more in scientific mode).
    pub max_digits: u32,
    seed: u64,
}

impl Engine {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
        Self {
            vals: Vec::new(),
            ops: Vec::new(),
            cur: Dec::ZERO,
            entry: None,
            label: None,
            awaiting: true,
            evaluated: false,
            repeat: None,
            tokens: Vec::new(),
            groups: Vec::new(),
            error: false,
            memory: None,
            radians: false,
            second: false,
            max_digits: 9,
            seed,
        }
    }

    fn reset(&mut self) {
        self.vals.clear();
        self.ops.clear();
        self.cur = Dec::ZERO;
        self.entry = None;
        self.label = None;
        self.awaiting = true;
        self.evaluated = false;
        self.repeat = None;
        self.tokens.clear();
        self.groups.clear();
        self.error = false;
    }

    /// After `=`, starting anything but an operator begins a new expression.
    fn start_fresh_if_evaluated(&mut self) {
        if self.evaluated {
            let (cur, repeat) = (self.cur, self.repeat);
            self.reset();
            self.cur = cur;
            self.repeat = repeat;
        }
    }

    // -- what to show -------------------------------------------------------

    pub fn display(&self) -> String {
        if self.error {
            return "Not a number".into();
        }
        match &self.entry {
            Some(e) => match e.split_once('e') {
                Some((m, x)) => format!("{}e{}", group(m), if x.is_empty() { "0" } else { x }),
                None => group(e),
            },
            None => self.cur.display(self.max_digits),
        }
    }

    /// The value as plain digits, for the clipboard.
    pub fn value_text(&self) -> String {
        if self.error { "Not a number".into() } else { Dec::parse(&self.display().replace(',', "")).unwrap_or(self.cur).plain() }
    }

    /// The expression so far (or the one just evaluated).
    pub fn expression(&self) -> String {
        // A lone number being typed is already on the display.
        if self.tokens.is_empty() && self.entry.is_some() {
            return String::new();
        }
        let mut s = self.tokens.concat();
        if !self.evaluated && !self.awaiting {
            if let Some(label) = self.current_label() {
                s.push_str(&label);
            }
        }
        s
    }

    fn current_label(&self) -> Option<String> {
        match &self.entry {
            Some(e) => Some(group(e)),
            None => self.label.clone(),
        }
    }

    /// "C" clears the entry; "AC" clears everything.
    pub fn clear_label(&self) -> &'static str {
        let typed = self.entry.as_deref().is_some_and(|e| e != "0");
        let computed = self.entry.is_none() && !self.awaiting && !self.evaluated && !self.cur.is_zero();
        if !self.error && (typed || computed) { "C" } else { "AC" }
    }

    /// The operator waiting for its second operand, to highlight its key.
    pub fn active_op(&self) -> Option<Op> {
        match (self.awaiting, self.ops.last()) {
            (true, Some(Pending::Bin(op))) if self.entry.is_none() => Some(*op),
            _ => None,
        }
    }

    // -- input --------------------------------------------------------------

    /// Takes a number from the clipboard. Returns false if it isn't one.
    pub fn paste(&mut self, text: &str) -> bool {
        let Some(v) = Dec::parse(&text.replace([',', ' ', '\n'], "")) else { return false };
        if self.error {
            self.reset();
        }
        self.start_fresh_if_evaluated();
        self.set_value(Some(v), v.display(self.max_digits));
        true
    }

    pub fn press(&mut self, key: Key) {
        if self.error && !matches!(key, Key::Clear | Key::Digit(_) | Key::Point | Key::Const(_) | Key::MemRecall) {
            return;
        }
        if self.error {
            self.reset();
        }
        match key {
            Key::Digit(d) => self.digit(d),
            Key::Point => self.point(),
            Key::Op(op) => self.operator(op),
            Key::Equals => self.equals(),
            Key::Percent => self.percent(),
            Key::Negate => self.negate(),
            Key::Clear => self.clear(),
            Key::Backspace => {
                if let Some(e) = &mut self.entry {
                    e.pop();
                    if e.is_empty() || e == "-" {
                        *e = "0".into();
                    }
                    self.sync_entry();
                }
            }
            Key::Open => self.open(),
            Key::Close => self.close(),
            Key::Func(f) => {
                let label = self.take_label();
                let label = match self.fold_group() {
                    Some(group) => group,
                    None => label,
                };
                let v = self.cur;
                let result = self.func(f, v);
                self.set_value(result, func_label(f, &label));
            }
            Key::Const(c) => {
                self.start_fresh_if_evaluated();
                let (v, label) = match c {
                    Const::Pi => (Dec::parse("3.14159265358979323846"), "π".to_string()),
                    Const::E => (Dec::parse("2.71828182845904523536"), "e".to_string()),
                    Const::Rand => {
                        let v = Dec::from_f64(self.random(), 15);
                        (v, v.map(|v| v.display(self.max_digits)).unwrap_or_default())
                    }
                };
                self.set_value(v, label);
            }
            Key::MemClear => self.memory = None,
            Key::MemAdd | Key::MemSub => {
                let v = self.operand();
                let v = if key == Key::MemSub { v.neg() } else { v };
                self.memory = self.memory.unwrap_or(Dec::ZERO).add(v);
                self.finish_entry();
            }
            Key::MemRecall => {
                if let Some(m) = self.memory {
                    self.start_fresh_if_evaluated();
                    self.set_value(Some(m), m.display(self.max_digits));
                }
            }
            Key::Second => self.second = !self.second,
            Key::RadDeg => self.radians = !self.radians,
        }
    }

    fn digit(&mut self, d: u8) {
        self.start_fresh_if_evaluated();
        let e = self.entry.get_or_insert_with(String::new);
        let (mantissa, exponent) = match e.split_once('e') {
            Some((m, x)) => (m.to_string(), Some(x.to_string())),
            None => (e.clone(), None),
        };
        match exponent {
            Some(x) if x.trim_start_matches('-').len() < 4 => e.push((b'0' + d) as char),
            Some(_) => {}
            None => {
                let count = mantissa.chars().filter(|c| c.is_ascii_digit()).count() as u32;
                if mantissa == "0" {
                    *e = ((b'0' + d) as char).to_string();
                } else if mantissa == "-0" {
                    *e = format!("-{d}");
                } else if count < self.max_digits {
                    e.push((b'0' + d) as char);
                }
            }
        }
        self.awaiting = false;
        self.sync_entry();
    }

    fn point(&mut self) {
        self.start_fresh_if_evaluated();
        let e = self.entry.get_or_insert_with(|| "0".into());
        if !e.contains(['.', 'e']) {
            e.push('.');
        }
        self.awaiting = false;
        self.sync_entry();
    }

    fn clear(&mut self) {
        if self.clear_label() == "C" {
            self.entry = Some("0".into());
            self.label = None;
            self.cur = Dec::ZERO;
            self.awaiting = false;
        } else {
            self.reset();
        }
    }

    fn sync_entry(&mut self) {
        if let Some(e) = &self.entry {
            self.cur = Dec::parse(e).unwrap_or(Dec::ZERO);
        }
    }

    fn negate(&mut self) {
        if let Some(e) = &mut self.entry {
            match e.find('e') {
                Some(i) if e[i + 1..].starts_with('-') => {
                    e.remove(i + 1);
                }
                Some(i) => e.insert(i + 1, '-'),
                None => match e.strip_prefix('-') {
                    Some(rest) => *e = rest.to_string(),
                    None => e.insert(0, '-'),
                },
            }
            self.sync_entry();
        } else if self.awaiting && !self.evaluated {
            self.entry = Some("-0".into());
            self.awaiting = false;
            self.sync_entry();
        } else {
            let label = self.take_label();
            let v = self.cur.neg();
            self.set_value(Some(v), if label.starts_with('-') { label[1..].to_string() } else { format!("-{label}") });
        }
    }

    /// The operand on display, as a value.
    fn operand(&self) -> Dec {
        self.cur
    }

    /// The display's label for a new value derived from it, starting a fresh
    /// expression after `=`.
    fn take_label(&mut self) -> String {
        let label = self.current_label().unwrap_or_else(|| self.cur.display(self.max_digits));
        self.start_fresh_if_evaluated();
        label
    }

    /// When a function applies to a just-closed parenthesis, pulls that group
    /// out of the expression line so it can be wrapped.
    fn fold_group(&mut self) -> Option<String> {
        if self.label.is_some() || self.entry.is_some() || self.awaiting {
            return None;
        }
        let start = self.groups.pop()?;
        Some(self.tokens.drain(start..).collect())
    }

    fn set_value(&mut self, v: Option<Dec>, label: String) {
        match v {
            Some(v) => {
                self.cur = v;
                self.entry = None;
                self.label = Some(label);
                self.awaiting = false;
                self.evaluated = false;
            }
            None => self.error = true,
        }
    }

    /// Typing stops; what's on display stays as the operand.
    fn finish_entry(&mut self) {
        if let Some(e) = self.entry.take() {
            self.label = Some(group(&e));
        }
    }

    /// Pushes the current operand onto the stack and the expression line.
    fn commit(&mut self) {
        let label = self.current_label();
        self.vals.push(self.cur);
        if let Some(l) = label {
            self.tokens.push(l);
        }
        self.entry = None;
        self.label = None;
    }

    fn reduce_one(&mut self) -> bool {
        let Some(Pending::Bin(op)) = self.ops.pop() else { return false };
        let (Some(b), Some(a)) = (self.vals.pop(), self.vals.pop()) else { return false };
        match op.apply(a, b) {
            Some(v) => {
                self.vals.push(v);
                true
            }
            None => {
                self.error = true;
                false
            }
        }
    }

    fn operator(&mut self, op: Op) {
        if self.evaluated {
            // Continue from the result.
            let cur = self.cur;
            self.reset();
            self.cur = cur;
            self.label = Some(cur.display(self.max_digits));
            self.awaiting = false;
        }
        if self.awaiting && matches!(self.ops.last(), Some(Pending::Bin(_))) {
            // Pressing another operator replaces the last one.
            self.ops.pop();
            self.tokens.pop();
        } else {
            if self.awaiting && self.label.is_none() {
                self.label = Some(self.cur.display(self.max_digits));
            }
            self.commit();
        }
        while let Some(Pending::Bin(prev)) = self.ops.last() {
            let p = (prev.precedence(), op.precedence());
            // Powers group right to left.
            if p.0 > p.1 || (p.0 == p.1 && op.precedence() < 3) {
                if !self.reduce_one() {
                    return;
                }
            } else {
                break;
            }
        }
        self.ops.push(Pending::Bin(op));
        self.tokens.push(op.symbol().into());
        self.cur = *self.vals.last().unwrap_or(&Dec::ZERO);
        self.awaiting = true;
    }

    fn open(&mut self) {
        self.start_fresh_if_evaluated();
        if !self.awaiting {
            // "2(" means "2×(".
            self.operator(Op::Mul);
        }
        self.ops.push(Pending::Paren);
        self.groups.push(self.tokens.len());
        self.tokens.push("(".into());
        self.cur = Dec::ZERO;
        self.awaiting = true;
    }

    fn close(&mut self) {
        if !self.ops.contains(&Pending::Paren) || self.evaluated {
            return;
        }
        if self.awaiting && self.label.is_none() {
            self.label = Some(self.cur.display(self.max_digits));
        }
        self.commit();
        while let Some(Pending::Bin(_)) = self.ops.last() {
            if !self.reduce_one() {
                return;
            }
        }
        self.ops.pop();
        self.tokens.push(")".into());
        self.cur = self.vals.pop().unwrap_or(Dec::ZERO);
        self.label = None;
        self.awaiting = false;
    }

    fn equals(&mut self) {
        if self.evaluated {
            if let Some((op, b)) = self.repeat {
                let a = self.cur;
                self.tokens = vec![a.display(self.max_digits), op.symbol().into(), b.display(self.max_digits)];
                match op.apply(a, b) {
                    Some(v) => self.cur = v,
                    None => self.error = true,
                }
            }
            return;
        }
        if self.awaiting && self.label.is_none() {
            // "2+=" uses the displayed value as the missing operand.
            self.label = Some(self.cur.display(self.max_digits));
        }
        let last_op = self.ops.iter().rev().find_map(|p| match p {
            Pending::Bin(op) => Some(*op),
            Pending::Paren => None,
        });
        let operand = self.cur;
        self.commit();
        while let Some(top) = self.ops.last() {
            if *top == Pending::Paren {
                self.ops.pop();
                self.tokens.push(")".into());
            } else if !self.reduce_one() {
                break;
            }
        }
        if !self.error {
            self.cur = self.vals.pop().unwrap_or(Dec::ZERO);
            self.repeat = last_op.map(|op| (op, operand));
        }
        self.vals.clear();
        self.ops.clear();
        self.groups.clear();
        self.awaiting = false;
        self.evaluated = true;
    }

    fn percent(&mut self) {
        let label = self.take_label();
        let hundred = Dec::from_int(100);
        // "50 + 10%" is 10% of 50, as on a desk calculator.
        let v = match (self.ops.last(), self.vals.last()) {
            (Some(Pending::Bin(Op::Add | Op::Sub)), Some(base)) if !self.awaiting => base.mul(self.cur).and_then(|v| v.div(hundred)),
            _ => self.cur.div(hundred),
        };
        self.set_value(v, format!("{label}%"));
    }

    fn func(&mut self, f: Func, x: Dec) -> Option<Dec> {
        let fx = x.to_f64();
        let angle = |v: f64| if self.radians { v } else { v.to_radians() };
        let unangle = |v: f64| if self.radians { v } else { v.to_degrees() };
        let float = |v: f64| Dec::from_f64(v, 15);
        match f {
            Func::Square => x.mul(x),
            Func::Cube => x.mul(x)?.mul(x),
            Func::Recip => Dec::ONE.div(x),
            Func::Sqrt => x.sqrt(),
            Func::Cbrt => root(x, Dec::from_int(3)),
            Func::Exp => float(fx.exp()),
            Func::Pow10 => Dec::from_int(10).pow(x),
            Func::Pow2 => Dec::from_int(2).pow(x),
            Func::Ln | Func::Log10 | Func::Log2 if fx <= 0.0 => None,
            Func::Ln => float(fx.ln()),
            Func::Log10 => float(fx.log10()),
            Func::Log2 => float(fx.log2()),
            Func::Factorial => x.factorial(),
            Func::Sin | Func::Cos | Func::Tan => {
                if !self.radians {
                    // Exact values at multiples of 90°.
                    if let Some(deg) = x.to_i64().filter(|d| d % 90 == 0) {
                        let q = deg.rem_euclid(360) / 90;
                        let (s, c) = [(0, 1), (1, 0), (0, -1), (-1, 0)][q as usize];
                        return match f {
                            Func::Sin => Some(Dec::from_int(s)),
                            Func::Cos => Some(Dec::from_int(c)),
                            _ if c == 0 => None,
                            _ => Some(Dec::ZERO),
                        };
                    }
                }
                let a = angle(fx);
                let v = match f {
                    Func::Sin => a.sin(),
                    Func::Cos => a.cos(),
                    _ => a.tan(),
                };
                // Float noise around zero (sin π) reads as 0.
                if v.abs() < 1e-15 { Some(Dec::ZERO) } else { float(v) }
            }
            Func::Asin | Func::Acos if fx.abs() > 1.0 => None,
            Func::Asin => float(unangle(fx.asin())),
            Func::Acos => float(unangle(fx.acos())),
            Func::Atan => float(unangle(fx.atan())),
            Func::Sinh => float(fx.sinh()),
            Func::Cosh => float(fx.cosh()),
            Func::Tanh => float(fx.tanh()),
            Func::Asinh => float(fx.asinh()),
            Func::Acosh => float(fx.acosh()),
            Func::Atanh => float(fx.atanh()),
        }
    }

    /// xorshift: plenty for a Rand key.
    fn random(&mut self) -> f64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// The key a typed character stands for: digits, `.`, `+ - * / x ^ ( ) = %`.
pub fn key_for_char(c: char) -> Option<Key> {
    Some(match c {
        '0'..='9' => Key::Digit(c as u8 - b'0'),
        '.' | ',' => Key::Point,
        '+' => Key::Op(Op::Add),
        '-' | '\u{2212}' => Key::Op(Op::Sub),
        '*' | 'x' | 'X' | '\u{d7}' => Key::Op(Op::Mul),
        '/' | '\u{f7}' => Key::Op(Op::Div),
        '^' => Key::Op(Op::Pow),
        '(' => Key::Open,
        ')' => Key::Close,
        '=' => Key::Equals,
        '%' => Key::Percent,
        _ => return None,
    })
}

/// The y-th root of x; odd roots of negatives are allowed.
fn root(x: Dec, y: Dec) -> Option<Dec> {
    if y.is_zero() {
        return None;
    }
    if y == Dec::from_int(2) {
        return x.sqrt();
    }
    let odd = y.to_i64().is_some_and(|n| n % 2 != 0);
    if x.is_negative() && !odd {
        return None;
    }
    let r = x.abs().to_f64().powf(1.0 / y.to_f64());
    // Snap to an exact integer root when there is one (∛27 = 3).
    let rounded = Dec::from_f64(r.round(), 15)?;
    let r = if rounded.pow(y) == Some(x.abs()) { rounded } else { Dec::from_f64(r, 15)? };
    Some(if x.is_negative() { r.neg() } else { r })
}

fn func_label(f: Func, x: &str) -> String {
    let wrap = |name: &str| format!("{name}({x})");
    match f {
        Func::Square => format!("{x}²"),
        Func::Cube => format!("{x}³"),
        Func::Recip => format!("1/{x}"),
        Func::Sqrt => format!("√{x}"),
        Func::Cbrt => format!("∛{x}"),
        Func::Exp => format!("e^{x}"),
        Func::Pow10 => format!("10^{x}"),
        Func::Pow2 => format!("2^{x}"),
        Func::Ln => wrap("ln"),
        Func::Log10 => wrap("log"),
        Func::Log2 => wrap("log₂"),
        Func::Factorial => format!("{x}!"),
        Func::Sin => wrap("sin"),
        Func::Cos => wrap("cos"),
        Func::Tan => wrap("tan"),
        Func::Asin => wrap("sin⁻¹"),
        Func::Acos => wrap("cos⁻¹"),
        Func::Atan => wrap("tan⁻¹"),
        Func::Sinh => wrap("sinh"),
        Func::Cosh => wrap("cosh"),
        Func::Tanh => wrap("tanh"),
        Func::Asinh => wrap("sinh⁻¹"),
        Func::Acosh => wrap("cosh⁻¹"),
        Func::Atanh => wrap("tanh⁻¹"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Presses keys written like "12+3*4=": digits, . + - * / ^ ( ) = % n (±) c (clear) b (backspace).
    fn run(e: &mut Engine, keys: &str) -> String {
        for ch in keys.chars() {
            let key = match ch {
                'n' => Key::Negate,
                'c' => Key::Clear,
                'b' => Key::Backspace,
                _ => key_for_char(ch).expect("unknown key"),
            };
            e.press(key);
        }
        e.display()
    }

    fn calc(keys: &str) -> String {
        run(&mut Engine::new(), keys)
    }

    #[test]
    fn exact_decimals() {
        assert_eq!(calc("0.1+0.2="), "0.3");
        assert_eq!(calc("1/3*3="), "1");
        assert_eq!(calc("0.3-0.1="), "0.2");
        assert_eq!(calc("1.1*1.1="), "1.21");
    }

    #[test]
    fn precedence_and_parentheses() {
        assert_eq!(calc("2+3*4="), "14");
        assert_eq!(calc("(2+3)*4="), "20");
        assert_eq!(calc("2*(3+4="), "14");
        assert_eq!(calc("2^3^2="), "512");
        assert_eq!(calc("10-2-3="), "5");
        assert_eq!(calc("2(3)="), "6");
    }

    #[test]
    fn display_while_typing() {
        let mut e = Engine::new();
        assert_eq!(run(&mut e, "1234567"), "1,234,567");
        assert_eq!(run(&mut e, "89"), "123,456,789");
        // The basic display holds nine digits.
        assert_eq!(run(&mut e, "0"), "123,456,789");
        assert_eq!(run(&mut e, "c"), "0");
        assert_eq!(calc("2+3*"), "3");
        assert_eq!(calc("2*3+"), "6");
        assert_eq!(calc("1.50"), "1.50");
        assert_eq!(calc("12bb"), "0");
    }

    #[test]
    fn expression_line() {
        let mut e = Engine::new();
        run(&mut e, "12+3*4");
        assert_eq!(e.expression(), "12+3\u{d7}4");
        run(&mut e, "=");
        assert_eq!(e.expression(), "12+3\u{d7}4");
        assert_eq!(e.display(), "24");
        e.press(Key::Func(Func::Square));
        assert_eq!(e.expression(), "24²");
        assert_eq!(e.display(), "576");
    }

    #[test]
    fn repeated_equals_and_operator_changes() {
        assert_eq!(calc("2+3=="), "8");
        assert_eq!(calc("5*=="), "125");
        assert_eq!(calc("2+*3="), "6");
        assert_eq!(calc("2*+3="), "5");
        assert_eq!(calc("4=+1="), "5");
    }

    #[test]
    fn percent_negate_and_clear() {
        assert_eq!(calc("50+10%="), "55");
        assert_eq!(calc("50*10%="), "5");
        assert_eq!(calc("25%"), "0.25");
        assert_eq!(calc("5n"), "-5");
        assert_eq!(calc("5nn"), "5");
        assert_eq!(calc("3+n2="), "1");
        // C clears the entry only; AC clears everything.
        let mut e = Engine::new();
        run(&mut e, "8+5");
        assert_eq!(e.clear_label(), "C");
        run(&mut e, "c");
        assert_eq!(e.clear_label(), "AC");
        assert_eq!(run(&mut e, "2="), "10");
        run(&mut e, "c");
        assert_eq!(e.expression(), "");
        assert_eq!(e.display(), "0");
    }

    #[test]
    fn errors() {
        assert_eq!(calc("1/0="), "Not a number");
        let mut e = Engine::new();
        run(&mut e, "1/0=");
        // Operators are ignored until a clear or new number.
        assert_eq!(run(&mut e, "+"), "Not a number");
        assert_eq!(run(&mut e, "7"), "7");
        assert_eq!(calc("0/0="), "Not a number");
    }

    #[test]
    fn big_and_small_numbers() {
        assert_eq!(calc("999999999*999999999="), "1e18");
        assert_eq!(calc("1/100000000/10="), "1e-9");
        let mut e = Engine::new();
        e.max_digits = 16;
        assert_eq!(run(&mut e, "2/3="), "0.666666666666667");
    }

    #[test]
    fn scientific_functions() {
        let mut e = Engine::new();
        run(&mut e, "30");
        e.press(Key::Func(Func::Sin));
        assert_eq!(e.display(), "0.5");
        run(&mut e, "c");
        run(&mut e, "90");
        e.press(Key::Func(Func::Cos));
        assert_eq!(e.display(), "0");
        run(&mut e, "c");
        run(&mut e, "90");
        e.press(Key::Func(Func::Tan));
        assert_eq!(e.display(), "Not a number");

        let mut e = Engine::new();
        e.radians = true;
        e.press(Key::Const(Const::Pi));
        e.press(Key::Func(Func::Sin));
        assert_eq!(e.display(), "0");

        let mut e = Engine::new();
        run(&mut e, "27");
        e.press(Key::Func(Func::Cbrt));
        assert_eq!(e.display(), "3");
        run(&mut e, "c5");
        e.press(Key::Func(Func::Factorial));
        assert_eq!(e.display(), "120");
        run(&mut e, "c100");
        e.press(Key::Func(Func::Log10));
        assert_eq!(e.display(), "2");
        run(&mut e, "c8");
        e.press(Key::Op(Op::Root));
        run(&mut e, "3=");
        assert_eq!(e.display(), "2");

        // ee: 1.5 EE 3 is 1500.
        let mut e = Engine::new();
        run(&mut e, "1.5");
        e.press(Key::Op(Op::Ee));
        assert_eq!(run(&mut e, "3="), "1,500");
    }

    #[test]
    fn memory() {
        let mut e = Engine::new();
        run(&mut e, "5");
        e.press(Key::MemAdd);
        run(&mut e, "c3");
        e.press(Key::MemAdd);
        run(&mut e, "c");
        e.press(Key::MemRecall);
        assert_eq!(e.display(), "8");
        e.press(Key::MemClear);
        e.press(Key::MemRecall);
        assert_eq!(e.display(), "8");
    }

    #[test]
    fn copy_text_is_plain() {
        let mut e = Engine::new();
        run(&mut e, "1234.5");
        assert_eq!(e.value_text(), "1234.5");
        run(&mut e, "*1000000=");
        assert_eq!(e.value_text(), "1234500000");
    }
}
