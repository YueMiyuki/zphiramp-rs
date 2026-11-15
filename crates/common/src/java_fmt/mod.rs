use std::fmt::{self, LowerExp};
use std::str::FromStr;

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
    push_java(
        out,
        v,
        v.is_nan(),
        v.is_infinite(),
        v.is_sign_negative(),
        v.abs(),
    );
}

pub fn push_double(out: &mut String, v: f64) {
    push_java(
        out,
        v,
        v.is_nan(),
        v.is_infinite(),
        v.is_sign_negative(),
        v.abs(),
    );
}

fn push_java<T: Shortest>(
    out: &mut String,
    v: T,
    nan: bool,
    infinite: bool,
    negative: bool,
    abs: T,
) {
    if nan {
        out.push_str("NaN");
        return;
    }
    if negative {
        out.push('-');
    }
    if infinite {
        out.push_str("Infinity");
    } else if v == T::ZERO {
        out.push_str("0.0");
    } else {
        layout(out, &shortest(abs));
    }
}

trait Shortest: ryu::Float + LowerExp + FromStr + PartialOrd {
    const ZERO: Self;
}

impl Shortest for f32 {
    const ZERO: Self = 0.0;
}

impl Shortest for f64 {
    const ZERO: Self = 0.0;
}

struct Decimal {
    digits: Vec<u8>,
    exp: i32,
}

fn parse_exp(text: &str) -> Decimal {
    let (mantissa, exp) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let exp: i32 = exp.parse().expect("decimal exponent");
    let point = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
    let all: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    let lead = all
        .iter()
        .take_while(|&&d| d == 0)
        .count()
        .min(all.len() - 1);
    let mut digits = all[lead..].to_vec();
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
    Decimal {
        digits,
        exp: exp + point - 1 - lead as i32,
    }
}

fn shortest<T: Shortest>(v: T) -> Decimal {
    // ryu rather than `{:e}`: an exact tie between two shortest candidates takes the even digit.
    let d = parse_exp(ryu::Buffer::new().format_finite(v));
    if d.digits.len() > 1 {
        return d;
    }
    // Never fewer than two significant digits: the closest two-digit decimal that reads back.
    let reads_back = |text: &str| text.parse::<T>().ok() == Some(v);
    let near = format!("{v:.1e}");
    if reads_back(&near) {
        return parse_exp(&near);
    }
    let n = parse_exp(&near);
    let m = i32::from(n.digits[0]) * 10 + i32::from(*n.digits.get(1).unwrap_or(&0));
    let above = near.parse::<T>().is_ok_and(|x| x > v);
    let (m, exp) = match (above, m) {
        (true, 10) => (99, n.exp - 1),
        (true, m) => (m - 1, n.exp),
        (false, 99) => (10, n.exp + 1),
        (false, m) => (m + 1, n.exp),
    };
    let other = format!("{}.{}e{exp}", m / 10, m % 10);
    if reads_back(&other) {
        parse_exp(&other)
    } else {
        d
    }
}

fn layout(out: &mut String, d: &Decimal) {
    let mut digits: Vec<char> = d.digits.iter().map(|&b| char::from(b'0' + b)).collect();
    while digits.len() > 1 && digits.last() == Some(&'0') {
        digits.pop();
    }
    let e = d.exp;
    if (0..7).contains(&e) {
        let int_len = e as usize + 1;
        for i in 0..int_len {
            out.push(*digits.get(i).unwrap_or(&'0'));
        }
        out.push('.');
        if digits.len() > int_len {
            out.extend(&digits[int_len..]);
        } else {
            out.push('0');
        }
    } else if (-3..0).contains(&e) {
        out.push_str("0.");
        for _ in 0..(-e - 1) {
            out.push('0');
        }
        out.extend(&digits);
    } else {
        out.push(digits[0]);
        out.push('.');
        if digits.len() > 1 {
            out.extend(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        out.push_str(&e.to_string());
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
        assert_eq!(format!("{}", JavaFloat(0.5)), "0.5");
    }
}
