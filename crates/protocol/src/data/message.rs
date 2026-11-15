use bytes::{Buf, BufMut};

use crate::error::{IdKind, ProtocolError};
use crate::wire::{
    Decode, Encode, decode_string_default, encode_string, get_bool, get_f32_le, get_i8, get_i32_le,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Chat {
        user: i32,

        content: String,
    },

    CreateRoom {
        user: i32,
    },

    JoinRoom {
        user: i32,

        name: String,
    },

    LeaveRoom {
        user: i32,

        name: String,
    },

    NewHost {
        user: i32,
    },

    SelectChart {
        user: i32,

        name: String,

        id: i32,
    },

    GameStart {
        user: i32,
    },

    Ready {
        user: i32,
    },

    CancelReady {
        user: i32,
    },

    CancelGame {
        user: i32,
    },

    StartPlaying,

    Played {
        user: i32,

        score: i32,

        accuracy: f32,

        full_combo: bool,
    },

    GameEnd,

    Abort {
        user: i32,
    },

    LockRoom {
        lock: bool,
    },

    CycleRoom {
        cycle: bool,
    },
}

impl Message {
    pub fn id(&self) -> u8 {
        match self {
            Message::Chat { .. } => 0x00,
            Message::CreateRoom { .. } => 0x01,
            Message::JoinRoom { .. } => 0x02,
            Message::LeaveRoom { .. } => 0x03,
            Message::NewHost { .. } => 0x04,
            Message::SelectChart { .. } => 0x05,
            Message::GameStart { .. } => 0x06,
            Message::Ready { .. } => 0x07,
            Message::CancelReady { .. } => 0x08,
            Message::CancelGame { .. } => 0x09,
            Message::StartPlaying => 0x0a,
            Message::Played { .. } => 0x0b,
            Message::GameEnd => 0x0c,
            Message::Abort { .. } => 0x0d,
            Message::LockRoom { .. } => 0x0e,
            Message::CycleRoom { .. } => 0x0f,
        }
    }
}

impl Encode for Message {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        buf.put_u8(self.id());
        match self {
            Message::Chat { user, content } => {
                buf.put_i32_le(*user);
                encode_string(buf, content);
            }
            Message::JoinRoom { user, name } | Message::LeaveRoom { user, name } => {
                buf.put_i32_le(*user);
                encode_string(buf, name);
            }
            Message::CreateRoom { user }
            | Message::NewHost { user }
            | Message::GameStart { user }
            | Message::Ready { user }
            | Message::CancelReady { user }
            | Message::CancelGame { user }
            | Message::Abort { user } => buf.put_i32_le(*user),
            Message::SelectChart { user, name, id } => {
                buf.put_i32_le(*user);
                encode_string(buf, name);
                buf.put_i32_le(*id);
            }
            Message::StartPlaying | Message::GameEnd => {}
            Message::Played {
                user,
                score,
                accuracy,
                full_combo,
            } => {
                buf.put_i32_le(*user);
                buf.put_i32_le(*score);
                buf.put_u32_le(accuracy.to_bits());
                buf.put_u8(u8::from(*full_combo));
            }
            Message::LockRoom { lock } => buf.put_u8(u8::from(*lock)),
            Message::CycleRoom { cycle } => buf.put_u8(u8::from(*cycle)),
        }
    }
}

impl Decode for Message {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(match get_i8(buf)? {
            0x00 => Message::Chat {
                user: get_i32_le(buf)?,
                content: decode_string_default(buf)?,
            },
            0x01 => Message::CreateRoom {
                user: get_i32_le(buf)?,
            },
            0x02 => Message::JoinRoom {
                user: get_i32_le(buf)?,
                name: decode_string_default(buf)?,
            },
            0x03 => Message::LeaveRoom {
                user: get_i32_le(buf)?,
                name: decode_string_default(buf)?,
            },
            0x04 => Message::NewHost {
                user: get_i32_le(buf)?,
            },
            0x05 => Message::SelectChart {
                user: get_i32_le(buf)?,
                name: decode_string_default(buf)?,
                id: get_i32_le(buf)?,
            },
            0x06 => Message::GameStart {
                user: get_i32_le(buf)?,
            },
            0x07 => Message::Ready {
                user: get_i32_le(buf)?,
            },
            0x08 => Message::CancelReady {
                user: get_i32_le(buf)?,
            },
            0x09 => Message::CancelGame {
                user: get_i32_le(buf)?,
            },
            0x0a => Message::StartPlaying,
            0x0b => Message::Played {
                user: get_i32_le(buf)?,
                score: get_i32_le(buf)?,
                accuracy: get_f32_le(buf)?,
                full_combo: get_bool(buf)?,
            },
            0x0c => Message::GameEnd,
            0x0d => Message::Abort {
                user: get_i32_le(buf)?,
            },
            0x0e => Message::LockRoom {
                lock: get_bool(buf)?,
            },
            0x0f => Message::CycleRoom {
                cycle: get_bool(buf)?,
            },
            id => {
                return Err(ProtocolError::UnknownId {
                    kind: IdKind::Message,
                    id: i32::from(id),
                });
            }
        })
    }
}
