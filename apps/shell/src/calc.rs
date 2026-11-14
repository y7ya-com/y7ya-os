//! Spotlight's calculator ("2^10 / 4", "sqrt(2) * pi") and unit conversions
//! ("10 km in mi", "72 f to c"). No currencies: those need the network.

/// What Spotlight shows for a query, if it's math or a conversion.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// `value` formatted for display, e.g. "1,024".
    Math { expr: String, value: String },
    Conversion { from: String, value: String },
}

impl Answer {
    pub fn value(&self) -> &str {
        match self {
            Answer::Math { value, .. } | Answer::Conversion { value, .. } => value,
        }
    }
}

pub fn answer(query: &str) -> Option<Answer> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    if let Some((value, from)) = convert(q) {
        return Some(Answer::Conversion { from, value });
    }
    let v = eval(q)?;
    // A bare number isn't a calculation, and "e" or "pi" alone are more
    // likely the start of a name.
    if q.parse::<f64>().is_ok() || !q.chars().any(|c| c.is_ascii_digit()) || !v.is_finite() {
        return None;
    }
    Some(Answer::Math { expr: q.to_string(), value: format_number(v) })
}

// -- expressions ----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == ',' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
            // "1,000": thousands separators are skipped.
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
                i += 1;
            }
            // Exponent: 1e6, 2.5E-3.
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '-' || chars[j] == '+') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().filter(|&&c| c != ',').collect();
            out.push(Tok::Num(text.parse().ok()?));
        } else if c.is_alphabetic() || c == 'π' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == 'π') {
                i += 1;
            }
            let ident = chars[start..i].iter().collect::<String>().to_lowercase();
            // "3 x 4" multiplies.
            out.push(if ident == "x" { Tok::Op('*') } else { Tok::Ident(ident) });
        } else {
            let op = match c {
                '×' | '·' => '*',
                '÷' => '/',
                '−' => '-',
                '*' if chars.get(i + 1) == Some(&'*') => {
                    i += 1;
                    '^'
                }
                '+' | '-' | '*' | '/' | '^' | '%' | '(' | ')' | '!' => c,
                _ => return None,
            };
            out.push(Tok::Op(op));
            i += 1;
        }
    }
    Some(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Tok::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat('+') {
                v += self.term()?;
            } else if self.eat('-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            if self.eat('*') {
                v *= self.unary()?;
            } else if self.eat('/') {
                v /= self.unary()?;
            } else if self.eat('%') {
                v %= self.unary()?;
            } else if matches!(self.peek(), Some(Tok::Op('(')) | Some(Tok::Ident(_))) {
                // Implicit multiplication: 2pi, 3(4+1).
                v *= self.unary()?;
            } else {
                return Some(v);
            }
        }
    }

    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.postfix()?;
        if self.eat('^') {
            // Right-associative, and binds tighter than a leading minus: -2^2 = -4.
            let exp = self.unary()?;
            return Some(base.powf(exp));
        }
        Some(base)
    }

    fn postfix(&mut self) -> Option<f64> {
        let mut v = self.primary()?;
        while self.eat('!') {
            if v < 0.0 || v.fract() != 0.0 || v > 170.0 {
                return None;
            }
            v = (1..=v as u64).map(|n| n as f64).product();
        }
        Some(v)
    }

    fn primary(&mut self) -> Option<f64> {
        match self.toks.get(self.pos).cloned()? {
            Tok::Num(n) => {
                self.pos += 1;
                Some(n)
            }
            Tok::Op('(') => {
                self.pos += 1;
                let v = self.expr()?;
                // Forgive a missing closing paren at the end, like a calculator.
                if !self.eat(')') && self.peek().is_some() {
                    return None;
                }
                Some(v)
            }
            Tok::Ident(name) => {
                self.pos += 1;
                match name.as_str() {
                    "pi" | "π" => return Some(std::f64::consts::PI),
                    "e" => return Some(std::f64::consts::E),
                    "tau" => return Some(std::f64::consts::TAU),
                    _ => {}
                }
                let f: fn(f64) -> f64 = match name.as_str() {
                    "sqrt" => f64::sqrt,
                    "cbrt" => f64::cbrt,
                    "sin" => f64::sin,
                    "cos" => f64::cos,
                    "tan" => f64::tan,
                    "asin" => f64::asin,
                    "acos" => f64::acos,
                    "atan" => f64::atan,
                    "ln" => f64::ln,
                    "log" => f64::log10,
                    "log2" => f64::log2,
                    "exp" => f64::exp,
                    "abs" => f64::abs,
                    "floor" => f64::floor,
                    "ceil" => f64::ceil,
                    "round" => f64::round,
                    _ => return None,
                };
                // Functions take a parenthesized argument or a plain number (sqrt 2).
                let arg = self.power()?;
                Some(f(arg))
            }
            Tok::Op(_) => None,
        }
    }
}

/// Evaluates an arithmetic expression; `None` if it isn't one.
pub fn eval(s: &str) -> Option<f64> {
    let toks = tokenize(s)?;
    if toks.is_empty() {
        return None;
    }
    let mut p = Parser { toks, pos: 0 };
    let v = p.expr()?;
    (p.pos == p.toks.len()).then_some(v)
}

/// "1,234.5", "0.333333333", "1.5e+20".
pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if v.abs() >= 1e15 || v.abs() < 1e-9 {
        let s = format!("{v:.6e}");
        let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let mant = mant.trim_end_matches('0').trim_end_matches('.');
        let exp: i32 = exp.parse().unwrap_or(0);
        return format!("{mant}e{}{}", if exp >= 0 { "+" } else { "" }, exp);
    }
    // Up to 10 significant digits after the point, no float noise (0.1+0.2).
    let digits = (9 - v.abs().log10().floor() as i32).clamp(0, 10) as usize;
    let s = format!("{:.*}", digits, v);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    let (sign, s) = s.strip_prefix('-').map_or(("", s.as_str()), |r| ("-", r));
    let (int, frac) = s.split_once('.').map_or((s, None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

// -- unit conversions -----------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Length,
    Mass,
    Volume,
    Time,
    Data,
    Speed,
    Area,
    Temperature,
}

struct Unit {
    kind: Kind,
    /// Size in the kind's base unit (meters, kilograms, liters, seconds, bytes, m/s, m²).
    factor: f64,
    symbol: &'static str,
    names: &'static [&'static str],
}

const UNITS: &[Unit] = &[
    Unit { kind: Kind::Length, factor: 0.001, symbol: "mm", names: &["mm", "millimeter", "millimeters", "millimetre", "millimetres"] },
    Unit { kind: Kind::Length, factor: 0.01, symbol: "cm", names: &["cm", "centimeter", "centimeters", "centimetre", "centimetres"] },
    Unit { kind: Kind::Length, factor: 1.0, symbol: "m", names: &["m", "meter", "meters", "metre", "metres"] },
    Unit { kind: Kind::Length, factor: 1000.0, symbol: "km", names: &["km", "kilometer", "kilometers", "kilometre", "kilometres"] },
    Unit { kind: Kind::Length, factor: 0.0254, symbol: "in", names: &["in", "inch", "inches", "\""] },
    Unit { kind: Kind::Length, factor: 0.3048, symbol: "ft", names: &["ft", "foot", "feet", "'"] },
    Unit { kind: Kind::Length, factor: 0.9144, symbol: "yd", names: &["yd", "yard", "yards"] },
    Unit { kind: Kind::Length, factor: 1609.344, symbol: "mi", names: &["mi", "mile", "miles"] },
    Unit { kind: Kind::Length, factor: 1852.0, symbol: "nmi", names: &["nmi", "nautical mile", "nautical miles"] },
    Unit { kind: Kind::Mass, factor: 1e-6, symbol: "mg", names: &["mg", "milligram", "milligrams"] },
    Unit { kind: Kind::Mass, factor: 0.001, symbol: "g", names: &["g", "gram", "grams"] },
    Unit { kind: Kind::Mass, factor: 1.0, symbol: "kg", names: &["kg", "kilo", "kilos", "kilogram", "kilograms"] },
    Unit { kind: Kind::Mass, factor: 1000.0, symbol: "t", names: &["t", "tonne", "tonnes", "metric ton", "metric tons"] },
    Unit { kind: Kind::Mass, factor: 0.028349523125, symbol: "oz", names: &["oz", "ounce", "ounces"] },
    Unit { kind: Kind::Mass, factor: 0.45359237, symbol: "lb", names: &["lb", "lbs", "pound", "pounds"] },
    Unit { kind: Kind::Mass, factor: 6.35029318, symbol: "st", names: &["st", "stone", "stones"] },
    Unit { kind: Kind::Volume, factor: 0.001, symbol: "mL", names: &["ml", "milliliter", "milliliters", "millilitre", "millilitres"] },
    Unit { kind: Kind::Volume, factor: 0.01, symbol: "cL", names: &["cl", "centiliter", "centiliters"] },
    Unit { kind: Kind::Volume, factor: 1.0, symbol: "L", names: &["l", "liter", "liters", "litre", "litres"] },
    Unit { kind: Kind::Volume, factor: 1000.0, symbol: "m³", names: &["m3", "m³", "cubic meter", "cubic meters"] },
    Unit { kind: Kind::Volume, factor: 0.00492892159375, symbol: "tsp", names: &["tsp", "teaspoon", "teaspoons"] },
    Unit { kind: Kind::Volume, factor: 0.01478676478125, symbol: "tbsp", names: &["tbsp", "tablespoon", "tablespoons"] },
    Unit { kind: Kind::Volume, factor: 0.0295735295625, symbol: "fl oz", names: &["floz", "fl oz", "fluid ounce", "fluid ounces"] },
    Unit { kind: Kind::Volume, factor: 0.2365882365, symbol: "cups", names: &["cup", "cups"] },
    Unit { kind: Kind::Volume, factor: 0.473176473, symbol: "pt", names: &["pt", "pint", "pints"] },
    Unit { kind: Kind::Volume, factor: 0.946352946, symbol: "qt", names: &["qt", "quart", "quarts"] },
    Unit { kind: Kind::Volume, factor: 3.785411784, symbol: "gal", names: &["gal", "gallon", "gallons"] },
    Unit { kind: Kind::Time, factor: 0.001, symbol: "ms", names: &["ms", "millisecond", "milliseconds"] },
    Unit { kind: Kind::Time, factor: 1.0, symbol: "s", names: &["s", "sec", "secs", "second", "seconds"] },
    Unit { kind: Kind::Time, factor: 60.0, symbol: "min", names: &["min", "mins", "minute", "minutes"] },
    Unit { kind: Kind::Time, factor: 3600.0, symbol: "h", names: &["h", "hr", "hrs", "hour", "hours"] },
    Unit { kind: Kind::Time, factor: 86400.0, symbol: "days", names: &["d", "day", "days"] },
    Unit { kind: Kind::Time, factor: 604800.0, symbol: "weeks", names: &["wk", "week", "weeks"] },
    Unit { kind: Kind::Time, factor: 31557600.0, symbol: "years", names: &["yr", "year", "years"] },
    Unit { kind: Kind::Data, factor: 0.125, symbol: "bits", names: &["bit", "bits"] },
    Unit { kind: Kind::Data, factor: 1.0, symbol: "bytes", names: &["b", "byte", "bytes"] },
    Unit { kind: Kind::Data, factor: 1e3, symbol: "KB", names: &["kb", "kilobyte", "kilobytes"] },
    Unit { kind: Kind::Data, factor: 1e6, symbol: "MB", names: &["mb", "megabyte", "megabytes"] },
    Unit { kind: Kind::Data, factor: 1e9, symbol: "GB", names: &["gb", "gigabyte", "gigabytes"] },
    Unit { kind: Kind::Data, factor: 1e12, symbol: "TB", names: &["tb", "terabyte", "terabytes"] },
    Unit { kind: Kind::Data, factor: 1024.0, symbol: "KiB", names: &["kib", "kibibyte", "kibibytes"] },
    Unit { kind: Kind::Data, factor: 1048576.0, symbol: "MiB", names: &["mib", "mebibyte", "mebibytes"] },
    Unit { kind: Kind::Data, factor: 1073741824.0, symbol: "GiB", names: &["gib", "gibibyte", "gibibytes"] },
    Unit { kind: Kind::Data, factor: 1099511627776.0, symbol: "TiB", names: &["tib", "tebibyte", "tebibytes"] },
    Unit { kind: Kind::Speed, factor: 1.0, symbol: "m/s", names: &["m/s", "mps"] },
    Unit { kind: Kind::Speed, factor: 1.0 / 3.6, symbol: "km/h", names: &["km/h", "kmh", "kph"] },
    Unit { kind: Kind::Speed, factor: 0.44704, symbol: "mph", names: &["mph", "mi/h"] },
    Unit { kind: Kind::Speed, factor: 0.514444, symbol: "kn", names: &["kn", "kt", "knot", "knots"] },
    Unit { kind: Kind::Area, factor: 1.0, symbol: "m²", names: &["m2", "m²", "sqm", "square meter", "square meters"] },
    Unit { kind: Kind::Area, factor: 1e6, symbol: "km²", names: &["km2", "km²", "square kilometer", "square kilometers"] },
    Unit { kind: Kind::Area, factor: 0.09290304, symbol: "ft²", names: &["ft2", "ft²", "sqft", "square foot", "square feet"] },
    Unit { kind: Kind::Area, factor: 1e4, symbol: "ha", names: &["ha", "hectare", "hectares"] },
    Unit { kind: Kind::Area, factor: 4046.8564224, symbol: "acres", names: &["acre", "acres"] },
    Unit { kind: Kind::Area, factor: 2589988.110336, symbol: "mi²", names: &["mi2", "mi²", "sqmi", "square mile", "square miles"] },
    Unit { kind: Kind::Temperature, factor: 0.0, symbol: "°C", names: &["c", "°c", "celsius", "centigrade"] },
    Unit { kind: Kind::Temperature, factor: 1.0, symbol: "°F", names: &["f", "°f", "fahrenheit"] },
    Unit { kind: Kind::Temperature, factor: 2.0, symbol: "K", names: &["k", "kelvin", "kelvins"] },
];

fn unit(name: &str) -> Option<&'static Unit> {
    let name = name.trim().trim_start_matches("degrees ").trim_start_matches("degree ");
    UNITS.iter().find(|u| u.names.iter().any(|n| n.eq_ignore_ascii_case(name)))
}

fn to_kelvin(v: f64, u: &Unit) -> f64 {
    match u.factor as u8 {
        0 => v + 273.15,
        1 => (v - 32.0) * 5.0 / 9.0 + 273.15,
        _ => v,
    }
}

fn from_kelvin(k: f64, u: &Unit) -> f64 {
    match u.factor as u8 {
        0 => k - 273.15,
        1 => (k - 273.15) * 9.0 / 5.0 + 32.0,
        _ => k,
    }
}

/// "10 km in mi" → ("6.21371 mi", "10 km"). The amount may be an expression.
pub fn convert(q: &str) -> Option<(String, String)> {
    let lower = q.to_lowercase();
    let (left, target) = [" to ", " in ", " as ", " → ", " -> ", " = "]
        .iter()
        .find_map(|sep| lower.rsplit_once(sep))?;
    let to = unit(target)?;
    let left = left.trim();
    // Split the amount from the unit: the longest unit name that ends `left`.
    let (amount, from) = UNITS
        .iter()
        .flat_map(|u| u.names.iter().map(move |n| (u, *n)))
        .filter(|(_, n)| left.ends_with(n))
        .filter(|(_, n)| left.len() > n.len())
        .max_by_key(|(_, n)| n.len())
        .map(|(u, n)| (left[..left.len() - n.len()].trim(), u))?;
    let amount = amount.trim_end_matches("degrees").trim_end_matches("degree").trim();
    let v = amount.parse::<f64>().ok().or_else(|| eval(amount))?;
    if from.kind != to.kind {
        return None;
    }
    let out = if from.kind == Kind::Temperature {
        from_kelvin(to_kelvin(v, from), to)
    } else {
        v * from.factor / to.factor
    };
    let shown = format_number(round_sig(out, 6));
    Some((format!("{shown} {}", to.symbol), format!("{} {}", format_number(v), from.symbol)))
}

fn round_sig(v: f64, digits: i32) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let mag = 10f64.powi(digits - 1 - v.abs().log10().floor() as i32);
    (v * mag).round() / mag
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(s: &str) -> String {
        answer(s).map(|a| a.value().to_string()).unwrap_or_default()
    }

    #[test]
    fn arithmetic() {
        assert_eq!(val("2+2"), "4");
        assert_eq!(val("2 + 3 * 4"), "14");
        assert_eq!(val("(2 + 3) * 4"), "20");
        assert_eq!(val("2^10"), "1,024");
        assert_eq!(val("2^3^2"), "512");
        assert_eq!(val("-2^2"), "-4");
        assert_eq!(val("10 / 4"), "2.5");
        assert_eq!(val("0.1 + 0.2"), "0.3");
        assert_eq!(val("7 % 3"), "1");
        assert_eq!(val("5!"), "120");
        assert_eq!(val("1,000 * 3"), "3,000");
        assert_eq!(val("2(3+4)"), "14");
        assert_eq!(val("1/3"), "0.3333333333");
        assert_eq!(val("1e20 * 10"), "1e+21");
        assert_eq!(val("3 × 4 ÷ 2"), "6");
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(val("sqrt(16)"), "4");
        assert_eq!(val("sqrt 2"), "1.414213562");
        assert_eq!(val("2pi"), "6.283185307");
        assert_eq!(val("log(1000)"), "3");
        assert_eq!(val("ln(e^2)"), "2");
        assert_eq!(val("abs(-3) + floor(2.7)"), "5");
        assert_eq!(val("(1+2"), "3");
    }

    #[test]
    fn not_math() {
        for q in ["", "42", "firefox", "2 +", "sqrt", "hello world", "1/0", "(", "foo(2)", "e", "pi", "7zip"] {
            assert_eq!(answer(q), None, "{q}");
        }
    }

    #[test]
    fn conversions() {
        assert_eq!(val("10 km in mi"), "6.21371 mi");
        assert_eq!(val("5 ft to cm"), "152.4 cm");
        assert_eq!(val("100 c to f"), "212 °F");
        assert_eq!(val("72 F to C"), "22.2222 °C");
        assert_eq!(val("0 celsius in kelvin"), "273.15 K");
        assert_eq!(val("2 kg in lbs"), "4.40925 lb");
        assert_eq!(val("1 gib to mb"), "1,073.74 MB");
        assert_eq!(val("90 min to hours"), "1.5 h");
        assert_eq!(val("60 mph in km/h"), "96.5606 km/h");
        assert_eq!(val("1 gallon in liters"), "3.78541 L");
        assert_eq!(val("2*3 m in cm"), "600 cm");
        assert_eq!(val("10km to m"), "10,000 m");
        // Different kinds don't convert.
        assert_eq!(convert("10 km in kg"), None);
        assert_eq!(convert("km in mi"), None);
    }
}
