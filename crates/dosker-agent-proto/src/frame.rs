//! Version 1 frame header, frame types, and limits.

use crate::crc32::{crc32, crc32_update};
use crate::error::{ProtoError, Result};

/// Frame magic bytes.
pub const MAGIC: [u8; 4] = *b"DKR1";
/// Supported protocol version.
pub const PROTOCOL_VERSION: u8 = 1;
/// Encoded header size.
pub const HEADER_BYTES: usize = 20;
/// Encoded CRC trailer size.
pub const CRC_BYTES: usize = 4;
/// Maximum payload size.
pub const MAX_PAYLOAD: usize = 4096;
/// Maximum encoded frame size.
pub const MAX_FRAME_BYTES: usize = HEADER_BYTES + MAX_PAYLOAD + CRC_BYTES;
/// Maximum captured command output.
pub const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Maximum command line length.
pub const MAX_COMMAND_CHARS: usize = 127;
/// Maximum single file transfer size.
pub const MAX_TRANSFER_BYTES: u64 = 16 * 1024 * 1024;
/// Default frame timeout.
pub const DEFAULT_TIMEOUT_MS: u64 = 500;
/// Retries after the first attempt.
pub const MAX_RETRIES: u32 = 3;
/// Agent version reported by the handshake.
pub const AGENT_VERSION: &str = "1.0.0";

/// A protocol frame type.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameType {
    Hello = 0x01,
    HelloAck = 0x02,
    Ack = 0x03,
    Nak = 0x04,
    ExecReq = 0x10,
    ExecData = 0x11,
    ExecEnd = 0x12,
    PutReq = 0x20,
    PutData = 0x21,
    PutEnd = 0x22,
    GetReq = 0x23,
    GetData = 0x24,
    GetEnd = 0x25,
    StatReq = 0x26,
    StatResp = 0x27,
    Ping = 0x30,
    Pong = 0x31,
    Shutdown = 0x40,
    ShutdownAck = 0x41,
    Error = 0x7F,
    Unknown(u8),
}

impl FrameType {
    /// Convert a wire byte to a frame type.
    pub fn from_u8(value: u8) -> Self {
        match value {
            0x01 => Self::Hello,
            0x02 => Self::HelloAck,
            0x03 => Self::Ack,
            0x04 => Self::Nak,
            0x10 => Self::ExecReq,
            0x11 => Self::ExecData,
            0x12 => Self::ExecEnd,
            0x20 => Self::PutReq,
            0x21 => Self::PutData,
            0x22 => Self::PutEnd,
            0x23 => Self::GetReq,
            0x24 => Self::GetData,
            0x25 => Self::GetEnd,
            0x26 => Self::StatReq,
            0x27 => Self::StatResp,
            0x30 => Self::Ping,
            0x31 => Self::Pong,
            0x40 => Self::Shutdown,
            0x41 => Self::ShutdownAck,
            0x7F => Self::Error,
            other => Self::Unknown(other),
        }
    }

    /// Convert a frame type to its wire byte.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Hello => 0x01,
            Self::HelloAck => 0x02,
            Self::Ack => 0x03,
            Self::Nak => 0x04,
            Self::ExecReq => 0x10,
            Self::ExecData => 0x11,
            Self::ExecEnd => 0x12,
            Self::PutReq => 0x20,
            Self::PutData => 0x21,
            Self::PutEnd => 0x22,
            Self::GetReq => 0x23,
            Self::GetData => 0x24,
            Self::GetEnd => 0x25,
            Self::StatReq => 0x26,
            Self::StatResp => 0x27,
            Self::Ping => 0x30,
            Self::Pong => 0x31,
            Self::Shutdown => 0x40,
            Self::ShutdownAck => 0x41,
            Self::Error => 0x7F,
            Self::Unknown(value) => value,
        }
    }

    /// Return true when the type is defined by the protocol.
    pub fn is_known(self) -> bool {
        !matches!(self, Self::Unknown(_))
    }

    /// Return the contract name of the frame type.
    pub fn name(self) -> &'static str {
        match self {
            Self::Hello => "HELLO",
            Self::HelloAck => "HELLO_ACK",
            Self::Ack => "ACK",
            Self::Nak => "NAK",
            Self::ExecReq => "EXEC_REQ",
            Self::ExecData => "EXEC_DATA",
            Self::ExecEnd => "EXEC_END",
            Self::PutReq => "PUT_REQ",
            Self::PutData => "PUT_DATA",
            Self::PutEnd => "PUT_END",
            Self::GetReq => "GET_REQ",
            Self::GetData => "GET_DATA",
            Self::GetEnd => "GET_END",
            Self::StatReq => "STAT_REQ",
            Self::StatResp => "STAT_RESP",
            Self::Ping => "PING",
            Self::Pong => "PONG",
            Self::Shutdown => "SHUTDOWN",
            Self::ShutdownAck => "SHUTDOWN_ACK",
            Self::Error => "ERROR",
            Self::Unknown(_) => "UNKNOWN",
        }
    }
}

/// The version 1 frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub version: u8,
    pub kind: FrameType,
    pub flags: u8,
    pub reserved: u8,
    pub request_id: u32,
    pub sequence: u32,
    pub payload_length: u32,
}

impl FrameHeader {
    /// Encode the header to wire bytes.
    pub fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut bytes = [0u8; HEADER_BYTES];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4] = self.version;
        bytes[5] = self.kind.as_u8();
        bytes[6] = self.flags;
        bytes[7] = self.reserved;
        bytes[8..12].copy_from_slice(&self.request_id.to_be_bytes());
        bytes[12..16].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[16..20].copy_from_slice(&self.payload_length.to_be_bytes());
        bytes
    }

    /// Decode a header from wire bytes.
    ///
    /// The reserved byte is stored but not validated.
    pub fn decode(bytes: &[u8; HEADER_BYTES]) -> Result<Self> {
        if &bytes[0..4] != MAGIC.as_slice() {
            let mut magic = [0u8; 4];
            magic.copy_from_slice(&bytes[0..4]);
            return Err(ProtoError::BadMagic(magic));
        }

        let version = bytes[4];
        if version != PROTOCOL_VERSION {
            return Err(ProtoError::UnsupportedVersion(version));
        }

        let flags = bytes[6];
        if flags != 0 {
            return Err(ProtoError::UnsupportedFlags(flags));
        }

        let payload_length = read_u32(bytes, 16);
        if payload_length as usize > MAX_PAYLOAD {
            return Err(ProtoError::PayloadTooLarge {
                length: payload_length,
            });
        }

        Ok(Self {
            version,
            kind: FrameType::from_u8(bytes[5]),
            flags,
            reserved: bytes[7],
            request_id: read_u32(bytes, 8),
            sequence: read_u32(bytes, 12),
            payload_length,
        })
    }
}

/// A complete protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub version: u8,
    pub kind: FrameType,
    pub flags: u8,
    pub reserved: u8,
    pub request_id: u32,
    pub sequence: u32,
    pub payload: Vec<u8>,
}

impl Frame {
    /// Create a version 1 frame with zero flags and reserved bytes.
    pub fn new(kind: FrameType, request_id: u32, sequence: u32, payload: Vec<u8>) -> Result<Self> {
        if payload.len() > MAX_PAYLOAD {
            return Err(ProtoError::PayloadTooLarge {
                length: payload.len() as u32,
            });
        }

        Ok(Self {
            version: PROTOCOL_VERSION,
            kind,
            flags: 0,
            reserved: 0,
            request_id,
            sequence,
            payload,
        })
    }

    /// Build the frame header.
    pub fn header(&self) -> FrameHeader {
        FrameHeader {
            version: self.version,
            kind: self.kind,
            flags: self.flags,
            reserved: self.reserved,
            request_id: self.request_id,
            sequence: self.sequence,
            payload_length: self.payload.len() as u32,
        }
    }

    /// Compute the CRC over the header and the payload.
    pub fn crc(&self) -> u32 {
        let header = self.header().encode();
        crc32_update(crc32(&header), &self.payload)
    }

    /// Encode the complete frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let header = self.header().encode();
        let crc = self.crc();
        let mut bytes = Vec::with_capacity(HEADER_BYTES + self.payload.len() + CRC_BYTES);
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&self.payload);
        bytes.extend_from_slice(&crc.to_be_bytes());
        Ok(bytes)
    }

    /// Decode a complete frame.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_BYTES {
            return Err(ProtoError::TruncatedFrame {
                expected: HEADER_BYTES,
                actual: bytes.len(),
            });
        }

        let header_bytes: &[u8; HEADER_BYTES] = bytes[..HEADER_BYTES]
            .try_into()
            .expect("slice has the header size");
        let header = FrameHeader::decode(header_bytes)?;

        let payload_length = header.payload_length as usize;
        let expected = HEADER_BYTES + payload_length + CRC_BYTES;
        if bytes.len() < expected {
            return Err(ProtoError::TruncatedFrame {
                expected,
                actual: bytes.len(),
            });
        }

        let crc_offset = HEADER_BYTES + payload_length;
        let expected_crc = crc32(&bytes[..crc_offset]);
        let actual_crc = read_u32(bytes, crc_offset);
        if expected_crc != actual_crc {
            return Err(ProtoError::CrcMismatch {
                request_id: header.request_id,
                sequence: header.sequence,
                expected: expected_crc,
                actual: actual_crc,
            });
        }

        Ok(Self {
            version: header.version,
            kind: header.kind,
            flags: header.flags,
            reserved: header.reserved,
            request_id: header.request_id,
            sequence: header.sequence,
            payload: bytes[HEADER_BYTES..crc_offset].to_vec(),
        })
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
