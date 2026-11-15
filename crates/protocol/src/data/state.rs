use bytes::{Buf, BufMut};

use crate::error::ProtocolError;
use crate::wire::{Decode, Encode, get_bool, get_i8, get_i32_le};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameState {
    SelectChart(Option<i32>),

    WaitForReady,

    Playing,
}

impl Encode for GameState {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        match self {
            GameState::SelectChart(chart_id) => {
                buf.put_u8(0x00);
                buf.put_u8(u8::from(chart_id.is_some()));
                if let Some(id) = chart_id {
                    buf.put_i32_le(*id);
                }
            }
            GameState::WaitForReady => buf.put_u8(0x01),
            GameState::Playing => buf.put_u8(0x02),
        }
    }
}

impl Decode for GameState {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        match get_i8(buf)? {
            0x00 => Ok(GameState::SelectChart(if get_bool(buf)? {
                Some(get_i32_le(buf)?)
            } else {
                None
            })),
            0x01 => Ok(GameState::WaitForReady),
            0x02 => Ok(GameState::Playing),
            id => Err(ProtocolError::UnknownGameStateId(id)),
        }
    }
}
