use std::time::Duration;

use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::error::{CodecError, HandshakeError};

pub const PROTOCOL_VERSION: u8 = 1;

pub const SUPPORTED_VERSIONS: [u8; 1] = [PROTOCOL_VERSION];

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(5000);

pub fn decode_handshake<B: Buf>(buf: &mut B) -> Result<Option<u8>, HandshakeError> {
    if !buf.has_remaining() {
        return Ok(None);
    }
    // Signed on purpose: the error text shows 0xff as -1.
    let version = buf.get_i8();
    match u8::try_from(version) {
        Ok(v) if SUPPORTED_VERSIONS.contains(&v) => Ok(Some(v)),
        _ => Err(HandshakeError::UnsupportedVersion(i32::from(version))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandshakeEncoder {
    version: u8,
}

impl HandshakeEncoder {
    pub fn new(version: i32) -> Result<Self, HandshakeError> {
        match u8::try_from(version) {
            Ok(v) if SUPPORTED_VERSIONS.contains(&v) => Ok(Self { version: v }),
            _ => Err(HandshakeError::UnsupportedVersion(version)),
        }
    }

    pub fn version(&self) -> u8 {
        self.version
    }

    pub fn encode<B: BufMut>(&self, buf: &mut B) {
        buf.put_u8(self.version);
    }
}

impl Default for HandshakeEncoder {
    fn default() -> Self {
        Self {
            version: PROTOCOL_VERSION,
        }
    }
}

impl Encoder<()> for HandshakeEncoder {
    type Error = CodecError;

    fn encode(&mut self, _item: (), dst: &mut BytesMut) -> Result<(), CodecError> {
        HandshakeEncoder::encode(self, dst);
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HandshakeDecoder;

impl Decoder for HandshakeDecoder {
    type Item = u8;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<u8>, CodecError> {
        Ok(decode_handshake(src)?)
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<u8>, CodecError> {
        match decode_handshake(src)? {
            Some(version) => Ok(Some(version)),
            None => Err(HandshakeError::ConnectionClosed.into()),
        }
    }
}
