//! Protocol errors.

use crate::frame::FrameType;
use crate::payload::ErrorPayload;

/// Result type for protocol operations.
pub type Result<T> = std::result::Result<T, ProtoError>;

/// An error raised by the agent protocol.
#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad frame magic: {0:?}")]
    BadMagic([u8; 4]),
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u8),
    #[error("unsupported frame flags: {0}")]
    UnsupportedFlags(u8),
    #[error("payload too large: {length} bytes")]
    PayloadTooLarge { length: u32 },
    #[error("truncated frame: expected {expected} bytes, got {actual}")]
    TruncatedFrame { expected: usize, actual: usize },
    #[error(
        "crc mismatch for request {request_id} sequence {sequence}: \
         expected {expected:#010x}, actual {actual:#010x}"
    )]
    CrcMismatch {
        request_id: u32,
        sequence: u32,
        expected: u32,
        actual: u32,
    },
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unexpected frame: expected {expected}, got {actual:?}")]
    UnexpectedFrame {
        expected: &'static str,
        actual: FrameType,
    },
    #[error("timed out waiting for a frame")]
    Timeout,
    #[error(
        "retries exhausted for request {request_id} sequence {sequence} \
         after {attempts} attempts"
    )]
    RetriesExhausted {
        request_id: u32,
        sequence: u32,
        attempts: u32,
    },
    #[error("crc failed after {attempts} attempts")]
    CrcFailed { attempts: u32 },
    #[error("session is closed")]
    Closed,
    #[error("command is too long: {length} characters, maximum {max}")]
    CommandTooLong { length: usize, max: usize },
    #[error("output is too large: {length} bytes, maximum {max}")]
    OutputTooLarge { length: usize, max: usize },
    #[error("transfer is too large: {size} bytes, maximum {max}")]
    TransferTooLarge { size: u64, max: u64 },
    #[error("byte count mismatch: expected {expected}, actual {actual}")]
    ByteCountMismatch { expected: u64, actual: u64 },
    #[error("unsupported protocol version: {0}")]
    UnsupportedProtocol(u8),
    #[error("session is already handshaken")]
    AlreadyHandshaken,
    #[error("guest rebooted")]
    GuestRebooted,
    #[error("handshake timed out")]
    HandshakeTimeout,
    #[error("agent error: {0}")]
    Agent(ErrorPayload),
}
