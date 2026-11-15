use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberFormatError {
    message: String,
}

impl NumberFormatError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn for_input_string(s: &str) -> Self {
        Self::new(format!("For input string: \"{s}\""))
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for NumberFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NumberFormatError {}

const DECIMAL_DIGIT_ZEROS: [u32; 37] = [
    0x0030, 0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6,
    0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80,
    0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0,
    0xFF10,
];

fn java_decimal_digit(c: char) -> Option<u32> {
    let cp = c as u32;
    if cp > 0xFFFF {
        return None;
    }
    DECIMAL_DIGIT_ZEROS
        .iter()
        .find(|&&zero| (zero..zero + 10).contains(&cp))
        .map(|&zero| cp - zero)
}

/// Accepts what Java's `Integer.parseInt` accepts, including non-ASCII decimal digits.
pub fn parse_int(s: &str) -> Result<i32, NumberFormatError> {
    parse_integral(s, i32::MIN as i64, i32::MAX as i64).map(|v| v as i32)
}

pub fn parse_long(s: &str) -> Result<i64, NumberFormatError> {
    parse_integral(s, i64::MIN, i64::MAX)
}

fn parse_integral(s: &str, min: i64, max: i64) -> Result<i64, NumberFormatError> {
    let mut chars = s.chars();
    let first = chars
        .next()
        .ok_or_else(|| NumberFormatError::for_input_string(""))?;
    let (negative, mut digits_seen, mut value): (bool, bool, i128) = match first {
        '-' => (true, false, 0),
        '+' => (false, false, 0),
        c => match java_decimal_digit(c) {
            Some(d) => (false, true, d as i128),
            None => return Err(NumberFormatError::for_input_string(s)),
        },
    };
    for c in chars {
        let d = java_decimal_digit(c).ok_or_else(|| NumberFormatError::for_input_string(s))?;
        value = value * 10 + d as i128;
        digits_seen = true;
        if value > max as i128 + 1 {
            return Err(NumberFormatError::for_input_string(s));
        }
    }
    if !digits_seen {
        return Err(NumberFormatError::for_input_string(s));
    }
    let signed = if negative { -value } else { value };
    if signed < min as i128 || signed > max as i128 {
        return Err(NumberFormatError::for_input_string(s));
    }
    Ok(signed as i64)
}

pub fn parse_boolean(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

fn is_java_space(c: char) -> bool {
    c <= ' '
}

pub fn parse_double(s: &str) -> Result<f64, NumberFormatError> {
    let check = |ok: bool| {
        if ok {
            Ok(())
        } else {
            Err(java_double_error(s))
        }
    };
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let skip_ws = |mut i: usize| {
        while i < len && is_java_space(chars[i]) {
            i += 1;
        }
        i
    };

    let mut i = skip_ws(0);
    if i == len {
        return Err(NumberFormatError::new("empty String"));
    }
    let mut negative = false;
    if chars[i] == '-' || chars[i] == '+' {
        negative = chars[i] == '-';
        i += 1;
    }

    let lower = |c: char| -> char {
        if c.is_ascii() {
            ((c as u8) | 0x20) as char
        } else {
            c
        }
    };

    let mut is_dec = true;
    if i < len {
        let ch = chars[i];
        if ch == 'I' || ch == 'N' {
            let sub = if ch == 'I' { "Infinity" } else { "NaN" };
            let end = i + sub.chars().count();
            check(
                end <= len && chars[i..end].iter().copied().eq(sub.chars()) && skip_ws(end) == len,
            )?;
            return Ok(if ch == 'N' {
                f64::NAN
            } else if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
        }
        if ch == '0' && i + 1 < len && lower(chars[i + 1]) == 'x' {
            is_dec = false;
            i += 2;
        }
    }

    let is_digit = |c: char| c.is_ascii_digit() || (!is_dec && ('a'..='f').contains(&lower(c)));

    let mut point: Option<usize> = None;
    let mut digits = String::new();
    let start = i;
    while i < len && (is_digit(chars[i]) || chars[i] == '.') {
        if chars[i] == '.' {
            if point.is_some() {
                return Err(NumberFormatError::new("multiple points"));
            }
            point = Some(digits.len());
        } else {
            digits.push(chars[i]);
        }
        i += 1;
    }
    check(i - start > usize::from(point.is_some()))?;
    let stop = i;

    let mut exp: i64 = 0;
    let mut has_exp = false;
    if i < len {
        let ch = lower(chars[i]);
        if (ch == 'e' && is_dec) || (ch == 'p' && !is_dec) {
            i += 1;
            let mut exp_sign = ' ';
            if i < len && (chars[i] == '-' || chars[i] == '+') {
                exp_sign = chars[i];
                i += 1;
            }
            while i < len && chars[i].is_ascii_digit() {
                let d = chars[i] as i64 - '0' as i64;
                exp = if exp < 10_000_000_000 / 10 {
                    10 * exp + d
                } else {
                    10_000_000_000
                };
                i += 1;
            }
            check(i - stop >= 3 || (i - stop == 2 && exp_sign == ' '))?;
            if exp_sign == '-' {
                exp = -exp;
            }
            has_exp = true;
        }
    }
    check(is_dec || has_exp)?;
    if i < len && matches!(lower(chars[i]), 'f' | 'd') {
        i += 1;
    }
    check(skip_ws(i) == len)?;

    let point = point.unwrap_or(digits.len());
    let value = if is_dec {
        let trimmed = digits.trim_start_matches('0');
        if trimmed.trim_end_matches('0').is_empty() {
            0.0
        } else {
            let scale = exp - (digits.len() - point) as i64;
            let scale = scale.clamp(-100_000_000, 100_000_000);
            format!("{trimmed}e{scale}")
                .parse::<f64>()
                .expect("normalized decimal literal always parses")
        }
    } else {
        hex_to_double(&digits, point, exp)
    };
    Ok(if negative { -value } else { value })
}

fn java_double_error(s: &str) -> NumberFormatError {
    const MAX_OUT: usize = 1_000;
    const OMITTED: &str = " ... ";
    let units: Vec<u16> = s.encode_utf16().collect();
    let shown = if units.len() > MAX_OUT {
        let l_half = (MAX_OUT - OMITTED.len()) / 2;
        let r_half = MAX_OUT - (l_half + OMITTED.len());
        let left = String::from_utf16_lossy(&units[..l_half]);
        let right = String::from_utf16_lossy(&units[units.len() - r_half..]);
        format!("{left}{OMITTED}{right}")
    } else {
        s.to_owned()
    };
    NumberFormatError::for_input_string(&shown)
}

fn hex_to_double(digits: &str, point: usize, bin_exp: i64) -> f64 {
    const P: i32 = 53;
    const Q_MIN: i64 = -1074;
    const QP_MIN: i64 = Q_MIN + P as i64;
    const QP_MAX: i64 = 3 - QP_MIN;
    const HEX_COUNT: usize = 15;

    let bytes = digits.as_bytes();
    let lz = bytes.iter().take_while(|&&b| b == b'0').count();
    let tnz = bytes.iter().rposition(|&b| b != b'0').map_or(0, |p| p + 1);
    if tnz == 0 {
        return 0.0;
    }
    let sig = &bytes[lz..tnz];
    let n = sig.len();
    let mut ep = bin_exp + 4 * (point as i64 - tnz as i64);
    let le = n.min(HEX_COUNT);
    let mut c: i64 = 0;
    for &b in &sig[..le] {
        c = (c << 4) | (b as char).to_digit(16).expect("hex digit") as i64;
    }
    if n > le {
        c |= 1;
        ep += 4 * (n - le) as i64;
    }
    let bl = 64 - c.leading_zeros() as i64;
    if ep < Q_MIN - bl {
        return 0.0;
    }
    if ep > QP_MAX - bl {
        return f64::INFINITY;
    }
    let mut q = ep;
    let shr;
    if q >= QP_MIN - bl {
        shr = bl - P as i64;
        q += shr;
    } else {
        shr = Q_MIN - q;
        q = Q_MIN;
    }
    if shr > 0 {
        let thr = 1_i64 << shr;
        let tail = (c & (thr - 1)) << 1;
        c = (c as u64 >> shr) as i64;
        if tail > thr || (tail == thr && (c & 1) != 0) {
            c += 1;
            if c >= 1_i64 << P {
                c = (c as u64 >> 1) as i64;
                q += 1;
            }
        }
    } else {
        c <<= -shr;
    }
    let be: i64 = if c < 1_i64 << (P - 1) {
        0
    } else {
        q + (1023 - 1) + P as i64
    };
    let bits = ((be as u64) << (P - 1)) | (c as u64 & ((1_u64 << (P - 1)) - 1));
    f64::from_bits(bits)
}
