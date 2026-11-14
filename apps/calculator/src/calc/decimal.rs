//! A small decimal floating-point number: an integer mantissa of up to 18
//! digits times a power of ten. Sums and products of typed-in numbers are
//! exact (0.1 + 0.2 is 0.3), quotients round to 18 significant digits (two
//! more than the display shows), and the exponent range is far beyond any
//! display. That's all a calculator needs, so no big-number dependency.
//! Transcendental functions go through `f64` and are rounded to 15 digits.

use std::cmp::Ordering;

/// Significant digits kept. Two mantissas multiply within a `u128`.
const DIGITS: u32 = 18;
const MAX_EXP: i64 = 9999;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dec {
    neg: bool,
    /// Below 10^18, without trailing zeros (zero is 0e0).
    mant: u128,
    exp: i32,
}

fn pow10(k: u32) -> u128 {
    10u128.pow(k)
}

fn digits(m: u128) -> u32 {
    if m == 0 { 1 } else { m.ilog10() + 1 }
}

/// `m / 10^k`, rounding half away from zero.
fn shift_round(m: u128, k: u32) -> u128 {
    if k == 0 {
        return m;
    }
    if k > 38 {
        return 0;
    }
    let p = pow10(k);
    let (q, r) = (m / p, m % p);
    if r >= p - r { q + 1 } else { q }
}

impl Dec {
    pub const ZERO: Dec = Dec { neg: false, mant: 0, exp: 0 };
    pub const ONE: Dec = Dec { neg: false, mant: 1, exp: 0 };

    /// Rounds to [`DIGITS`] and normalizes. `None` means overflow.
    fn make(neg: bool, mant: u128, exp: i64) -> Option<Dec> {
        Self::make_digits(neg, mant, exp, DIGITS)
    }

    fn make_digits(neg: bool, mut mant: u128, mut exp: i64, keep: u32) -> Option<Dec> {
        if mant == 0 {
            return Some(Dec::ZERO);
        }
        let d = digits(mant);
        if d > keep {
            mant = shift_round(mant, d - keep);
            exp += (d - keep) as i64;
            if mant == 0 {
                return Some(Dec::ZERO);
            }
        }
        while mant % 10 == 0 {
            mant /= 10;
            exp += 1;
        }
        if exp + digits(mant) as i64 - 1 > MAX_EXP {
            return None;
        }
        if exp + (digits(mant) as i64) < -MAX_EXP {
            return Some(Dec::ZERO);
        }
        Some(Dec { neg, mant, exp: exp as i32 })
    }

    pub fn from_int(v: i64) -> Dec {
        Dec::make(v < 0, v.unsigned_abs() as u128, 0).unwrap()
    }

    /// Converts a float result, keeping `sig` significant digits so binary
    /// noise (0.49999999999999994) rounds away.
    pub fn from_f64(v: f64, sig: usize) -> Option<Dec> {
        if !v.is_finite() {
            return None;
        }
        Dec::parse(&format!("{:.*e}", sig.saturating_sub(1), v))
    }

    /// Parses what the keypad builds: `-12.50`, `.5`, `3e-7`, `1.`, `2e`.
    pub fn parse(s: &str) -> Option<Dec> {
        let s = s.trim();
        let (neg, s) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let (num, exp) = match s.split_once(['e', 'E']) {
            Some((n, e)) => (n, if e.is_empty() || e == "-" || e == "+" { 0 } else { e.parse::<i64>().ok()? }),
            None => (s, 0),
        };
        let (int, frac) = num.split_once('.').unwrap_or((num, ""));
        if int.is_empty() && frac.is_empty() || !(int.chars().chain(frac.chars()).all(|c| c.is_ascii_digit())) {
            return None;
        }
        let all = format!("{int}{frac}");
        let all = all.trim_start_matches('0');
        // Digits beyond what we keep only matter for rounding.
        let keep = all.len().min(DIGITS as usize + 1);
        let mant: u128 = if keep == 0 { 0 } else { all[..keep].parse().ok()? };
        let exp = exp - frac.len() as i64 + (all.len() - keep) as i64;
        Dec::make(neg, mant, exp)
    }

    pub fn is_zero(&self) -> bool {
        self.mant == 0
    }

    pub fn is_negative(&self) -> bool {
        self.neg && self.mant != 0
    }

    pub fn is_integer(&self) -> bool {
        self.exp >= 0
    }

    pub fn neg(self) -> Dec {
        if self.is_zero() { self } else { Dec { neg: !self.neg, ..self } }
    }

    pub fn abs(self) -> Dec {
        Dec { neg: false, ..self }
    }

    /// Decimal exponent of the leading digit.
    fn top(&self) -> i64 {
        self.exp as i64 + digits(self.mant) as i64 - 1
    }

    pub fn to_i64(&self) -> Option<i64> {
        if !self.is_integer() || self.top() > 17 {
            return None;
        }
        let v = (self.mant * pow10(self.exp as u32)) as i64;
        Some(if self.neg { -v } else { v })
    }

    pub fn to_f64(&self) -> f64 {
        let v: f64 = format!("{}e{}", self.mant, self.exp).parse().unwrap_or(f64::INFINITY);
        if self.neg { -v } else { v }
    }

    pub fn add(self, o: Dec) -> Option<Dec> {
        if self.is_zero() {
            return Some(o);
        }
        if o.is_zero() {
            return Some(self);
        }
        // Too far apart to change the larger one's 18 digits.
        match self.top() - o.top() {
            d if d > DIGITS as i64 + 1 => return Some(self),
            d if d < -(DIGITS as i64 + 1) => return Some(o),
            _ => {}
        }
        let e = self.exp.min(o.exp);
        let a = self.mant * pow10((self.exp - e) as u32);
        let b = o.mant * pow10((o.exp - e) as u32);
        let (neg, mant) = match (self.neg, o.neg) {
            (x, y) if x == y => (x, a + b),
            _ => match a.cmp(&b) {
                Ordering::Greater => (self.neg, a - b),
                Ordering::Less => (o.neg, b - a),
                Ordering::Equal => return Some(Dec::ZERO),
            },
        };
        Dec::make(neg, mant, e as i64)
    }

    pub fn sub(self, o: Dec) -> Option<Dec> {
        self.add(o.neg())
    }

    pub fn mul(self, o: Dec) -> Option<Dec> {
        Dec::make(self.neg != o.neg, self.mant * o.mant, self.exp as i64 + o.exp as i64)
    }

    /// `None` when dividing by zero.
    pub fn div(self, o: Dec) -> Option<Dec> {
        if o.is_zero() {
            return None;
        }
        if self.is_zero() {
            return Some(Dec::ZERO);
        }
        // Widen the dividend to 37 digits so the quotient has at least 19.
        let s = 37 - digits(self.mant);
        let q = self.mant * pow10(s) / o.mant;
        Dec::make(self.neg != o.neg, q, self.exp as i64 - s as i64 - o.exp as i64)
    }

    /// Integer powers are exact (by repeated squaring); others use `f64`.
    pub fn pow(self, y: Dec) -> Option<Dec> {
        if let Some(n) = y.to_i64().filter(|n| n.unsigned_abs() <= 100_000) {
            let mut result = Dec::ONE;
            let mut base = self;
            let mut k = n.unsigned_abs();
            while k > 0 {
                if k & 1 == 1 {
                    result = result.mul(base)?;
                }
                k >>= 1;
                if k > 0 {
                    base = base.mul(base)?;
                }
            }
            return if n < 0 { Dec::ONE.div(result) } else { Some(result) };
        }
        if self.is_negative() {
            return None;
        }
        Dec::from_f64(self.to_f64().powf(y.to_f64()), 15)
    }

    /// Exact for perfect squares (Newton's method in decimal).
    pub fn sqrt(self) -> Option<Dec> {
        if self.is_negative() {
            return None;
        }
        if self.is_zero() {
            return Some(Dec::ZERO);
        }
        let mut x = Dec::from_f64(self.to_f64().sqrt(), 15)?;
        let half = Dec { neg: false, mant: 5, exp: -1 };
        for _ in 0..3 {
            x = x.add(self.div(x)?)?.mul(half)?;
        }
        Some(x)
    }

    pub fn factorial(self) -> Option<Dec> {
        let n = self.to_i64().filter(|n| (0..=5000).contains(n))?;
        (2..=n).try_fold(Dec::ONE, |acc, k| acc.mul(Dec::from_int(k)))
    }

    /// Every digit, no grouping or exponent (for copying and `EE`).
    pub fn plain(&self) -> String {
        if self.is_zero() {
            return "0".into();
        }
        let m = self.mant.to_string();
        let body = if self.exp >= 0 {
            format!("{m}{}", "0".repeat(self.exp as usize))
        } else {
            let point = m.len() as i64 + self.exp as i64;
            if point > 0 {
                format!("{}.{}", &m[..point as usize], &m[point as usize..])
            } else {
                format!("0.{}{m}", "0".repeat(-point as usize))
            }
        };
        if self.neg { format!("-{body}") } else { body }
    }

    /// How the display shows it, in at most `max` digits: grouped thousands,
    /// switching to scientific notation when it can't fit, like `1.2345e15`.
    pub fn display(&self, max: u32) -> String {
        if self.is_zero() {
            return "0".into();
        }
        let top = self.top();
        if top < max as i64 {
            // Round to the digits that fit after the integer part.
            let frac = max as i64 - top.max(0) - 1;
            let rounded = Dec::make_digits(self.neg, self.mant, self.exp as i64, (top + 1 + frac).max(0) as u32);
            if let Some(r) = rounded.filter(|r| !r.is_zero() && r.top() < max as i64) {
                let exact = r == *self;
                // Tiny numbers that lost digits read better in scientific form.
                if exact || top >= -4 {
                    return group(&r.plain());
                }
            }
        }
        // Scientific: leave room for the exponent.
        let exp_len = format!("e{top}").len() as u32;
        let keep = max.saturating_sub(exp_len).max(1);
        let r = Dec::make_digits(self.neg, self.mant, self.exp as i64, keep).unwrap_or(*self);
        let m = r.mant.to_string();
        let top = r.top();
        let mantissa = if m.len() > 1 { format!("{}.{}", &m[..1], &m[1..]) } else { m };
        format!("{}{mantissa}e{top}", if r.neg { "-" } else { "" })
    }
}

/// Inserts thousands separators into a plain number like `-1234567.5`.
pub fn group(plain: &str) -> String {
    let (sign, rest) = match plain.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", plain),
    };
    let (int, frac) = match rest.find(['.', 'e']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}{frac}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }

    #[test]
    fn exact_decimal_sums() {
        assert_eq!(d("0.1").add(d("0.2")).unwrap(), d("0.3"));
        assert_eq!(d("1.1").mul(d("1.1")).unwrap(), d("1.21"));
        assert_eq!(d("0.3").sub(d("0.1")).unwrap().display(9), "0.2");
        assert_eq!(d("1e30").add(d("1")).unwrap(), d("1e30"));
        assert_eq!(d("123456789012345678").add(d("1")).unwrap().plain(), "123456789012345679");
        assert_eq!(d("-5").add(d("5")).unwrap(), Dec::ZERO);
    }

    #[test]
    fn division() {
        assert_eq!(d("1").div(d("3")).unwrap().display(9), "0.33333333");
        assert_eq!(d("1").div(d("3")).unwrap().mul(d("3")).unwrap().display(16), "1");
        assert_eq!(d("2").div(d("3")).unwrap().display(16), "0.666666666666667");
        assert_eq!(d("1e-9").display(9), "1e-9");
        assert_eq!(d("10").div(d("4")).unwrap(), d("2.5"));
        assert!(d("1").div(Dec::ZERO).is_none());
    }

    #[test]
    fn parse_and_plain() {
        assert_eq!(d("-12.50").plain(), "-12.5");
        assert_eq!(d(".5").plain(), "0.5");
        assert_eq!(d("1.").plain(), "1");
        assert_eq!(d("3e-3").plain(), "0.003");
        assert_eq!(d("2e").plain(), "2");
        assert_eq!(d("000120").plain(), "120");
        assert!(Dec::parse("1.2.3").is_none());
        assert!(Dec::parse("").is_none());
    }

    #[test]
    fn display_rules() {
        assert_eq!(d("1234567").display(9), "1,234,567");
        assert_eq!(d("123456789").display(9), "123,456,789");
        assert_eq!(d("1234567890").display(9), "1.234568e9");
        assert_eq!(d("0.00001").display(9), "0.00001");
        assert_eq!(d("0.000000015").display(9), "1.5e-8");
        assert_eq!(d("-0.5").display(9), "-0.5");
        assert_eq!(d("99999999.99").display(9), "100,000,000");
        assert_eq!(d("1e100").display(16), "1e100");
    }

    #[test]
    fn powers_roots_factorials() {
        assert_eq!(d("2").pow(d("10")).unwrap(), d("1024"));
        assert_eq!(d("2").pow(d("-2")).unwrap(), d("0.25"));
        assert_eq!(d("9").sqrt().unwrap(), d("3"));
        assert_eq!(d("2").sqrt().unwrap().display(16), "1.414213562373095");
        assert_eq!(d("5").factorial().unwrap(), d("120"));
        assert!(d("2.5").factorial().is_none());
        assert!(d("-4").sqrt().is_none());
        assert!(d("10").pow(d("10000")).is_none());
    }

    #[test]
    fn float_bridge_rounds_noise_away() {
        let v = Dec::from_f64(30f64.to_radians().sin(), 15).unwrap();
        assert_eq!(v, d("0.5"));
        assert!(Dec::from_f64(f64::NAN, 15).is_none());
    }
}
