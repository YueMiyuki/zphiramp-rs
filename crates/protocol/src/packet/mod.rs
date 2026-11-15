mod clientbound;
mod serverbound;

pub use clientbound::{AuthenticateOk, ClientBoundPacket, JoinRoomOk};
pub use serverbound::{MAX_CHAT_LENGTH, MAX_TOKEN_LENGTH, ServerBoundPacket};

pub mod serverbound_id {

    pub const PING: u8 = 0x00;

    pub const AUTHENTICATE: u8 = 0x01;

    pub const CHAT: u8 = 0x02;

    pub const TOUCHES: u8 = 0x03;

    pub const JUDGES: u8 = 0x04;

    pub const CREATE_ROOM: u8 = 0x05;

    pub const JOIN_ROOM: u8 = 0x06;

    pub const LEAVE_ROOM: u8 = 0x07;

    pub const LOCK_ROOM: u8 = 0x08;

    pub const CYCLE_ROOM: u8 = 0x09;

    pub const SELECT_CHART: u8 = 0x0a;

    pub const REQUEST_START: u8 = 0x0b;

    pub const READY: u8 = 0x0c;

    pub const CANCEL_READY: u8 = 0x0d;

    pub const PLAYED: u8 = 0x0e;

    pub const ABORT: u8 = 0x0f;
}

pub mod clientbound_id {

    pub const PONG: u8 = 0x00;

    pub const AUTHENTICATE: u8 = 0x01;

    pub const CHAT: u8 = 0x02;

    pub const TOUCHES: u8 = 0x03;

    pub const JUDGES: u8 = 0x04;

    pub const MESSAGE: u8 = 0x05;

    pub const CHANGE_STATE: u8 = 0x06;

    pub const CHANGE_HOST: u8 = 0x07;

    pub const CREATE_ROOM: u8 = 0x08;

    pub const JOIN_ROOM: u8 = 0x09;

    pub const ON_JOIN_ROOM: u8 = 0x0a;

    pub const LEAVE_ROOM: u8 = 0x0b;

    pub const LOCK_ROOM: u8 = 0x0c;

    pub const CYCLE_ROOM: u8 = 0x0d;

    pub const SELECT_CHART: u8 = 0x0e;

    pub const REQUEST_START: u8 = 0x0f;

    pub const READY: u8 = 0x10;

    pub const CANCEL_READY: u8 = 0x11;

    pub const PLAYED: u8 = 0x12;

    pub const ABORT: u8 = 0x13;
}
