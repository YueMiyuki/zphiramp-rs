use bytes::{Buf, BufMut};

use crate::error::ProtocolError;
use half::f16;

pub const MAXIMUM_VARINT_SIZE: usize = 5;

/// In UTF-8 bytes, not characters.
pub const MAX_STRING_LENGTH: i32 = 131_072;

pub trait Encode {
    fn encode<B: BufMut>(&self, buf: &mut B);
}

pub trait Decode: Sized {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError>;
}

impl Encode for () {
    fn encode<B: BufMut>(&self, _buf: &mut B) {}
}

impl Decode for () {
    fn decode<B: Buf>(_buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(())
    }
}

#[inline]
fn need<B: Buf>(buf: &B, needed: usize) -> Result<(), ProtocolError> {
    if buf.remaining() < needed {
        Err(ProtocolError::Malformed("truncated"))
    } else {
        Ok(())
    }
}

pub fn get_u8<B: Buf>(buf: &mut B) -> Result<u8, ProtocolError> {
    need(buf, 1)?;
    Ok(buf.get_u8())
}

pub fn get_i8<B: Buf>(buf: &mut B) -> Result<i8, ProtocolError> {
    need(buf, 1)?;
    Ok(buf.get_i8())
}

pub fn get_bool<B: Buf>(buf: &mut B) -> Result<bool, ProtocolError> {
    // Wire compatibility: any non-zero byte is true.
    Ok(get_u8(buf)? != 0)
}

pub fn get_i32_le<B: Buf>(buf: &mut B) -> Result<i32, ProtocolError> {
    need(buf, 4)?;
    Ok(buf.get_i32_le())
}

pub fn get_f32_le<B: Buf>(buf: &mut B) -> Result<f32, ProtocolError> {
    need(buf, 4)?;
    Ok(f32::from_bits(buf.get_u32_le()))
}

pub(crate) fn try_decode_varint<B: Buf>(buf: &mut B) -> Result<Option<i32>, ProtocolError> {
    if !buf.has_remaining() {
        return Ok(None);
    }
    let b = buf.get_u8();
    if b & 0x80 == 0 {
        return Ok(Some(i32::from(b)));
    }
    let mut value = u32::from(b & 0x7f);
    let mut shift = 7;
    for _ in 1..MAXIMUM_VARINT_SIZE {
        if !buf.has_remaining() {
            return Ok(None);
        }
        let b = buf.get_u8();
        // Wire compatibility: surplus high bits of the fifth byte fall off, non-canonical forms pass.
        value |= u32::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok(Some(value as i32));
        }
        shift += 7;
    }
    Err(ProtocolError::Malformed("bad varint"))
}

pub fn decode_varint<B: Buf>(buf: &mut B) -> Result<i32, ProtocolError> {
    try_decode_varint(buf)?.ok_or(ProtocolError::Malformed("truncated"))
}

pub fn encode_varint<B: BufMut>(buf: &mut B, value: i32) {
    let mut v = value as u32;
    while v >= 0x80 {
        buf.put_u8((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    buf.put_u8(v as u8);
}

pub fn varint_len(value: i32) -> usize {
    match value as u32 {
        0..0x80 => 1,
        0x80..0x4000 => 2,
        0x4000..0x20_0000 => 3,
        0x20_0000..0x1000_0000 => 4,
        _ => 5,
    }
}

pub fn decode_string<B: Buf>(buf: &mut B, max_length: i32) -> Result<String, ProtocolError> {
    let length = decode_varint(buf)?;
    if length < 0 {
        return Err(ProtocolError::Malformed("negative string length"));
    }
    if length > max_length {
        return Err(ProtocolError::StringTooLong {
            length,
            max: max_length,
        });
    }
    let length = length as usize;
    if buf.remaining() < length {
        return Err(ProtocolError::Malformed("truncated"));
    }
    let bytes = buf.copy_to_bytes(length);
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn decode_string_default<B: Buf>(buf: &mut B) -> Result<String, ProtocolError> {
    decode_string(buf, MAX_STRING_LENGTH)
}

pub fn encode_string<B: BufMut>(buf: &mut B, s: &str) {
    encode_varint(buf, s.len() as i32);
    buf.put_slice(s.as_bytes());
}

pub fn decode_float16_le<B: Buf>(buf: &mut B) -> Result<f32, ProtocolError> {
    need(buf, 2)?;
    Ok(f16::from_bits(buf.get_u16_le()).to_f32())
}

pub fn encode_float16_le<B: BufMut>(buf: &mut B, value: f32) {
    // Every NaN goes out as the same quiet NaN (sign kept), whatever its payload.
    let bits = if value.is_nan() {
        0x7e00 | ((value.to_bits() >> 16) & 0x8000) as u16
    } else {
        f16::from_f32(value).to_bits()
    };
    buf.put_u16_le(bits);
}

pub const MAX_INITIAL_CAPACITY: usize = 1024;

/// Capacity for a list whose count came off the wire, bounded so a tiny frame cannot force a big allocation.
pub fn initial_capacity(count: usize, remaining: usize, min_element_size: usize) -> usize {
    count
        .min(remaining / min_element_size.max(1))
        .min(MAX_INITIAL_CAPACITY)
}

pub fn decode_list<B: Buf, T>(
    buf: &mut B,
    min_element_size: usize,
    mut decode_element: impl FnMut(&mut B) -> Result<T, ProtocolError>,
) -> Result<Vec<T>, ProtocolError> {
    let count = decode_varint(buf)?;
    if count < 0 {
        return Err(ProtocolError::Malformed("negative list count"));
    }
    let mut list = Vec::with_capacity(initial_capacity(
        count as usize,
        buf.remaining(),
        min_element_size,
    ));
    for _ in 0..count {
        list.push(decode_element(buf)?);
    }
    Ok(list)
}

pub fn encode_list<B: BufMut, T: Encode>(buf: &mut B, list: &[T]) {
    encode_varint(buf, list.len() as i32);
    for element in list {
        element.encode(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(v: i32) -> Vec<u8> {
        let mut b = Vec::new();
        encode_varint(&mut b, v);
        b
    }

    #[test]
    fn varint_roundtrip_and_len() {
        for v in [
            0,
            1,
            127,
            128,
            16383,
            16384,
            2_097_151,
            2_097_152,
            268_435_455,
            268_435_456,
            i32::MAX,
            -1,
            i32::MIN,
        ] {
            let bytes = enc(v);
            assert_eq!(bytes.len(), varint_len(v), "{v}");
            assert_eq!(decode_varint(&mut &bytes[..]).unwrap(), v);
        }
    }

    #[test]
    fn initial_capacity_is_bounded_by_data_and_a_constant() {
        assert_eq!(initial_capacity(3, 1000, 5), 3);

        assert_eq!(initial_capacity(1000, 50, 5), 10);
        assert_eq!(initial_capacity(5, 0, 5), 0);

        assert_eq!(
            initial_capacity(2_000_000, 2 * 1024 * 1024, 5),
            MAX_INITIAL_CAPACITY
        );
        assert_eq!(
            initial_capacity(i32::MAX as usize, usize::MAX, 1),
            MAX_INITIAL_CAPACITY
        );
        assert_eq!(initial_capacity(10, 100, 0), 10);
    }

    #[test]
    fn huge_count_in_a_tiny_buffer_reserves_nothing_and_fails() {
        let data = [0xff, 0xff, 0xff, 0x7f];
        let mut reserved = 0;
        let r = decode_list(&mut &data[..], 5, |_| -> Result<u8, _> {
            reserved += 1;
            Err(ProtocolError::Malformed("truncated"))
        });
        assert_eq!(r, Err(ProtocolError::Malformed("truncated")));
        assert_eq!(reserved, 1, "stops at the first element");
    }

    #[test]
    fn varint_errors() {
        assert_eq!(
            decode_varint(&mut &[][..]),
            Err(ProtocolError::Malformed("truncated"))
        );
        assert_eq!(
            decode_varint(&mut &[0x80][..]),
            Err(ProtocolError::Malformed("truncated"))
        );
        assert_eq!(
            decode_varint(&mut &[0xff; 5][..]),
            Err(ProtocolError::Malformed("bad varint"))
        );

        assert_eq!(
            decode_varint(&mut &[0x80, 0x80, 0x80, 0x80, 0x10][..]),
            Ok(0)
        );
    }
}
