use std::io::{self, Cursor, Read, Write};

use dosker_agent_proto::{
    Frame, FrameType, HEADER_BYTES, Incoming, MAX_PAYLOAD, ProtoError, read_frame, write_frame,
};

fn encoded_frame(kind: FrameType, request_id: u32, sequence: u32, payload: &[u8]) -> Vec<u8> {
    Frame::new(kind, request_id, sequence, payload.to_vec())
        .expect("frame is valid")
        .encode()
        .expect("frame encodes")
}

fn frame(kind: FrameType, request_id: u32, sequence: u32, payload: &[u8]) -> Frame {
    Frame::new(kind, request_id, sequence, payload.to_vec()).expect("frame is valid")
}

struct OneByteReader {
    bytes: Vec<u8>,
    position: usize,
}

impl OneByteReader {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, position: 0 }
    }
}

impl Read for OneByteReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.position == self.bytes.len() {
            return Ok(0);
        }

        if buffer.is_empty() {
            return Ok(0);
        }

        buffer[0] = self.bytes[self.position];
        self.position += 1;
        Ok(1)
    }
}

#[derive(Default)]
struct FlushSpy {
    data: Vec<u8>,
    flushed: bool,
}

impl Write for FlushSpy {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.data.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushed = true;
        Ok(())
    }
}

#[test]
fn read_frame_returns_one_frame() {
    let expected = frame(FrameType::ExecData, 7, 2, b"hello");
    let bytes = expected.encode().expect("frame encodes");
    let mut reader = Cursor::new(bytes);

    let incoming = read_frame(&mut reader).expect("frame reads");

    match incoming {
        Incoming::Frame(actual) => assert_eq!(actual, expected),
        other => panic!("expected a frame, got {other:?}"),
    }
}

#[test]
fn read_frame_handles_split_reads() {
    let expected = frame(FrameType::PutData, 11, 3, b"split reads");
    let mut reader = OneByteReader::new(expected.encode().expect("frame encodes"));

    let incoming = read_frame(&mut reader).expect("frame reads");

    match incoming {
        Incoming::Frame(actual) => assert_eq!(actual, expected),
        other => panic!("expected a frame, got {other:?}"),
    }
}

#[test]
fn read_frame_reports_eof_on_empty_reader() {
    let mut reader = Cursor::new(Vec::new());

    assert!(matches!(
        read_frame(&mut reader).expect("eof is not an error"),
        Incoming::Eof
    ));
}

#[test]
fn read_frame_reports_truncated_header() {
    let bytes = encoded_frame(FrameType::Ping, 1, 0, b"");
    let mut reader = Cursor::new(bytes[..HEADER_BYTES - 4].to_vec());

    match read_frame(&mut reader) {
        Err(ProtoError::TruncatedFrame { expected, actual }) => {
            assert_eq!(expected, HEADER_BYTES);
            assert_eq!(actual, HEADER_BYTES - 4);
        }
        other => panic!("expected a truncated header, got {other:?}"),
    }
}

#[test]
fn read_frame_reports_truncated_payload() {
    let bytes = encoded_frame(FrameType::ExecData, 3, 4, b"payload");
    let mut reader = Cursor::new(bytes[..HEADER_BYTES + 2].to_vec());

    assert!(matches!(
        read_frame(&mut reader),
        Err(ProtoError::TruncatedFrame { .. })
    ));
}

#[test]
fn read_frame_reports_corrupt_crc_with_ids() {
    let mut bytes = encoded_frame(FrameType::ExecData, 0x1234, 9, b"corrupt me");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    let mut reader = Cursor::new(bytes);

    match read_frame(&mut reader).expect("crc failure is not an io error") {
        Incoming::Corrupt {
            request_id,
            sequence,
        } => {
            assert_eq!(request_id, 0x1234);
            assert_eq!(sequence, 9);
        }
        other => panic!("expected a corrupt frame, got {other:?}"),
    }
}

#[test]
fn read_frame_rejects_bad_magic() {
    let mut bytes = encoded_frame(FrameType::Ping, 1, 0, b"");
    bytes[0] = b'X';
    let mut reader = Cursor::new(bytes);

    assert!(matches!(
        read_frame(&mut reader),
        Err(ProtoError::BadMagic(_))
    ));
}

#[test]
fn read_frame_rejects_unsupported_version() {
    let mut bytes = encoded_frame(FrameType::Ping, 1, 0, b"");
    bytes[4] = 2;
    let mut reader = Cursor::new(bytes);

    assert!(matches!(
        read_frame(&mut reader),
        Err(ProtoError::UnsupportedVersion(2))
    ));
}

#[test]
fn read_frame_rejects_payload_over_limit() {
    let mut bytes = encoded_frame(FrameType::Ping, 1, 0, b"");
    let length = (MAX_PAYLOAD as u32) + 1;
    bytes[16..20].copy_from_slice(&length.to_be_bytes());
    let mut reader = Cursor::new(bytes);

    assert!(matches!(
        read_frame(&mut reader),
        Err(ProtoError::PayloadTooLarge { length: 4097 })
    ));
}

#[test]
fn write_frame_then_read_frame_round_trips() {
    let expected = frame(FrameType::StatResp, 0xAABB_CCDD, 5, b"{\"size\":1}");
    let mut writer = Vec::new();

    write_frame(&mut writer, &expected).expect("frame writes");
    let mut reader = Cursor::new(writer);

    match read_frame(&mut reader).expect("frame reads") {
        Incoming::Frame(actual) => assert_eq!(actual, expected),
        other => panic!("expected a frame, got {other:?}"),
    }
}

#[test]
fn write_frame_flushes() {
    let expected = frame(FrameType::Ping, 1, 0, b"");
    let mut writer = FlushSpy::default();

    write_frame(&mut writer, &expected).expect("frame writes");

    assert!(writer.flushed);
    assert_eq!(writer.data, expected.encode().expect("frame encodes"));
}
