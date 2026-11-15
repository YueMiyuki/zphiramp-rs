use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::clientbound_id as id;
use crate::data::{
    FullUserProfile, GameState, JudgeEvent, Message, PacketResult, RoomInfo, TouchFrame,
    UserProfile,
};
use crate::error::{IdKind, ProtocolError};
use crate::wire::{Decode, Encode, decode_list, encode_list, get_bool, get_i32_le, get_u8};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticateOk {
    pub user_profile: FullUserProfile,

    pub room_info: Option<RoomInfo>,
}

impl Encode for AuthenticateOk {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        self.user_profile.encode(buf);
        buf.put_u8(u8::from(self.room_info.is_some()));
        if let Some(room) = &self.room_info {
            room.encode(buf);
        }
    }
}

impl Decode for AuthenticateOk {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let user_profile = FullUserProfile::decode(buf)?;
        let room_info = if get_bool(buf)? {
            Some(RoomInfo::decode(buf)?)
        } else {
            None
        };
        Ok(Self {
            user_profile,
            room_info,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRoomOk {
    pub game_state: GameState,

    pub users: Vec<FullUserProfile>,

    pub live: bool,
}

impl JoinRoomOk {
    pub fn from_lists(
        game_state: GameState,
        users: &[UserProfile],
        monitors: &[UserProfile],
        live: bool,
    ) -> Self {
        Self {
            game_state,
            users: FullUserProfile::from_lists(users, monitors),
            live,
        }
    }
}

impl Encode for JoinRoomOk {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        self.game_state.encode(buf);
        encode_list(buf, &self.users);
        buf.put_u8(u8::from(self.live));
    }
}

impl Decode for JoinRoomOk {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            game_state: GameState::decode(buf)?,
            users: decode_list(buf, FullUserProfile::MIN_WIRE_SIZE, FullUserProfile::decode)?,
            live: get_bool(buf)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientBoundPacket {
    Pong,

    Authenticate(PacketResult<AuthenticateOk>),

    Chat(PacketResult<()>),

    Touches { id: i32, frames: Vec<TouchFrame> },

    Judges { id: i32, judges: Vec<JudgeEvent> },

    Message(Message),

    ChangeState(GameState),

    ChangeHost { host: bool },

    CreateRoom(PacketResult<()>),

    JoinRoom(PacketResult<JoinRoomOk>),

    OnJoinRoom(FullUserProfile),

    LeaveRoom(PacketResult<()>),

    LockRoom(PacketResult<()>),

    CycleRoom(PacketResult<()>),

    SelectChart(PacketResult<()>),

    RequestStart(PacketResult<()>),

    Ready(PacketResult<()>),

    CancelReady(PacketResult<()>),

    Played(PacketResult<()>),

    Abort(PacketResult<()>),
}

impl ClientBoundPacket {
    pub fn id(&self) -> u8 {
        match self {
            Self::Pong => id::PONG,
            Self::Authenticate(_) => id::AUTHENTICATE,
            Self::Chat(_) => id::CHAT,
            Self::Touches { .. } => id::TOUCHES,
            Self::Judges { .. } => id::JUDGES,
            Self::Message(_) => id::MESSAGE,
            Self::ChangeState(_) => id::CHANGE_STATE,
            Self::ChangeHost { .. } => id::CHANGE_HOST,
            Self::CreateRoom(_) => id::CREATE_ROOM,
            Self::JoinRoom(_) => id::JOIN_ROOM,
            Self::OnJoinRoom(_) => id::ON_JOIN_ROOM,
            Self::LeaveRoom(_) => id::LEAVE_ROOM,
            Self::LockRoom(_) => id::LOCK_ROOM,
            Self::CycleRoom(_) => id::CYCLE_ROOM,
            Self::SelectChart(_) => id::SELECT_CHART,
            Self::RequestStart(_) => id::REQUEST_START,
            Self::Ready(_) => id::READY,
            Self::CancelReady(_) => id::CANCEL_READY,
            Self::Played(_) => id::PLAYED,
            Self::Abort(_) => id::ABORT,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Pong => "Pong",
            Self::Authenticate(_) => "Authenticate",
            Self::Chat(_) => "Chat",
            Self::Touches { .. } => "Touches",
            Self::Judges { .. } => "Judges",
            Self::Message(_) => "Message",
            Self::ChangeState(_) => "ChangeState",
            Self::ChangeHost { .. } => "ChangeHost",
            Self::CreateRoom(_) => "CreateRoom",
            Self::JoinRoom(_) => "JoinRoom",
            Self::OnJoinRoom(_) => "OnJoinRoom",
            Self::LeaveRoom(_) => "LeaveRoom",
            Self::LockRoom(_) => "LockRoom",
            Self::CycleRoom(_) => "CycleRoom",
            Self::SelectChart(_) => "SelectChart",
            Self::RequestStart(_) => "RequestStart",
            Self::Ready(_) => "Ready",
            Self::CancelReady(_) => "CancelReady",
            Self::Played(_) => "Played",
            Self::Abort(_) => "Abort",
        }
    }

    pub fn encode<B: BufMut>(&self, buf: &mut B) {
        buf.put_u8(self.id());
        self.encode_payload(buf);
    }

    pub fn encode_payload<B: BufMut>(&self, buf: &mut B) {
        match self {
            Self::Pong => {}
            Self::Authenticate(r) => r.encode(buf),
            Self::Touches { id, frames } => {
                buf.put_i32_le(*id);
                encode_list(buf, frames);
            }
            Self::Judges { id, judges } => {
                buf.put_i32_le(*id);
                encode_list(buf, judges);
            }
            Self::Message(m) => m.encode(buf),
            Self::ChangeState(s) => s.encode(buf),
            Self::ChangeHost { host } => buf.put_u8(u8::from(*host)),
            Self::JoinRoom(r) => r.encode(buf),
            Self::OnJoinRoom(u) => u.encode(buf),
            Self::Chat(r)
            | Self::CreateRoom(r)
            | Self::LeaveRoom(r)
            | Self::LockRoom(r)
            | Self::CycleRoom(r)
            | Self::SelectChart(r)
            | Self::RequestStart(r)
            | Self::Ready(r)
            | Self::CancelReady(r)
            | Self::Played(r)
            | Self::Abort(r) => r.encode(buf),
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
            id::PONG => Self::Pong,
            id::AUTHENTICATE => Self::Authenticate(PacketResult::decode(buf)?),
            id::CHAT => Self::Chat(PacketResult::decode(buf)?),
            id::TOUCHES => Self::Touches {
                id: get_i32_le(buf)?,
                frames: decode_list(buf, TouchFrame::MIN_WIRE_SIZE, TouchFrame::decode)?,
            },
            id::JUDGES => Self::Judges {
                id: get_i32_le(buf)?,
                judges: decode_list(buf, JudgeEvent::WIRE_SIZE, JudgeEvent::decode)?,
            },
            id::MESSAGE => Self::Message(Message::decode(buf)?),
            id::CHANGE_STATE => Self::ChangeState(GameState::decode(buf)?),
            id::CHANGE_HOST => Self::ChangeHost {
                host: get_bool(buf)?,
            },
            id::CREATE_ROOM => Self::CreateRoom(PacketResult::decode(buf)?),
            id::JOIN_ROOM => Self::JoinRoom(PacketResult::decode(buf)?),
            id::ON_JOIN_ROOM => Self::OnJoinRoom(FullUserProfile::decode(buf)?),
            id::LEAVE_ROOM => Self::LeaveRoom(PacketResult::decode(buf)?),
            id::LOCK_ROOM => Self::LockRoom(PacketResult::decode(buf)?),
            id::CYCLE_ROOM => Self::CycleRoom(PacketResult::decode(buf)?),
            id::SELECT_CHART => Self::SelectChart(PacketResult::decode(buf)?),
            id::REQUEST_START => Self::RequestStart(PacketResult::decode(buf)?),
            id::READY => Self::Ready(PacketResult::decode(buf)?),
            id::CANCEL_READY => Self::CancelReady(PacketResult::decode(buf)?),
            id::PLAYED => Self::Played(PacketResult::decode(buf)?),
            id::ABORT => Self::Abort(PacketResult::decode(buf)?),
            other => {
                return Err(ProtocolError::UnknownId {
                    kind: IdKind::ClientBoundPacket,
                    id: i32::from(other),
                });
            }
        })
    }
}
