const REPLACEMENT: char = '\u{FFFD}';

fn is_not_continuation(b: u8) -> bool {
    b & 0xc0 != 0x80
}

fn is_malformed3_2(b1: u8, b2: u8) -> bool {
    (b1 == 0xe0 && (b2 & 0xe0) == 0x80) || (b2 & 0xc0) != 0x80
}

fn is_malformed4_2(b1: u8, b2: u8) -> bool {
    (b1 == 0xf0 && !(0x90..=0xbf).contains(&b2))
        || (b1 == 0xf4 && (b2 & 0xf0) != 0x80)
        || (b2 & 0xc0) != 0x80
}

pub(crate) fn decode(bytes: &[u8]) -> String {
    // Wire compatibility with the JDK: an encoded surrogate is one U+FFFD, not three.
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }

    let n = bytes.len();

    let mut out = String::with_capacity(n);
    let mut i = 0;
    while i < n {
        let b1 = bytes[i];
        let rem = n - i;

        if b1 < 0x80 {
            out.push(b1 as char);
            i += 1;
        } else if (b1 >> 5) == 0b110 && (b1 & 0x1e) != 0 {
            if rem < 2 {
                out.push(REPLACEMENT);
                break;
            }
            let b2 = bytes[i + 1];
            if is_not_continuation(b2) {
                out.push(REPLACEMENT);
                i += 1;
            } else {
                let c = (u32::from(b1 & 0x1f) << 6) | u32::from(b2 & 0x3f);
                out.push(char::from_u32(c).unwrap_or(REPLACEMENT));
                i += 2;
            }
        } else if (b1 >> 4) == 0b1110 {
            if rem < 3 {
                if rem > 1 && is_malformed3_2(b1, bytes[i + 1]) {
                    out.push(REPLACEMENT);
                    i += 1;
                    continue;
                }
                out.push(REPLACEMENT);
                break;
            }
            let (b2, b3) = (bytes[i + 1], bytes[i + 2]);
            if is_malformed3_2(b1, b2) || is_not_continuation(b3) {
                let len = if (b1 == 0xe0 && (b2 & 0xe0) == 0x80) || is_not_continuation(b2) {
                    1
                } else {
                    2
                };
                out.push(REPLACEMENT);
                i += len;
                continue;
            }
            let c =
                (u32::from(b1 & 0x0f) << 12) | (u32::from(b2 & 0x3f) << 6) | u32::from(b3 & 0x3f);
            match char::from_u32(c) {
                Some(ch) => out.push(ch),
                None => out.push(REPLACEMENT),
            }
            i += 3;
        } else if (b1 >> 3) == 0b11110 {
            if rem < 4 {
                if b1 > 0xf4 || (rem > 1 && is_malformed4_2(b1, bytes[i + 1])) {
                    out.push(REPLACEMENT);
                    i += 1;
                    continue;
                }
                if rem > 2 && is_not_continuation(bytes[i + 2]) {
                    out.push(REPLACEMENT);
                    i += 2;
                    continue;
                }
                out.push(REPLACEMENT);
                break;
            }
            let (b2, b3, b4) = (bytes[i + 1], bytes[i + 2], bytes[i + 3]);
            let uc = (u32::from(b1 & 0x07) << 18)
                | (u32::from(b2 & 0x3f) << 12)
                | (u32::from(b3 & 0x3f) << 6)
                | u32::from(b4 & 0x3f);
            let malformed = is_not_continuation(b2)
                || is_not_continuation(b3)
                || is_not_continuation(b4)
                || !(0x1_0000..=0x10_ffff).contains(&uc);
            if malformed {
                let len = if b1 > 0xf4
                    || (b1 == 0xf0 && !(0x90..=0xbf).contains(&b2))
                    || (b1 == 0xf4 && (b2 & 0xf0) != 0x80)
                    || is_not_continuation(b2)
                {
                    1
                } else if is_not_continuation(b3) {
                    2
                } else {
                    3
                };
                out.push(REPLACEMENT);
                i += len;
                continue;
            }
            out.push(char::from_u32(uc).unwrap_or(REPLACEMENT));
            i += 4;
        } else {
            out.push(REPLACEMENT);
            i += 1;
        }
    }
    out
}
