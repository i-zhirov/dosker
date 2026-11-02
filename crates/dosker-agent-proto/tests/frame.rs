use dosker_agent_proto::{
    CRC_BYTES, Frame, FrameHeader, FrameType, HEADER_BYTES, MAGIC, MAX_PAYLOAD, PROTOCOL_VERSION,
    ProtoError, crc32,
};

fn encoded(kind: FrameType, request_id: u32, sequence: u32, payload: &[u8]) -> Vec<u8> {
    Frame::new(kind, request_id, sequence, payload.to_vec())
        .expect("frame is valid")
        .encode()
        .expect("frame encodes")
}

#[test]
fn header_is_twenty_bytes_and_fields_are_big_endian() {
    let header = FrameHeader {
        version: PROTOCOL_VERSION,
        kind: FrameType::ExecReq,
        flags: 0,
        reserved: 0,
        request_id: 0x0102_0304,
        sequence: 0x0506_0708,
        payload_length: 0x090A_0B0C,
    };

    let bytes = header.encode();

    assert_eq!(HEADER_BYTES, 20);
    assert_eq!(bytes.len(), 20);
    assert_eq!(&bytes[0..4], b"DKR1");
    assert_eq!(&bytes[8..12], &[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(&bytes[12..16], &[0x05, 0x06, 0x07, 0x08]);
    assert_eq!(&bytes[16..20], &[0x09, 0x0A, 0x0B, 0x0C]);
}

#[test]
fn encode_decode_round_trip() {
    let frame = Frame::new(
        FrameType::ExecData,
        0xAABB_CCDD,
        0x0011_2233,
        b"hello".to_vec(),
    )
    .expect("frame is valid");

    let decoded = Frame::decode(&frame.encode().expect("frame encodes")).expect("frame decodes");

    assert_eq!(decoded.version, PROTOCOL_VERSION);
    assert_eq!(decoded.kind, FrameType::ExecData);
    assert_eq!(decoded.flags, 0);
    assert_eq!(decoded.reserved, 0);
    assert_eq!(decoded.request_id, 0xAABB_CCDD);
    assert_eq!(decoded.sequence, 0x0011_2233);
    assert_eq!(decoded.payload, b"hello");
}

#[test]
fn magic_is_dkr1() {
    assert_eq!(MAGIC, *b"DKR1");
    assert_eq!(&encoded(FrameType::Ping, 0, 0, b"")[0..4], b"DKR1");
}

#[test]
fn version_is_one() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert_eq!(encoded(FrameType::Ping, 0, 0, b"")[4], 1);
}

#[test]
fn payload_limit_accepts_4096() {
    let payload = vec![0x5A; MAX_PAYLOAD];
    let frame = Frame::new(FrameType::PutData, 1, 2, payload.clone()).expect("4096 bytes fit");

    assert_eq!(frame.payload.len(), 4096);
    let decoded = Frame::decode(&frame.encode().expect("frame encodes")).expect("frame decodes");
    assert_eq!(decoded.payload, payload);
}

#[test]
fn payload_limit_rejects_4097() {
    let result = Frame::new(FrameType::PutData, 1, 2, vec![0u8; MAX_PAYLOAD + 1]);

    match result {
        Err(ProtoError::PayloadTooLarge { length }) => assert_eq!(length, 4097),
        other => panic!("expected PayloadTooLarge, got {other:?}"),
    }
}

#[test]
fn crc_covers_header_and_payload() {
    let first = Frame::new(FrameType::ExecData, 1, 0, b"first".to_vec()).expect("frame is valid");
    let second = Frame::new(FrameType::ExecData, 1, 0, b"second".to_vec()).expect("frame is valid");

    assert_ne!(first.crc(), second.crc());
}

#[test]
fn corrupted_payload_fails_crc() {
    let frame =
        Frame::new(FrameType::ExecData, 42, 9, b"payload".to_vec()).expect("frame is valid");
    let mut bytes = frame.encode().expect("frame encodes");
    bytes[HEADER_BYTES + 2] ^= 0xFF;

    match Frame::decode(&bytes) {
        Err(ProtoError::CrcMismatch {
            request_id,
            sequence,
            expected,
            actual,
        }) => {
            assert_eq!(request_id, 42);
            assert_eq!(sequence, 9);
            assert_ne!(expected, actual);
        }
        other => panic!("expected CrcMismatch, got {other:?}"),
    }
}

#[test]
fn flags_must_be_zero() {
    let mut bytes = encoded(FrameType::Ping, 1, 0, b"");
    bytes[6] = 1;

    match Frame::decode(&bytes) {
        Err(ProtoError::UnsupportedFlags(flags)) => assert_eq!(flags, 1),
        other => panic!("expected UnsupportedFlags, got {other:?}"),
    }
}

#[test]
fn reserved_is_ignored() {
    let frame = Frame::new(FrameType::Ping, 7, 3, b"data".to_vec()).expect("frame is valid");
    let mut bytes = frame.encode().expect("frame encodes");
    bytes[7] = 0xAA;

    let crc_offset = HEADER_BYTES + frame.payload.len();
    let crc = crc32(&bytes[..crc_offset]);
    bytes[crc_offset..].copy_from_slice(&crc.to_be_bytes());

    let decoded = Frame::decode(&bytes).expect("reserved byte is ignored");
    assert_eq!(decoded.reserved, 0xAA);
}

#[test]
fn unknown_type_round_trips_the_byte() {
    assert_eq!(FrameType::from_u8(0x99).as_u8(), 0x99);
    assert!(!FrameType::from_u8(0x99).is_known());
    assert!(FrameType::from_u8(0x10).is_known());

    let frame = Frame::new(FrameType::Unknown(0x99), 1, 2, Vec::new()).expect("frame is valid");
    let decoded = Frame::decode(&frame.encode().expect("frame encodes")).expect("frame decodes");

    assert_eq!(decoded.kind, FrameType::Unknown(0x99));
}

#[test]
fn truncated_frame_is_rejected() {
    let bytes = encoded(FrameType::ExecData, 5, 6, b"some payload");
    let short = &bytes[..bytes.len() - 2];

    match Frame::decode(short) {
        Err(ProtoError::TruncatedFrame { expected, actual }) => {
            assert_eq!(expected, bytes.len());
            assert_eq!(actual, bytes.len() - 2);
        }
        other => panic!("expected TruncatedFrame, got {other:?}"),
    }
}

#[test]
fn bad_magic_is_rejected() {
    let mut bytes = encoded(FrameType::Ping, 1, 0, b"");
    bytes[0] = b'X';

    match Frame::decode(&bytes) {
        Err(ProtoError::BadMagic(magic)) => assert_eq!(magic, *b"XKR1"),
        other => panic!("expected BadMagic, got {other:?}"),
    }
}

#[test]
fn unsupported_version_is_rejected() {
    let mut bytes = encoded(FrameType::Ping, 1, 0, b"");
    bytes[4] = 2;

    match Frame::decode(&bytes) {
        Err(ProtoError::UnsupportedVersion(version)) => assert_eq!(version, 2),
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}

#[test]
fn decode_rejects_payload_length_over_limit() {
    let header = FrameHeader {
        version: PROTOCOL_VERSION,
        kind: FrameType::PutData,
        flags: 0,
        reserved: 0,
        request_id: 1,
        sequence: 2,
        payload_length: (MAX_PAYLOAD + 1) as u32,
    };
    let mut bytes = header.encode().to_vec();
    bytes.extend_from_slice(&[0u8; CRC_BYTES]);

    match Frame::decode(&bytes) {
        Err(ProtoError::PayloadTooLarge { length }) => assert_eq!(length, 4097),
        other => panic!("expected PayloadTooLarge, got {other:?}"),
    }
}
