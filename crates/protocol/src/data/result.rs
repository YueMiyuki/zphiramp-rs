use bytes::{Buf, BufMut};

use crate::error::ProtocolError;
use crate::wire::{Decode, Encode, decode_string_default, encode_string, get_bool};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PacketResult<T> {
    Success(T),

    Failed(String),
}

impl<T> PacketResult<T> {
    pub fn success(value: T) -> Self {
        PacketResult::Success(value)
    }

    pub fn failed(message: impl Into<String>) -> Self {
        PacketResult::Failed(message.into())
    }

    pub fn is_success(&self) -> bool {
        matches!(self, PacketResult::Success(_))
    }

    pub fn result(&self) -> Option<&T> {
        match self {
            PacketResult::Success(v) => Some(v),
            PacketResult::Failed(_) => None,
        }
    }

    pub fn failed_message(&self) -> Option<&str> {
        match self {
            PacketResult::Success(_) => None,
            PacketResult::Failed(m) => Some(m),
        }
    }

    pub fn into_result(self) -> Result<T, String> {
        match self {
            PacketResult::Success(v) => Ok(v),
            PacketResult::Failed(m) => Err(m),
        }
    }
}

impl PacketResult<()> {
    pub fn success_void() -> Self {
        PacketResult::Success(())
    }
}

impl<T> From<Result<T, String>> for PacketResult<T> {
    fn from(r: Result<T, String>) -> Self {
        match r {
            Ok(v) => PacketResult::Success(v),
            Err(m) => PacketResult::Failed(m),
        }
    }
}

impl<T: Encode> Encode for PacketResult<T> {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        match self {
            PacketResult::Success(v) => {
                buf.put_u8(1);
                v.encode(buf);
            }
            PacketResult::Failed(m) => {
                buf.put_u8(0);
                encode_string(buf, m);
            }
        }
    }
}

impl<T: Decode> Decode for PacketResult<T> {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        if !get_bool(buf)? {
            return Ok(PacketResult::Failed(decode_string_default(buf)?));
        }
        Ok(PacketResult::Success(T::decode(buf)?))
    }
}
