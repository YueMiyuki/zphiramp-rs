use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::error::{CodecError, ProtocolError};
use crate::frame::{DEFAULT_MAX_FRAME_LENGTH, UNLIMITED_FRAME_LENGTH, decode_frame, encode_frame};
use crate::packet::{ClientBoundPacket, ServerBoundPacket};

fn next_frame(src: &mut BytesMut, max_frame_length: usize) -> Result<Option<Bytes>, ProtocolError> {
    loop {
        match decode_frame(src, max_frame_length)? {
            // Empty frames are dropped, not errors.
            Some(frame) if frame.is_empty() => continue,
            other => return Ok(other),
        }
    }
}

#[derive(Debug)]
pub struct ServerCodec {
    scratch: BytesMut,
    max_frame_length: usize,
}

impl ServerCodec {
    pub fn new() -> Self {
        Self::with_max_frame_length(DEFAULT_MAX_FRAME_LENGTH)
    }

    pub fn with_max_frame_length(max_frame_length: usize) -> Self {
        Self {
            scratch: BytesMut::new(),
            max_frame_length,
        }
    }

    pub fn unlimited() -> Self {
        Self::with_max_frame_length(UNLIMITED_FRAME_LENGTH)
    }

    pub fn max_frame_length(&self) -> usize {
        self.max_frame_length
    }

    pub fn set_max_frame_length(&mut self, max_frame_length: usize) {
        self.max_frame_length = max_frame_length;
    }
}

impl Default for ServerCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for ServerCodec {
    type Item = ServerBoundPacket;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<ServerBoundPacket>, CodecError> {
        match next_frame(src, self.max_frame_length)? {
            None => Ok(None),
            Some(mut frame) => Ok(Some(ServerBoundPacket::decode(&mut frame)?)),
        }
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<ServerBoundPacket>, CodecError> {
        let packet = self.decode(src)?;
        if packet.is_none() {
            src.clear();
        }
        Ok(packet)
    }
}

impl Encoder<&ClientBoundPacket> for ServerCodec {
    type Error = CodecError;

    fn encode(&mut self, item: &ClientBoundPacket, dst: &mut BytesMut) -> Result<(), CodecError> {
        self.scratch.clear();
        item.encode(&mut self.scratch);
        Ok(encode_frame(&self.scratch, dst)?)
    }
}

impl Encoder<ClientBoundPacket> for ServerCodec {
    type Error = CodecError;

    fn encode(&mut self, item: ClientBoundPacket, dst: &mut BytesMut) -> Result<(), CodecError> {
        Encoder::<&ClientBoundPacket>::encode(self, &item, dst)
    }
}

#[derive(Debug)]
pub struct ClientCodec {
    scratch: BytesMut,
    max_frame_length: usize,
}

impl ClientCodec {
    pub fn new() -> Self {
        Self::with_max_frame_length(DEFAULT_MAX_FRAME_LENGTH)
    }

    pub fn with_max_frame_length(max_frame_length: usize) -> Self {
        Self {
            scratch: BytesMut::new(),
            max_frame_length,
        }
    }

    pub fn unlimited() -> Self {
        Self::with_max_frame_length(UNLIMITED_FRAME_LENGTH)
    }

    pub fn max_frame_length(&self) -> usize {
        self.max_frame_length
    }

    pub fn set_max_frame_length(&mut self, max_frame_length: usize) {
        self.max_frame_length = max_frame_length;
    }
}

impl Default for ClientCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for ClientCodec {
    type Item = ClientBoundPacket;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<ClientBoundPacket>, CodecError> {
        match next_frame(src, self.max_frame_length)? {
            None => Ok(None),
            Some(mut frame) => Ok(Some(ClientBoundPacket::decode(&mut frame)?)),
        }
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<ClientBoundPacket>, CodecError> {
        let packet = self.decode(src)?;
        if packet.is_none() {
            src.clear();
        }
        Ok(packet)
    }
}

impl Encoder<&ServerBoundPacket> for ClientCodec {
    type Error = CodecError;

    fn encode(&mut self, item: &ServerBoundPacket, dst: &mut BytesMut) -> Result<(), CodecError> {
        self.scratch.clear();
        item.encode(&mut self.scratch);
        Ok(encode_frame(&self.scratch, dst)?)
    }
}

impl Encoder<ServerBoundPacket> for ClientCodec {
    type Error = CodecError;

    fn encode(&mut self, item: ServerBoundPacket, dst: &mut BytesMut) -> Result<(), CodecError> {
        Encoder::<&ServerBoundPacket>::encode(self, &item, dst)
    }
}
