mod g_table;
mod parse;
mod to_decimal;

use std::fmt;

pub use parse::{NumberFormatError, parse_boolean, parse_double, parse_int, parse_long};

use to_decimal::{Split, layout, plain_mantissa, split_double, split_float};

/// Same text as Java's `Float.toString`, which differs from Rust's `{}` for floats.
pub fn float_to_string(v: f32) -> String {
    let mut s = String::with_capacity(16);
    push_float(&mut s, v);
    s
}

/// Same text as Java's `Double.toString`.
pub fn double_to_string(v: f64) -> String {
    let mut s = String::with_capacity(24);
    push_double(&mut s, v);
    s
}

pub fn push_float(out: &mut String, v: f32) {
    push_split(out, v.is_sign_negative(), split_float(v));
}

pub fn push_double(out: &mut String, v: f64) {
    push_split(out, v.is_sign_negative(), split_double(v));
}

fn push_split(out: &mut String, negative: bool, split: Split) {
    match split {
        Split::NaN => out.push_str("NaN"),
        Split::Infinite if negative => out.push_str("-Infinity"),
        Split::Infinite => out.push_str("Infinity"),
        Split::Zero if negative => out.push_str("-0.0"),
        Split::Zero => out.push_str("0.0"),
        Split::Finite(d) => {
            if negative {
                out.push('-');
            }
            layout(out, d);
        }
    }
}

/// Java's `%.Nf`: rounds half-up on the shortest digits, unlike Rust's `{:.N}`.
pub fn format_fixed(v: f64, decimals: usize) -> String {
    let mut out = String::new();
    push_fixed(&mut out, v, decimals);
    out
}

pub fn push_fixed(out: &mut String, v: f64, decimals: usize) {
    if v.is_nan() {
        out.push_str("NaN");
        return;
    }
    if v.is_sign_negative() {
        out.push('-');
    }
    let abs = v.abs();
    if abs.is_infinite() {
        out.push_str("Infinity");
        return;
    }
    let prec = decimals as i64;
    let mant = plain_mantissa(abs, prec);
    out.push_str(&mant);
    let dot = mant.find('.');
    let have = match dot {
        Some(i) => mant.len() - i - 1,
        None => 0,
    };
    if have < decimals {
        if dot.is_none() {
            out.push('.');
        }
        for _ in have..decimals {
            out.push('0');
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JavaFloat(pub f32);

impl fmt::Display for JavaFloat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&float_to_string(self.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JavaDouble(pub f64);

impl fmt::Display for JavaDouble {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&double_to_string(self.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fixed {
    pub value: f64,
    pub decimals: usize,
}

impl Fixed {
    pub fn new(value: f64, decimals: usize) -> Self {
        Self { value, decimals }
    }
}

impl fmt::Display for Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format_fixed(self.value, self.decimals))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(float_to_string(100.0), "100.0");
        assert_eq!(float_to_string(98.76), "98.76");
        assert_eq!(float_to_string(1.0e-4), "1.0E-4");
        assert_eq!(float_to_string(1.2345678e7), "1.2345678E7");
        assert_eq!(float_to_string(f32::NAN), "NaN");
        assert_eq!(float_to_string(-0.0), "-0.0");
        assert_eq!(double_to_string(f64::from_bits(1)), "4.9E-324");
        assert_eq!(double_to_string(1.0e7), "1.0E7");
        assert_eq!(double_to_string(0.001), "0.001");
        assert_eq!(double_to_string(f64::MAX), "1.7976931348623157E308");
        assert_eq!(float_to_string(f32::from_bits(1)), "1.4E-45");
        assert_eq!(format_fixed(1.005, 2), "1.01");
        assert_eq!(format_fixed(0.15, 1), "0.2");
        assert_eq!(format_fixed(-0.0001, 3), "-0.000");
        assert_eq!(format_fixed(2.5, 0), "3");
        assert_eq!(format_fixed(1234.0 / 1000.0, 3), "1.234");
        assert_eq!(format!("{}", JavaFloat(0.5)), "0.5");
        assert_eq!(format!("{}", Fixed::new(1.0, 3)), "1.000");
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_double(" 1.5f "), Ok(1.5));
        assert_eq!(parse_double("0x1.8p1"), Ok(3.0));
        assert_eq!(parse_double("-NaN").map(f64::is_nan), Ok(true));
        assert_eq!(parse_double("").unwrap_err().message(), "empty String");
        assert_eq!(
            parse_double("1..2").unwrap_err().message(),
            "multiple points"
        );
        assert_eq!(
            parse_double("abc").unwrap_err().message(),
            "For input string: \"abc\""
        );
        assert_eq!(parse_int("+5"), Ok(5));
        assert_eq!(parse_int("\u{0665}"), Ok(5));
        assert_eq!(
            parse_int("2147483648").unwrap_err().message(),
            "For input string: \"2147483648\""
        );
        assert_eq!(parse_long("-9223372036854775808"), Ok(i64::MIN));
        assert!(parse_boolean("TrUe"));
        assert!(!parse_boolean("yes"));
    }
}
