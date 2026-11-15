use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::error::{CodecError, ProtocolError};
use crate::wire::{encode_varint, try_decode_varint};

pub const DEFAULT_MAX_FRAME_LENGTH: usize = 2 * 1024 * 1024;

/// Cap before login: the largest legal frame (an Authenticate) is 38 bytes.
pub const PRE_AUTH_MAX_FRAME_LENGTH: usize = 1024;

pub fn decode_frame(
    src: &mut BytesMut,
    max_frame_length: usize,
) -> Result<Option<Bytes>, ProtocolError> {
    // Wire compatibility: zero bytes before a length prefix are padding and are skipped.
    match src.iter().position(|&b| b != 0) {
        None => {
            src.clear();
            return Ok(None);
        }
        Some(skip) => src.advance(skip),
    }

    let mut peek: &[u8] = &src[..];
    let before = peek.len();
    let Some(length) = try_decode_varint(&mut peek)? else {
        return Ok(None);
    };
    if length < 0 {
        return Err(ProtocolError::Malformed("negative frame length"));
    }
    let prefix_len = before - peek.len();
    let length = length as usize;
    // Checked before the body is buffered, so a peer cannot make us hold it.
    if length > max_frame_length {
        return Err(ProtocolError::FrameTooLarge {
            length,
            max: max_frame_length,
        });
    }
    if src.len() - prefix_len < length {
        return Ok(None);
    }
    src.advance(prefix_len);
    Ok(Some(src.split_to(length).freeze()))
}

pub fn encode_frame<B: BufMut>(payload: &[u8], dst: &mut B) -> Result<(), ProtocolError> {
    let length = i32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge {
        length: payload.len(),
        max: i32::MAX as usize,
    })?;
    encode_varint(dst, length);
    dst.put_slice(payload);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDecoder {
    max_frame_length: usize,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::with_max_frame_length(DEFAULT_MAX_FRAME_LENGTH)
    }

    pub fn with_max_frame_length(max_frame_length: usize) -> Self {
        Self { max_frame_length }
    }

    pub fn max_frame_length(&self) -> usize {
        self.max_frame_length
    }

    pub fn set_max_frame_length(&mut self, max_frame_length: usize) {
        self.max_frame_length = max_frame_length;
    }
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for FrameDecoder {
    type Item = Bytes;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Bytes>, CodecError> {
        Ok(decode_frame(src, self.max_frame_length)?)
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Bytes>, CodecError> {
        // The leftover of an incomplete frame at end of stream is dropped without error.
        let frame = decode_frame(src, self.max_frame_length)?;
        if frame.is_none() {
            src.clear();
        }
        Ok(frame)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FrameEncoder;

impl Encoder<&[u8]> for FrameEncoder {
    type Error = CodecError;

    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> Result<(), CodecError> {
        Ok(encode_frame(item, dst)?)
    }
}

impl Encoder<Bytes> for FrameEncoder {
    type Error = CodecError;

    fn encode(&mut self, item: Bytes, dst: &mut BytesMut) -> Result<(), CodecError> {
        Ok(encode_frame(&item, dst)?)
    }
}
