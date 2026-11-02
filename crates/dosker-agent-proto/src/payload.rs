//! JSON payloads carried by protocol frames.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{ProtoError, Result};
use crate::frame::{
    MAX_COMMAND_CHARS, MAX_OUTPUT_BYTES, MAX_PAYLOAD, MAX_TRANSFER_BYTES, PROTOCOL_VERSION,
};

/// Agent handshake announcement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u8,
    pub agent_version: String,
    pub dos: String,
    pub memory_kb: u16,
    pub control_port: u8,
    pub max_payload: u16,
    pub features: Vec<String>,
}

impl Hello {
    /// Check the protocol version and the advertised payload limit.
    pub fn validate(&self) -> Result<()> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(ProtoError::UnsupportedProtocol(self.protocol_version));
        }

        if usize::from(self.max_payload) != MAX_PAYLOAD {
            return Err(ProtoError::PayloadTooLarge {
                length: u32::from(self.max_payload),
            });
        }

        Ok(())
    }
}

/// Host handshake reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAck {
    pub session_id: String,
}

/// Command execution request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecRequest {
    pub command: String,
    pub env: BTreeMap<String, String>,
    pub workdir: String,
    pub max_output_bytes: u32,
}

impl ExecRequest {
    /// Check the command length and the requested output limit.
    pub fn validate(&self) -> Result<()> {
        let length = self.command.chars().count();
        if length > MAX_COMMAND_CHARS {
            return Err(ProtoError::CommandTooLong {
                length,
                max: MAX_COMMAND_CHARS,
            });
        }

        if self.max_output_bytes as usize > MAX_OUTPUT_BYTES {
            return Err(ProtoError::OutputTooLarge {
                length: self.max_output_bytes as usize,
                max: MAX_OUTPUT_BYTES,
            });
        }

        Ok(())
    }
}

/// Command execution result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecEnd {
    pub exit_code: i32,
    pub output_bytes: u64,
    pub truncated: bool,
    pub duration_ms: u64,
    #[serde(default)]
    pub command_not_found: bool,
}

/// File upload request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutRequest {
    pub path: String,
    pub size: u64,
    pub overwrite: bool,
}

impl PutRequest {
    /// Check the file size and the path length.
    pub fn validate(&self) -> Result<()> {
        if self.size > MAX_TRANSFER_BYTES {
            return Err(ProtoError::TransferTooLarge {
                size: self.size,
                max: MAX_TRANSFER_BYTES,
            });
        }

        let length = self.path.chars().count();
        if length > MAX_COMMAND_CHARS {
            return Err(ProtoError::CommandTooLong {
                length,
                max: MAX_COMMAND_CHARS,
            });
        }

        Ok(())
    }
}

/// File upload result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutEnd {
    pub ok: bool,
    pub bytes: u64,
}

/// File download request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRequest {
    pub path: String,
}

/// File download result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEnd {
    pub ok: bool,
    pub size: u64,
}

/// Path status request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatRequest {
    pub path: String,
}

/// DOS file attributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attributes {
    pub read_only: bool,
    pub hidden: bool,
    pub system: bool,
    pub archive: bool,
}

/// Path status response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatResponse {
    pub exists: bool,
    pub size: u64,
    pub directory: bool,
    pub attributes: Attributes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// Liveness reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pong {
    pub agent_version: String,
    pub busy: bool,
}

/// Shutdown request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownRequest {
    pub grace_ms: u64,
}

/// Shutdown reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownAck {
    pub ready: bool,
}

/// Protocol error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    Busy,
    NoFile,
    NoSpace,
    BadPath,
    TooLong,
    TooLarge,
    Unsupported,
    Crc,
    Session,
    Internal,
}

impl ErrorCode {
    /// Return the contract spelling of the code.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "BUSY",
            Self::NoFile => "NO_FILE",
            Self::NoSpace => "NO_SPACE",
            Self::BadPath => "BAD_PATH",
            Self::TooLong => "TOO_LONG",
            Self::TooLarge => "TOO_LARGE",
            Self::Unsupported => "UNSUPPORTED",
            Self::Crc => "CRC",
            Self::Session => "SESSION",
            Self::Internal => "INTERNAL",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Error payload carried by an error frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
    pub request_id: u32,
}

impl fmt::Display for ErrorPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

/// Serialize a payload to JSON bytes.
pub fn encode_payload<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}

/// Deserialize a payload from JSON bytes.
pub fn decode_payload<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    Ok(serde_json::from_slice(bytes)?)
}
