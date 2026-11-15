use super::g_table::{G, GE_MAX, GE_MIN};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decimal {
    pub f: u64,
    pub e: i32,
    pub len: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Split {
    Finite(Decimal),
    Zero,
    Infinite,
    NaN,
}

const Q_10: i64 = 41;
const C_10: i64 = 661_971_961_083;
const A_10: i64 = -274_743_187_321;
const Q_2: i64 = 38;
const C_2: i64 = 913_124_641_741;

const POW10: [u64; 20] = [
    1,
    10,
    100,
    1_000,
    10_000,
    100_000,
    1_000_000,
    10_000_000,
    100_000_000,
    1_000_000_000,
    10_000_000_000,
    100_000_000_000,
    1_000_000_000_000,
    10_000_000_000_000,
    100_000_000_000_000,
    1_000_000_000_000_000,
    10_000_000_000_000_000,
    100_000_000_000_000_000,
    1_000_000_000_000_000_000,
    10_000_000_000_000_000_000,
];

pub(crate) fn pow10(e: i32) -> u64 {
    POW10[e as usize]
}

fn flog10pow2(q: i32) -> i32 {
    ((q as i64 * C_10) >> Q_10) as i32
}

fn flog10_three_quarters_pow2(q: i32) -> i32 {
    ((q as i64 * C_10 + A_10) >> Q_10) as i32
}

fn flog2pow10(e: i32) -> i32 {
    ((e as i64 * C_2) >> Q_2) as i32
}

fn g1(e: i32) -> i64 {
    debug_assert!((GE_MIN..=GE_MAX).contains(&e));
    G[(e - GE_MIN) as usize].0 as i64
}

fn g0(e: i32) -> i64 {
    G[(e - GE_MIN) as usize].1 as i64
}

fn multiply_high(x: i64, y: i64) -> i64 {
    ((x as i128 * y as i128) >> 64) as i64
}

fn decimal_len_u64(f: i64) -> i32 {
    let mut len = flog10pow2(64 - f.leading_zeros() as i32);
    if f as u64 >= pow10(len) {
        len += 1;
    }
    len
}

mod double {
    pub const P: i32 = 53;
    pub const W: i32 = 11;
    pub const Q_MIN: i32 = (-1 << (W - 1)) - P + 3;
    pub const C_MIN: i64 = 1 << (P - 1);
    pub const C_TINY: i64 = 3;
    pub const BQ_MASK: i32 = (1 << W) - 1;
    pub const T_MASK: i64 = (1 << (P - 1)) - 1;
    pub const MASK_63: i64 = i64::MAX;
}

pub(crate) fn split_double(v: f64) -> Split {
    use double::*;
    let bits = v.to_bits() as i64;
    let t = bits & T_MASK;
    let bq = ((bits as u64 >> (P - 1)) as i32) & BQ_MASK;
    if bq < BQ_MASK {
        if bq != 0 {
            let mq = -Q_MIN + 1 - bq;
            let c = C_MIN | t;
            if 0 < mq && mq < P {
                let f = c >> mq;
                if f << mq == c {
                    return Split::Finite(to_decimal_result(f, 0));
                }
            }
            return Split::Finite(double_to_decimal(-mq, c, 0));
        }
        if t != 0 {
            return Split::Finite(if t < C_TINY {
                double_to_decimal(Q_MIN, 10 * t, -1)
            } else {
                double_to_decimal(Q_MIN, t, 0)
            });
        }
        return Split::Zero;
    }
    if t != 0 {
        return Split::NaN;
    }
    Split::Infinite
}

fn double_to_decimal(q: i32, c: i64, dk: i32) -> Decimal {
    use double::*;
    let out = c & 0x1;
    let cb = c << 2;
    let cbr = cb + 2;
    let (cbl, k) = if c != C_MIN || q == Q_MIN {
        (cb - 2, flog10pow2(q))
    } else {
        (cb - 1, flog10_three_quarters_pow2(q))
    };
    let h = q + flog2pow10(-k) + 2;

    let g1 = g1(-k);
    let g0 = g0(-k);

    let vb = rop_double(g1, g0, cb << h);
    let vbl = rop_double(g1, g0, cbl << h);
    let vbr = rop_double(g1, g0, cbr << h);

    let s = vb >> 2;
    if s >= 100 {
        let sp10 = 10 * multiply_high(s, 115_292_150_460_684_698_i64 << 4);
        let tp10 = sp10 + 10;
        let upin = vbl + out <= sp10 << 2;
        let wpin = (tp10 << 2) + out <= vbr;
        if upin != wpin {
            return to_decimal_result(if upin { sp10 } else { tp10 }, k);
        }
    }

    let t = s + 1;
    let uin = vbl + out <= s << 2;
    let win = (t << 2) + out <= vbr;
    if uin != win {
        return to_decimal_result(if uin { s } else { t }, k + dk);
    }
    let cmp = vb - ((s + t) << 1);
    let away = cmp > 0 || (cmp == 0 && (s & 0x1) != 0);
    to_decimal_result(if away { t } else { s }, k + dk)
}

// Wrapping arithmetic and unsigned shifts are intentional; the chosen digits depend on them.
fn rop_double(g1: i64, g0: i64, cp: i64) -> i64 {
    use double::MASK_63;
    let x1 = multiply_high(g0, cp);
    let y0 = g1.wrapping_mul(cp);
    let y1 = multiply_high(g1, cp);
    let z = ((y0 as u64 >> 1) as i64).wrapping_add(x1);
    let vbp = y1.wrapping_add((z as u64 >> 63) as i64);
    vbp | (((z & MASK_63).wrapping_add(MASK_63)) as u64 >> 63) as i64
}

fn to_decimal_result(f: i64, e: i32) -> Decimal {
    Decimal {
        f: f as u64,
        e,
        len: decimal_len_u64(f),
    }
}

mod float {
    pub const P: i32 = 24;
    pub const W: i32 = 8;
    pub const Q_MIN: i32 = (-1 << (W - 1)) - P + 3;
    pub const C_MIN: i32 = 1 << (P - 1);
    pub const C_TINY: i32 = 8;
    pub const BQ_MASK: i32 = (1 << W) - 1;
    pub const T_MASK: i32 = (1 << (P - 1)) - 1;
    pub const MASK_32: i64 = (1_i64 << 32) - 1;
}

pub(crate) fn split_float(v: f32) -> Split {
    use float::*;
    let bits = v.to_bits() as i32;
    let t = bits & T_MASK;
    let bq = ((bits as u32 >> (P - 1)) as i32) & BQ_MASK;
    if bq < BQ_MASK {
        if bq != 0 {
            let mq = -Q_MIN + 1 - bq;
            let c = C_MIN | t;
            if 0 < mq && mq < P {
                let f = c >> mq;
                if f << mq == c {
                    return Split::Finite(to_decimal_result(f as i64, 0));
                }
            }
            return Split::Finite(float_to_decimal(-mq, c, 0));
        }
        if t != 0 {
            return Split::Finite(if t < C_TINY {
                float_to_decimal(Q_MIN, 10 * t, -1)
            } else {
                float_to_decimal(Q_MIN, t, 0)
            });
        }
        return Split::Zero;
    }
    if t != 0 {
        return Split::NaN;
    }
    Split::Infinite
}

fn float_to_decimal(q: i32, c: i32, dk: i32) -> Decimal {
    use float::*;
    let out = c & 0x1;
    let cb = (c << 2) as i64;
    let cbr = cb + 2;
    let (cbl, k) = if c != C_MIN || q == Q_MIN {
        (cb - 2, flog10pow2(q))
    } else {
        (cb - 1, flog10_three_quarters_pow2(q))
    };
    let h = q + flog2pow10(-k) + 33;

    let g = g1(-k) + 1;

    let vb = rop_float(g, cb << h);
    let vbl = rop_float(g, cbl << h);
    let vbr = rop_float(g, cbr << h);

    let s = vb >> 2;
    if s >= 100 {
        let sp10 = 10 * ((s as i64 * 1_717_986_919_i64) as u64 >> 34) as i32;
        let tp10 = sp10 + 10;
        let upin = vbl + out <= sp10 << 2;
        let wpin = (tp10 << 2) + out <= vbr;
        if upin != wpin {
            return to_decimal_result(if upin { sp10 } else { tp10 } as i64, k);
        }
    }

    let t = s + 1;
    let uin = vbl + out <= s << 2;
    let win = (t << 2) + out <= vbr;
    if uin != win {
        return to_decimal_result(if uin { s } else { t } as i64, k + dk);
    }
    let cmp = vb - ((s + t) << 1);
    let away = cmp > 0 || (cmp == 0 && (s & 0x1) != 0);
    to_decimal_result(if away { t } else { s } as i64, k + dk)
}

fn rop_float(g: i64, cp: i64) -> i32 {
    use float::MASK_32;
    let x1 = multiply_high(g, cp);
    let vbp = (x1 as u64 >> 31) as i64;
    (vbp | (((x1 & MASK_32) + MASK_32) as u64 >> 32) as i64) as i32
}

pub(crate) fn layout(out: &mut String, d: Decimal) {
    let mut digits = d.f.to_string().into_bytes();
    debug_assert_eq!(digits.len() as i32, d.len);
    while digits.len() > 1 && *digits.last().unwrap() == b'0' {
        digits.pop();
    }
    let e10 = d.e + d.len;
    if 0 < e10 && e10 <= 7 {
        let e = e10 as usize;
        for i in 0..e {
            out.push(*digits.get(i).unwrap_or(&b'0') as char);
        }
        out.push('.');
        if digits.len() > e {
            for &b in &digits[e..] {
                out.push(b as char);
            }
        } else {
            out.push('0');
        }
    } else if -3 < e10 && e10 <= 0 {
        out.push_str("0.");
        for _ in e10..0 {
            out.push('0');
        }
        for &b in &digits {
            out.push(b as char);
        }
    } else {
        out.push(digits[0] as char);
        out.push('.');
        if digits.len() > 1 {
            for &b in &digits[1..] {
                out.push(b as char);
            }
        } else {
            out.push('0');
        }
        out.push('E');
        let exp = e10 - 1;
        out.push_str(&exp.to_string());
    }
}

pub(crate) fn plain_mantissa(v: f64, prec: i64) -> String {
    let (mut f, mut e, mut n) = match split_double(v) {
        Split::Finite(d) => (d.f, d.e, d.len),
        Split::Zero => (0, 0, 0),
        Split::Infinite | Split::NaN => unreachable!("caller handles non-finite values"),
    };

    let pp = n as i64 + e as i64 + prec;
    if n == 0 || pp < 0 {
        f = 0;
        e = 0;
        n = 1;
    } else if pp < n as i64 {
        let p = pp as i32;
        e += n - p;
        let pow = pow10(n - p);
        f = (f + (pow >> 1)) / pow;
        if p == 0 {
            n = 1;
            if f == 0 {
                e = 0;
            }
        } else {
            n = p;
            if f == pow10(p) {
                f /= 10;
                e += 1;
            }
        }
    }

    let fd = format!("{f:0width$}", width = n as usize);
    let mut s = String::new();
    if e >= 0 {
        s.push_str(&fd);
        for _ in 0..e {
            s.push('0');
        }
    } else if n + e > 0 {
        let split = (n + e) as usize;
        s.push_str(&fd[..split]);
        s.push('.');
        s.push_str(&fd[split..]);
    } else {
        s.push_str("0.");
        for _ in 0..(-e - n) {
            s.push('0');
        }
        s.push_str(&fd);
    }
    s
}
