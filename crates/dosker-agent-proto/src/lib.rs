//! Host-side frame codec and payloads for the DOS agent protocol.

pub mod client;
pub mod codec;
pub mod crc32;
pub mod error;
pub mod frame;
pub mod payload;
pub mod session;

pub use client::{AgentClient, ExecOutcome};
pub use codec::{Incoming, read_frame, write_frame};
pub use crc32::{IEEE_POLY, crc32, crc32_update};
pub use error::{ProtoError, Result};
pub use frame::{
    AGENT_VERSION, CRC_BYTES, DEFAULT_TIMEOUT_MS, Frame, FrameHeader, FrameType, HEADER_BYTES,
    MAGIC, MAX_COMMAND_CHARS, MAX_FRAME_BYTES, MAX_OUTPUT_BYTES, MAX_PAYLOAD, MAX_RETRIES,
    MAX_TRANSFER_BYTES, PROTOCOL_VERSION,
};
pub use payload::{
    Attributes, ErrorCode, ErrorPayload, ExecEnd, ExecRequest, GetEnd, GetRequest, Hello, HelloAck,
    Pong, PutEnd, PutRequest, ShutdownAck, ShutdownRequest, StatRequest, StatResponse,
    decode_payload, encode_payload,
};
pub use session::{Session, Transport};
