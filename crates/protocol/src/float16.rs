const SIGN_SHIFT: u32 = 15;
const EXPONENT_SHIFT: u32 = 10;
const SIGN_MASK: i32 = 0x8000;
const SHIFTED_EXPONENT_MASK: i32 = 0x1f;
const SIGNIFICAND_MASK: i32 = 0x3ff;
const EXPONENT_BIAS: i32 = 15;
const FP32_SIGN_SHIFT: u32 = 31;
const FP32_EXPONENT_SHIFT: u32 = 23;
const FP32_SHIFTED_EXPONENT_MASK: i32 = 0xff;
const FP32_SIGNIFICAND_MASK: i32 = 0x7f_ffff;
const FP32_EXPONENT_BIAS: i32 = 127;
const FP32_QNAN_MASK: i32 = 0x40_0000;
const FP32_DENORMAL_MAGIC: i32 = 126 << 23;

pub fn half_to_float(h: u16) -> f32 {
    let bits = i32::from(h);
    let s = bits & SIGN_MASK;
    let e = (bits >> EXPONENT_SHIFT) & SHIFTED_EXPONENT_MASK;
    let m = bits & SIGNIFICAND_MASK;
    let mut out_e = 0;
    let mut out_m = 0;
    if e == 0 {
        if m != 0 {
            let denormal_float = f32::from_bits(FP32_DENORMAL_MAGIC as u32);
            let mut o = f32::from_bits((FP32_DENORMAL_MAGIC + m) as u32);
            o -= denormal_float;
            return if s == 0 { o } else { -o };
        }
    } else {
        out_m = m << 13;
        if e == 0x1f {
            out_e = 0xff;
            if out_m != 0 {
                out_m |= FP32_QNAN_MASK;
            }
        } else {
            out_e = e - EXPONENT_BIAS + FP32_EXPONENT_BIAS;
        }
    }
    let out = ((s as u32) << 16) | ((out_e as u32) << FP32_EXPONENT_SHIFT) | (out_m as u32);
    f32::from_bits(out)
}

pub fn float_to_half(f: f32) -> u16 {
    let bits = f.to_bits() as i32;
    let s = ((bits as u32) >> FP32_SIGN_SHIFT) as i32;
    let mut e = (bits >> FP32_EXPONENT_SHIFT) & FP32_SHIFTED_EXPONENT_MASK;
    let mut m = bits & FP32_SIGNIFICAND_MASK;
    let mut out_e = 0;
    let mut out_m = 0;
    if e == 0xff {
        out_e = 0x1f;
        // Every NaN becomes the same quiet pattern whatever its payload.
        out_m = if m != 0 { 0x200 } else { 0 };
    } else {
        e = e - FP32_EXPONENT_BIAS + EXPONENT_BIAS;
        if e >= 0x1f {
            out_e = 0x1f;
        } else if e <= 0 {
            if e >= -10 {
                m |= 0x80_0000;
                let shift = 14 - e;
                out_m = m >> shift;
                let lowm = m & ((1 << shift) - 1);
                let hway = 1 << (shift - 1);

                if lowm + (out_m & 1) > hway {
                    out_m += 1;
                }
            }
        } else {
            out_e = e;
            out_m = m >> 13;
            if (m & 0x1fff) + (out_m & 0x1) > 0x1000 {
                out_m += 1;
            }
        }
    }

    // The + (not |) lets a rounding carry spill into the exponent.
    ((s << SIGN_SHIFT) | ((out_e << EXPONENT_SHIFT) + out_m)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(float_to_half(1.0), 0x3c00);
        assert_eq!(float_to_half(-2.0), 0xc000);
        assert_eq!(float_to_half(0.0), 0);
        assert_eq!(float_to_half(-0.0), 0x8000);
        assert_eq!(float_to_half(65504.0), 0x7bff);
        assert_eq!(float_to_half(f32::INFINITY), 0x7c00);
        assert_eq!(float_to_half(f32::NAN), 0x7e00);
        assert_eq!(half_to_float(0x3c00), 1.0);
        assert_eq!(half_to_float(0x0001), 5.960_464_5e-8);
        assert!(half_to_float(0x7c01).is_nan());
    }
}
