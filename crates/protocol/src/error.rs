use std::fmt;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    ServerBound,

    ClientBound,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Direction::ServerBound => "ServerBound",
            Direction::ClientBound => "ClientBound",
        })
    }
}

/// How the server reports a decode failure before closing the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineAction {
    CloseSilently,

    ExceptionCaught,

    WarnAndClose,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("Bad VarInt decoded")]
    BadVarInt,

    #[error("Bad packet length")]
    BadPacketLength,

    #[error("Buffer does not contain enough bytes to decode")]
    NeedMoreData,

    #[error("Bad string length: {length} (max: {max})")]
    BadStringLength { length: i32, max: i32 },

    #[error("Unknown {direction} packet id: {id}")]
    UnknownPacketId { direction: Direction, id: u8 },

    #[error("Unknown GameState id: {0}")]
    UnknownGameStateId(i8),

    #[error("Unknown Message id: {0}")]
    UnknownMessageId(i8),

    #[error("Unknown Judgement id: {0}")]
    UnknownJudgementId(i8),

    #[error("index out of bounds: need {needed} byte(s), {remaining} left")]
    IndexOutOfBounds { needed: usize, remaining: usize },

    #[error("Illegal Capacity: {0}")]
    IllegalCapacity(i32),

    #[error("frame of {0} bytes does not fit the length prefix")]
    FrameTooLarge(usize),

    #[error("frame length {length} exceeds the limit of {max} bytes")]
    InboundFrameTooLarge { length: usize, max: usize },
}

impl ProtocolError {
    pub fn pipeline_action(&self) -> PipelineAction {
        match self {
            ProtocolError::IndexOutOfBounds { .. } | ProtocolError::IllegalCapacity(_) => {
                PipelineAction::ExceptionCaught
            }
            ProtocolError::InboundFrameTooLarge { .. } => PipelineAction::WarnAndClose,
            _ => PipelineAction::CloseSilently,
        }
    }

    pub fn java_exception(&self) -> Option<&'static str> {
        Some(match self {
            ProtocolError::BadVarInt => "top.rymc.phira.protocol.exception.BadVarIntException",
            ProtocolError::BadPacketLength => "io.netty.handler.codec.CorruptedFrameException",
            ProtocolError::NeedMoreData => {
                "top.rymc.phira.protocol.exception.NeedMoreDataException"
            }
            ProtocolError::BadStringLength { .. }
            | ProtocolError::UnknownPacketId { .. }
            | ProtocolError::UnknownGameStateId(_)
            | ProtocolError::UnknownMessageId(_)
            | ProtocolError::UnknownJudgementId(_) => "io.netty.handler.codec.DecoderException",
            ProtocolError::IndexOutOfBounds { .. } => "java.lang.IndexOutOfBoundsException",
            ProtocolError::IllegalCapacity(_) => "java.lang.IllegalArgumentException",
            ProtocolError::FrameTooLarge(_) | ProtocolError::InboundFrameTooLarge { .. } => {
                return None;
            }
        })
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

impl HandshakeError {
    pub fn java_exception(&self) -> &'static str {
        match self {
            HandshakeError::UnsupportedVersion(_) => "java.lang.IllegalStateException",
            HandshakeError::Timeout => "java.util.concurrent.TimeoutException",
            HandshakeError::ConnectionClosed => "java.io.IOException",
        }
    }
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
