//! Phira multiplayer wire protocol: handshake, framing, packets and data types.
//! Byte-compatible with existing Phira clients and servers.

#![forbid(unsafe_code)]

pub mod codec;
pub mod data;
pub mod error;
pub mod float16;
pub mod frame;
pub mod handshake;
mod java_utf8;
pub mod packet;
pub mod wire;

pub use codec::{ClientCodec, ServerCodec};
pub use data::{
    CompactPos, FullUserProfile, GameState, JudgeEvent, Judgement, MAX_ROOM_ID_LENGTH, Message,
    PacketResult, RoomInfo, TouchFrame, TouchPoint, UserProfile,
};
pub use error::{CodecError, Direction, HandshakeError, PipelineAction, ProtocolError};
pub use frame::{
    DEFAULT_MAX_FRAME_LENGTH, FrameDecoder, FrameEncoder, PRE_AUTH_MAX_FRAME_LENGTH,
    UNLIMITED_FRAME_LENGTH, decode_frame, encode_frame,
};
pub use handshake::{
    HANDSHAKE_TIMEOUT, HandshakeDecoder, HandshakeEncoder, PROTOCOL_VERSION, SUPPORTED_VERSIONS,
    decode_handshake,
};
pub use packet::{
    AuthenticateOk, ClientBoundPacket, JoinRoomOk, MAX_CHAT_LENGTH, MAX_TOKEN_LENGTH,
    ServerBoundPacket, clientbound_id, serverbound_id,
};
pub use wire::{Decode, Encode, MAX_STRING_LENGTH};
