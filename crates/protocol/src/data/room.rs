use bytes::{Buf, BufMut};

use crate::data::state::GameState;
use crate::data::user::{FullUserProfile, UserProfile};
use crate::error::ProtocolError;
use crate::wire::{
    Decode, Encode, decode_string, decode_varint, encode_string, encode_varint, get_bool,
    get_i32_le, initial_capacity,
};

/// In UTF-8 bytes, not characters.
pub const MAX_ROOM_ID_LENGTH: i32 = 20;

const ROOM_USER_MIN_WIRE_SIZE: usize = 4 + FullUserProfile::MIN_WIRE_SIZE;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoomInfo {
    pub room_id: String,

    pub state: GameState,

    pub live: bool,

    pub locked: bool,

    pub cycle: bool,

    pub is_host: bool,

    pub is_ready: bool,

    pub users: Vec<FullUserProfile>,
}

impl RoomInfo {
    #[allow(clippy::too_many_arguments)]
    pub fn from_lists(
        room_id: impl Into<String>,
        state: GameState,
        live: bool,
        locked: bool,
        cycle: bool,
        is_host: bool,
        is_ready: bool,
        users: &[UserProfile],
        monitors: &[UserProfile],
    ) -> Self {
        Self {
            room_id: room_id.into(),
            state,
            live,
            locked,
            cycle,
            is_host,
            is_ready,
            users: FullUserProfile::from_lists(users, monitors),
        }
    }
}

impl Encode for RoomInfo {
    fn encode<B: BufMut>(&self, buf: &mut B) {
        encode_string(buf, &self.room_id);
        self.state.encode(buf);
        buf.put_u8(u8::from(self.live));
        buf.put_u8(u8::from(self.locked));
        buf.put_u8(u8::from(self.cycle));
        buf.put_u8(u8::from(self.is_host));
        buf.put_u8(u8::from(self.is_ready));
        encode_varint(buf, self.users.len() as i32);
        for user in &self.users {
            buf.put_i32_le(user.user_id());
            user.encode(buf);
        }
    }
}

impl Decode for RoomInfo {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let room_id = decode_string(buf, MAX_ROOM_ID_LENGTH)?;
        let state = GameState::decode(buf)?;
        let live = get_bool(buf)?;
        let locked = get_bool(buf)?;
        let cycle = get_bool(buf)?;
        let is_host = get_bool(buf)?;
        let is_ready = get_bool(buf)?;
        let size = decode_varint(buf)?;

        let mut users = Vec::with_capacity(initial_capacity(
            size.max(0) as usize,
            buf.remaining(),
            ROOM_USER_MIN_WIRE_SIZE,
        ));
        // A negative count reads no entries; the map key repeats the user id and is dropped.
        for _ in 0..size {
            get_i32_le(buf)?;
            users.push(FullUserProfile::decode(buf)?);
        }
        Ok(Self {
            room_id,
            state,
            live,
            locked,
            cycle,
            is_host,
            is_ready,
            users,
        })
    }
}
