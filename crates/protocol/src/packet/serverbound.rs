use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::clientbound::ClientBoundPacket;
use super::serverbound_id as id;
use crate::data::{JudgeEvent, MAX_ROOM_ID_LENGTH, TouchFrame};
use crate::error::{Direction, ProtocolError};
use crate::wire::{
    Decode, decode_list, decode_string, encode_list, encode_string, get_bool, get_i32_le, get_u8,
};

/// In UTF-8 bytes, not characters.
pub const MAX_TOKEN_LENGTH: i32 = 32;

/// In UTF-8 bytes, not characters.
pub const MAX_CHAT_LENGTH: i32 = 200;

#[derive(Debug, Clone, PartialEq)]
pub enum ServerBoundPacket {
    Ping,

    Authenticate { token: String },

    Chat { message: String },

    Touches { frames: Vec<TouchFrame> },

    Judges { judges: Vec<JudgeEvent> },

    CreateRoom { room_id: String },

    JoinRoom { room_id: String, monitor: bool },

    LeaveRoom,

    LockRoom { lock: bool },

    CycleRoom { cycle: bool },

    SelectChart { id: i32 },

    RequestStart,

    Ready,

    CancelReady,

    Played { id: i32 },

    Abort,
}

impl ServerBoundPacket {
    pub fn id(&self) -> u8 {
        match self {
            Self::Ping => id::PING,
            Self::Authenticate { .. } => id::AUTHENTICATE,
            Self::Chat { .. } => id::CHAT,
            Self::Touches { .. } => id::TOUCHES,
            Self::Judges { .. } => id::JUDGES,
            Self::CreateRoom { .. } => id::CREATE_ROOM,
            Self::JoinRoom { .. } => id::JOIN_ROOM,
            Self::LeaveRoom => id::LEAVE_ROOM,
            Self::LockRoom { .. } => id::LOCK_ROOM,
            Self::CycleRoom { .. } => id::CYCLE_ROOM,
            Self::SelectChart { .. } => id::SELECT_CHART,
            Self::RequestStart => id::REQUEST_START,
            Self::Ready => id::READY,
            Self::CancelReady => id::CANCEL_READY,
            Self::Played { .. } => id::PLAYED,
            Self::Abort => id::ABORT,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Ping => "Ping",
            Self::Authenticate { .. } => "Authenticate",
            Self::Chat { .. } => "Chat",
            Self::Touches { .. } => "Touches",
            Self::Judges { .. } => "Judges",
            Self::CreateRoom { .. } => "CreateRoom",
            Self::JoinRoom { .. } => "JoinRoom",
            Self::LeaveRoom => "LeaveRoom",
            Self::LockRoom { .. } => "LockRoom",
            Self::CycleRoom { .. } => "CycleRoom",
            Self::SelectChart { .. } => "SelectChart",
            Self::RequestStart => "RequestStart",
            Self::Ready => "Ready",
            Self::CancelReady => "CancelReady",
            Self::Played { .. } => "Played",
            Self::Abort => "Abort",
        }
    }

    pub fn encode<B: BufMut>(&self, buf: &mut B) {
        buf.put_u8(self.id());
        self.encode_payload(buf);
    }

    pub fn encode_payload<B: BufMut>(&self, buf: &mut B) {
        match self {
            Self::Ping
            | Self::LeaveRoom
            | Self::RequestStart
            | Self::Ready
            | Self::CancelReady
            | Self::Abort => {}
            Self::Authenticate { token } => encode_string(buf, token),
            Self::Chat { message } => encode_string(buf, message),
            Self::Touches { frames } => encode_list(buf, frames),
            Self::Judges { judges } => encode_list(buf, judges),
            Self::CreateRoom { room_id } => encode_string(buf, room_id),
            Self::JoinRoom { room_id, monitor } => {
                encode_string(buf, room_id);
                buf.put_u8(u8::from(*monitor));
            }
            Self::LockRoom { lock } => buf.put_u8(u8::from(*lock)),
            Self::CycleRoom { cycle } => buf.put_u8(u8::from(*cycle)),
            Self::SelectChart { id } | Self::Played { id } => buf.put_i32_le(*id),
        }
    }

    pub fn to_bytes(&self) -> Bytes {
        let mut buf = BytesMut::new();
        self.encode(&mut buf);
        buf.freeze()
    }

    pub fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let packet_id = get_u8(buf)?;
        // Bytes after the payload are ignored.
        Ok(match packet_id {
            id::PING => Self::Ping,
            id::AUTHENTICATE => Self::Authenticate {
                token: decode_string(buf, MAX_TOKEN_LENGTH)?,
            },
            id::CHAT => Self::Chat {
                message: decode_string(buf, MAX_CHAT_LENGTH)?,
            },
            id::TOUCHES => Self::Touches {
                frames: decode_list(buf, TouchFrame::MIN_WIRE_SIZE, TouchFrame::decode)?,
            },
            id::JUDGES => Self::Judges {
                judges: decode_list(buf, JudgeEvent::WIRE_SIZE, JudgeEvent::decode)?,
            },
            id::CREATE_ROOM => Self::CreateRoom {
                room_id: decode_string(buf, MAX_ROOM_ID_LENGTH)?,
            },
            id::JOIN_ROOM => Self::JoinRoom {
                room_id: decode_string(buf, MAX_ROOM_ID_LENGTH)?,
                monitor: get_bool(buf)?,
            },
            id::LEAVE_ROOM => Self::LeaveRoom,
            id::LOCK_ROOM => Self::LockRoom {
                lock: get_bool(buf)?,
            },
            id::CYCLE_ROOM => Self::CycleRoom {
                cycle: get_bool(buf)?,
            },
            id::SELECT_CHART => Self::SelectChart {
                id: get_i32_le(buf)?,
            },
            id::REQUEST_START => Self::RequestStart,
            id::READY => Self::Ready,
            id::CANCEL_READY => Self::CancelReady,
            id::PLAYED => Self::Played {
                id: get_i32_le(buf)?,
            },
            id::ABORT => Self::Abort,
            other => {
                return Err(ProtocolError::UnknownPacketId {
                    direction: Direction::ServerBound,
                    id: other,
                });
            }
        })
    }

    pub fn default_response(&self) -> Option<ClientBoundPacket> {
        match self {
            Self::Ping => Some(ClientBoundPacket::Pong),
            _ => None,
        }
    }
}
