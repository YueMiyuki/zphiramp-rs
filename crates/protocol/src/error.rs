use std::fmt;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdKind {
    ServerBoundPacket,
    ClientBoundPacket,
    GameState,
    Message,
    Judgement,
}

impl fmt::Display for IdKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IdKind::ServerBoundPacket => "serverbound packet",
            IdKind::ClientBoundPacket => "clientbound packet",
            IdKind::GameState => "game state",
            IdKind::Message => "message",
            IdKind::Judgement => "judgement",
        })
    }
}

/// What the server does with a connection after a protocol error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineAction {
    Close,
    WarnAndClose,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("malformed data: {0}")]
    Malformed(&'static str),

    #[error("unknown {kind} id {id}")]
    UnknownId { kind: IdKind, id: i32 },

    #[error("string of {length} bytes exceeds the limit of {max}")]
    StringTooLong { length: i32, max: i32 },

    #[error("frame of {length} bytes exceeds the limit of {max}")]
    FrameTooLarge { length: usize, max: usize },
}

impl ProtocolError {
    pub fn pipeline_action(&self) -> PipelineAction {
        match self {
            ProtocolError::FrameTooLarge { .. } => PipelineAction::WarnAndClose,
            _ => PipelineAction::Close,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandshakeError {
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(i32),

    #[error("Handshake timeout")]
    Timeout,

    #[error("Connection closed before handshake")]
    ConnectionClosed,
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    #[error(transparent)]
    Handshake(#[from] HandshakeError),

    #[error(transparent)]
    Io(#[from] io::Error),
}
