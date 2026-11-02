//! Streaming frame reader and writer.

use std::io::{self, Read, Write};

use crate::crc32::{crc32, crc32_update};
use crate::error::{ProtoError, Result};
use crate::frame::{CRC_BYTES, Frame, FrameHeader, HEADER_BYTES};

/// One result from a streaming frame read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// A complete and valid frame.
    Frame(Frame),
    /// A frame whose header is valid but whose CRC does not match.
    Corrupt { request_id: u32, sequence: u32 },
    /// The reader ended before a frame started.
    Eof,
}

/// Read one frame from a streaming reader.
///
/// The reader can return fewer bytes than requested. A zero-byte first read
/// reports end of stream. A short read after the first byte reports a
/// truncated frame.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Incoming> {
    let mut header_bytes = [0u8; HEADER_BYTES];
    let Some(header_read) = read_until_full(reader, &mut header_bytes)? else {
        return Ok(Incoming::Eof);
    };

    if header_read < HEADER_BYTES {
        return Err(ProtoError::TruncatedFrame {
            expected: HEADER_BYTES,
            actual: header_read,
        });
    }

    let header = FrameHeader::decode(&header_bytes)?;
    let payload_length = header.payload_length as usize;
    let rest_length = payload_length + CRC_BYTES;
    let mut rest = vec![0u8; rest_length];
    let rest_read = read_until_full(reader, &mut rest)?.unwrap_or(0);

    if rest_read < rest_length {
        return Err(ProtoError::TruncatedFrame {
            expected: HEADER_BYTES + rest_length,
            actual: HEADER_BYTES + rest_read,
        });
    }

    let expected_crc = crc32_update(crc32(&header_bytes), &rest[..payload_length]);
    let actual_crc = u32::from_be_bytes([
        rest[payload_length],
        rest[payload_length + 1],
        rest[payload_length + 2],
        rest[payload_length + 3],
    ]);

    if expected_crc != actual_crc {
        return Ok(Incoming::Corrupt {
            request_id: header.request_id,
            sequence: header.sequence,
        });
    }

    Ok(Incoming::Frame(Frame {
        version: header.version,
        kind: header.kind,
        flags: header.flags,
        reserved: header.reserved,
        request_id: header.request_id,
        sequence: header.sequence,
        payload: rest[..payload_length].to_vec(),
    }))
}

/// Write one frame to a streaming writer and flush it.
pub fn write_frame<W: Write>(writer: &mut W, frame: &Frame) -> Result<()> {
    let bytes = frame.encode()?;
    writer.write_all(&bytes)?;
    writer.flush()?;

    Ok(())
}

/// Fill a buffer from a reader.
///
/// Returns `None` when the reader gives zero bytes before any data arrives.
/// Returns the number of bytes read when the stream ends or the buffer is
/// full.
fn read_until_full<R: Read>(reader: &mut R, buffer: &mut [u8]) -> io::Result<Option<usize>> {
    let mut read = 0;

    while read < buffer.len() {
        match reader.read(&mut buffer[read..]) {
            Ok(0) => return Ok(if read == 0 { None } else { Some(read) }),
            Ok(count) => read += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }

    Ok(Some(read))
}
