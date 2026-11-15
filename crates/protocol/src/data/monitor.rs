use bytes::{Buf, BufMut};

use crate::error::{IdKind, ProtocolError};
use crate::wire::{
    Decode, Encode, decode_float16_le, decode_list, encode_float16_le, encode_list, get_f32_le,
    get_i8, get_i32_le,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Judgement {
    Perfect,

    Good,

    Bad,

    Miss,

    HoldPerfect,

    HoldGood,
}

impl Judgement {
    pub fn id(self) -> u8 {
        match self {
            Judgement::Perfect => 0,
            Judgement::Good => 1,
            Judgement::Bad => 2,
            Judgement::Miss => 3,
            Judgement::HoldPerfect => 4,
            Judgement::HoldGood => 5,
        }
    }
}

impl Encode for Judgement {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        buf.put_u8(self.id());
    }
}

impl Decode for Judgement {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        match get_i8(buf)? {
            0 => Ok(Judgement::Perfect),
            1 => Ok(Judgement::Good),
            2 => Ok(Judgement::Bad),
            3 => Ok(Judgement::Miss),
            4 => Ok(Judgement::HoldPerfect),
            5 => Ok(Judgement::HoldGood),
            id => Err(ProtocolError::UnknownId {
                kind: IdKind::Judgement,
                id: i32::from(id),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JudgeEvent {
    pub time: f32,

    pub line_id: i32,

    pub note_id: i32,

    pub judgement: Judgement,
}

impl JudgeEvent {
    pub const WIRE_SIZE: usize = 13;
}

impl Encode for JudgeEvent {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        buf.put_u32_le(self.time.to_bits());
        buf.put_i32_le(self.line_id);
        buf.put_i32_le(self.note_id);
        self.judgement.encode(buf);
    }
}

impl Decode for JudgeEvent {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            time: get_f32_le(buf)?,
            line_id: get_i32_le(buf)?,
            note_id: get_i32_le(buf)?,
            judgement: Judgement::decode(buf)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactPos {
    pub x: f32,

    pub y: f32,
}

impl Encode for CompactPos {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        encode_float16_le(buf, self.x);
        encode_float16_le(buf, self.y);
    }
}

impl Decode for CompactPos {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            x: decode_float16_le(buf)?,
            y: decode_float16_le(buf)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchPoint {
    pub id: i8,

    pub pos: CompactPos,
}

impl TouchPoint {
    pub const WIRE_SIZE: usize = 5;
}

impl Encode for TouchPoint {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        buf.put_i8(self.id);
        self.pos.encode(buf);
    }
}

impl Decode for TouchPoint {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: get_i8(buf)?,
            pos: CompactPos::decode(buf)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TouchFrame {
    pub time: f32,

    pub points: Vec<TouchPoint>,
}

impl TouchFrame {
    pub const MIN_WIRE_SIZE: usize = 5;
}

impl Encode for TouchFrame {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        buf.put_u32_le(self.time.to_bits());
        encode_list(buf, &self.points);
    }
}

impl Decode for TouchFrame {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            time: get_f32_le(buf)?,
            points: decode_list(buf, TouchPoint::WIRE_SIZE, TouchPoint::decode)?,
        })
    }
}
