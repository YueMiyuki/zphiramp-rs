mod common;

use common::*;
use zphira_protocol::float16::{float_to_half, half_to_float};
use zphira_protocol::wire::{
    decode_string, decode_varint, encode_string, encode_varint, varint_len,
};
use zphira_protocol::{ProtocolError, encode_frame};

#[test]
fn varints() {
    let (mut enc, mut dec, mut frame) = (0, 0, 0);
    for row in rows("varint.tsv") {
        match row[0].as_str() {
            "enc" => {
                let v: i32 = row[1].parse().unwrap();
                let mut b = Vec::new();
                encode_varint(&mut b, v);
                assert_eq!(hex(&b), row[2], "encode {v}");
                assert_eq!(varint_len(v), b.len(), "len {v}");
                enc += 1;
            }
            "dec" => {
                let input = unhex(&row[1]);
                let mut buf = &input[..];
                let got = decode_varint(&mut buf);
                let want = &row[2];
                if want == "err" {
                    let err = got.expect_err(&row[1]);
                    assert_eq!(err.java_exception(), Some(row[3].as_str()), "{}", row[1]);
                    assert_eq!(err.to_string(), row[4], "{}", row[1]);
                } else {
                    let (value, consumed) = want.split_once(':').unwrap();
                    assert_eq!(
                        got.unwrap_or_else(|e| panic!("{}: {e}", row[1])),
                        value.parse::<i32>().unwrap(),
                        "{}",
                        row[1]
                    );
                    assert_eq!(
                        input.len() - buf.len(),
                        consumed.parse::<usize>().unwrap(),
                        "consumed {}",
                        row[1]
                    );
                }
                dec += 1;
            }
            "frame" => {
                let len: usize = row[1].parse().unwrap();
                let mut out = Vec::new();
                encode_frame(&vec![0u8; len], &mut out).unwrap();
                let prefix = unhex(&row[2]);
                assert_eq!(&out[..prefix.len()], &prefix[..], "frame prefix for {len}");
                assert_eq!(out.len(), prefix.len() + len);
                frame += 1;
            }
            other => panic!("unknown kind {other}"),
        }
    }
    eprintln!("{enc} varint encodes, {dec} decodes, {frame} frame prefixes");
}

#[test]
fn strings() {
    let mut n = 0;
    for row in rows("strings.tsv") {
        match row[0].as_str() {
            "enc" => {
                let text = String::from_utf8(unhex(&row[1])).unwrap();
                let mut b = Vec::new();
                encode_string(&mut b, &text);
                assert_eq!(hex(&b), row[2], "encode {text:?}");

                let back = decode_string(&mut &b[..], 131_072).unwrap();
                assert_eq!(back, text);
            }
            "dec" => {
                let max: i32 = row[1].parse().unwrap();
                let input = unhex(&row[2]);
                let mut buf = &input[..];
                let got = decode_string(&mut buf, max);
                let what = format!("max {max} input {}", row[2]);
                if row[3] == "err" {
                    let err = got.expect_err(&what);
                    assert_eq!(err.java_exception(), Some(row[4].as_str()), "{what}");
                    assert_eq!(err.to_string(), row[5], "{what}");
                } else {
                    let (text_hex, consumed) = row[4].split_once(':').unwrap();
                    let text = got.unwrap_or_else(|e| panic!("{what}: {e}"));
                    assert_eq!(hex(text.as_bytes()), text_hex, "{what}");
                    assert_eq!(
                        input.len() - buf.len(),
                        consumed.parse::<usize>().unwrap(),
                        "consumed {what}"
                    );
                }
            }
            "len" => {
                let n: usize = row[1].parse().unwrap();
                let mut b = Vec::new();
                encode_string(&mut b, &"a".repeat(n));
                assert_eq!(hex(&b[..4]), row[2], "length prefix of {n} chars");
            }
            "big" => {
                let (len, max) = match row[1].as_str() {
                    "default_max_ok" => (131_072, 131_072),
                    "default_max_plus_one" => (131_073, 131_072),
                    "short_max_ok" => (32_767, 32_767),
                    "short_max_plus_one" => (32_768, 32_767),
                    other => panic!("unknown big case {other}"),
                };
                let mut b = Vec::new();
                encode_varint(&mut b, len);
                b.extend(std::iter::repeat_n(b'a', len as usize));
                let got = match decode_string(&mut &b[..], max) {
                    Ok(s) => format!("ok:{}", s.len()),
                    Err(e) => format!("err:{}", code(&e)),
                };
                assert_eq!(got, row[2], "{}", row[1]);
            }
            other => panic!("unknown kind {other}"),
        }
        n += 1;
    }
    eprintln!("{n} string vectors");
}

#[test]
fn utf8_replacement_matches_the_jdk() {
    let rows = rows("utf8_lossy.tsv");
    let mut differs_from_rust_lossy = 0;
    for row in &rows {
        let input = unhex(&row[0]);
        let mut framed = Vec::new();
        encode_varint(&mut framed, input.len() as i32);
        framed.extend_from_slice(&input);
        let got = decode_string(&mut &framed[..], 131_072).unwrap();
        assert_eq!(hex(got.as_bytes()), row[1], "input {}", row[0]);
        if got != String::from_utf8_lossy(&input) {
            differs_from_rust_lossy += 1;
        }
    }

    assert!(differs_from_rust_lossy > 0);
    eprintln!(
        "{} inputs, {differs_from_rust_lossy} of them decode differently with from_utf8_lossy",
        rows.len()
    );
}

#[test]
fn half_to_float_is_exhaustively_exact() {
    let words: Vec<String> = lines("float16_decode.txt");
    let mut i = 0u32;
    for line in &words {
        assert_eq!(line.len(), 256 * 8);
        for chunk in line.as_bytes().chunks(8) {
            let want = u32::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
            assert_eq!(half_to_float(i as u16).to_bits(), want, "half {i:#06x}");
            i += 1;
        }
    }
    assert_eq!(i, 65536);
}

const MANTISSAS: [u32; 32] = [
    0,
    1,
    0xfff,
    0x1000,
    0x1001,
    0x1fff,
    0x2000,
    0x2001,
    0x3000,
    0x3fff,
    0x400000,
    0x400001,
    0x7fffff,
    0x7ff000,
    0x7fefff,
    0x7ffe00,
    0x200,
    0x201,
    0x1ff,
    0x555555,
    0x2aaaaa,
    0x123456,
    0x7f0000,
    0x00ffff,
    0x010000,
    0x008000,
    0x004000,
    0x000800,
    0x000400,
    0x000002,
    0x001000,
    0x7fffff - 0x1000,
];

#[test]
fn float_to_half_matches_java() {
    let mut inputs: Vec<u32> = Vec::new();
    for s in 0..2u32 {
        for e in 0..256u32 {
            for m in MANTISSAS {
                inputs.push((s << 31) | (e << 23) | m);
            }
        }
    }
    let mut r = Rng(0xF16);
    for _ in 0..32768 {
        inputs.push(r.next() as u32);
    }
    let want: String = lines("float16_encode.txt").concat();
    assert_eq!(want.len(), inputs.len() * 4);
    for (i, bits) in inputs.iter().enumerate() {
        let got = float_to_half(f32::from_bits(*bits));
        assert_eq!(
            format!("{got:04x}"),
            want[4 * i..4 * i + 4],
            "float {bits:#010x} (input {i})"
        );
    }
    eprintln!("{} float to half conversions", inputs.len());
}

#[test]
fn string_limit_check_order() {
    assert_eq!(
        decode_string(&mut &[0x05, b'a'][..], 3),
        Err(ProtocolError::BadStringLength { length: 5, max: 3 })
    );
    assert_eq!(
        decode_string(&mut &[0x03, b'a'][..], 3),
        Err(ProtocolError::NeedMoreData)
    );
}
