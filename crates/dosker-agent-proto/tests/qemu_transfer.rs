#![cfg(unix)]

mod support;

use std::os::unix::net::UnixStream;
use std::time::Duration;

use dosker_agent_proto::{
    AgentClient, ErrorCode, ErrorPayload, Frame, FrameType, GetEnd, GetRequest, Incoming,
    MAX_PAYLOAD, ProtoError, PutEnd, PutRequest, Result, StatRequest, decode_payload,
    encode_payload, read_frame, write_frame,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const DATA_PATH: &str = "C:\\APP\\DATA.BIN";
const TWO_PATH: &str = "C:\\DOSKER\\TWO.TXT";

fn start() -> (support::QemuAgent, AgentClient<UnixStream>) {
    let mut qemu = support::QemuAgent::start().expect("failed to start QEMU");
    let mut client = qemu.connect().expect("failed to connect to the agent");
    client
        .handshake(HANDSHAKE_TIMEOUT)
        .expect("failed to complete the handshake");

    (qemu, client)
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| (index.wrapping_mul(37).wrapping_add(11) & 0xff) as u8)
        .collect()
}

fn expect_agent_error<T: std::fmt::Debug>(result: Result<T>, code: ErrorCode) {
    match result {
        Err(ProtoError::Agent(error)) => assert_eq!(error.code, code),
        other => panic!("unexpected result: {other:?}"),
    }
}

fn raw_start() -> (support::QemuAgent, UnixStream) {
    let (qemu, client) = start();
    let stream = client.into_inner();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("failed to set the read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .expect("failed to set the write timeout");

    (qemu, stream)
}

fn send_raw(
    stream: &mut UnixStream,
    kind: FrameType,
    request_id: u32,
    sequence: u32,
    payload: &[u8],
) {
    let frame = Frame::new(kind, request_id, sequence, payload.to_vec())
        .expect("failed to build the frame");
    write_frame(stream, &frame).expect("failed to send the frame");
}

fn read_raw(stream: &mut UnixStream) -> Frame {
    match read_frame(stream).expect("failed to read a frame") {
        Incoming::Frame(frame) => frame,
        other => panic!("unexpected incoming data: {other:?}"),
    }
}

fn ack_raw(stream: &mut UnixStream, request_id: u32, sequence: u32) {
    let frame =
        Frame::new(FrameType::Ack, request_id, sequence, Vec::new()).expect("failed to build ACK");
    write_frame(stream, &frame).expect("failed to send ACK");
}

fn expect_raw_ack(stream: &mut UnixStream, request_id: u32, sequence: u32) {
    let frame = read_raw(stream);

    assert_eq!(frame.kind, FrameType::Ack, "unexpected frame: {frame:?}");
    assert_eq!(frame.request_id, request_id);
    assert_eq!(frame.sequence, sequence);
}

fn expect_raw_error(stream: &mut UnixStream, request_id: u32, sequence: u32, code: ErrorCode) {
    let frame = read_raw(stream);

    assert_eq!(frame.kind, FrameType::Error, "unexpected frame: {frame:?}");
    assert_eq!(frame.request_id, request_id);
    assert_eq!(frame.sequence, sequence);

    let error: ErrorPayload = decode_payload(&frame.payload).expect("failed to decode the error");
    assert_eq!(error.code, code);

    ack_raw(stream, request_id, sequence);
}

#[test]
fn put_get_round_trip() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let data = pattern(5000);

    client
        .put(DATA_PATH, &data, false)
        .expect("failed to upload the file");
    let downloaded = client.get(DATA_PATH).expect("failed to download the file");

    assert_eq!(downloaded, data);
}

#[test]
fn put_creates_the_file_and_stat_reports_size() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let data = pattern(5000);

    client
        .put(DATA_PATH, &data, false)
        .expect("failed to upload the file");
    let status = client.stat(DATA_PATH).expect("failed to stat the file");

    assert!(status.exists, "the uploaded file does not exist");
    assert_eq!(status.size, 5000);
    assert!(!status.directory, "the uploaded path is a directory");
    assert!(status.attributes.archive, "the archive bit is not set");
}

#[test]
fn put_without_overwrite_on_an_existing_file_returns_bad_path() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let result = client.put(TWO_PATH, b"NEW", false);

    expect_agent_error(result, ErrorCode::BadPath);
}

#[test]
fn put_with_overwrite_replaces_the_file() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let data = pattern(300);

    client
        .put(TWO_PATH, &data, true)
        .expect("failed to overwrite the file");

    let downloaded = client
        .get(TWO_PATH)
        .expect("failed to download the replaced file");
    let status = client
        .stat(TWO_PATH)
        .expect("failed to stat the replaced file");

    assert_eq!(downloaded, data);
    assert_eq!(status.size, data.len() as u64);
}

#[test]
fn get_missing_file_returns_no_file() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let result = client.get("C:\\APP\\MISSING.BIN");

    expect_agent_error(result, ErrorCode::NoFile);
}

#[test]
fn stat_missing_file_reports_exists_false() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let status = client
        .stat("C:\\APP\\MISSING.BIN")
        .expect("failed to stat the missing file");

    assert!(!status.exists, "the missing path reports exists true");
    assert_eq!(status.size, 0);
    assert!(!status.directory, "the missing path reports a directory");
    assert!(
        status.timestamp.is_none(),
        "the missing path has a timestamp"
    );
}

#[test]
fn stat_directory_reports_directory_true() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let status = client
        .stat("C:\\APP")
        .expect("failed to stat the directory");

    assert!(status.exists, "the directory does not exist");
    assert!(status.directory, "the directory flag is false");
    assert_eq!(status.size, 0);
}

#[test]
fn stat_file_reports_directory_false() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let status = client.stat(TWO_PATH).expect("failed to stat the file");

    assert!(!status.directory, "the file flag reports a directory");
    assert!(status.size > 0, "the file size is zero");
}

#[test]
fn stat_timestamp_ends_with_z() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let status = client.stat(TWO_PATH).expect("failed to stat the file");
    let timestamp = status.timestamp.expect("the file has no timestamp");

    assert!(
        timestamp.ends_with('Z'),
        "the timestamp is not UTC: {timestamp:?}"
    );
}

#[test]
fn put_rejects_a_path_with_a_long_component() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let result = client.put("C:\\APP\\TOOLONGNAME.BIN", b"NEW", false);

    expect_agent_error(result, ErrorCode::BadPath);
}

#[test]
fn duplicate_put_request_is_acknowledged_again() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut stream) = raw_start();
    let request = PutRequest {
        path: "C:\\APP\\DUP.BIN".to_owned(),
        size: 10,
        overwrite: false,
    };
    let payload = encode_payload(&request).expect("failed to encode the request");

    send_raw(&mut stream, FrameType::PutReq, 1000, 0, &payload);
    expect_raw_ack(&mut stream, 1000, 0);

    send_raw(&mut stream, FrameType::PutReq, 1000, 0, &payload);
    expect_raw_ack(&mut stream, 1000, 0);
}

#[test]
fn duplicate_failed_put_request_returns_the_error_again() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut stream) = raw_start();
    let request = PutRequest {
        path: TWO_PATH.to_owned(),
        size: 1,
        overwrite: false,
    };
    let payload = encode_payload(&request).expect("failed to encode the request");

    send_raw(&mut stream, FrameType::PutReq, 1001, 0, &payload);
    expect_raw_error(&mut stream, 1001, 0, ErrorCode::BadPath);

    send_raw(&mut stream, FrameType::PutReq, 1001, 0, &payload);
    expect_raw_error(&mut stream, 1001, 0, ErrorCode::BadPath);
}

#[test]
fn put_data_duplicate_does_not_repeat_bytes() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut stream) = raw_start();
    let path = "C:\\APP\\DUP.DAT";
    let data = pattern(5000);
    let chunks: Vec<&[u8]> = data.chunks(MAX_PAYLOAD).collect();
    assert_eq!(chunks.len(), 2, "the test needs two data chunks");

    let request_id = 1002;
    let request = PutRequest {
        path: path.to_owned(),
        size: data.len() as u64,
        overwrite: false,
    };
    let payload = encode_payload(&request).expect("failed to encode the request");

    send_raw(&mut stream, FrameType::PutReq, request_id, 0, &payload);
    expect_raw_ack(&mut stream, request_id, 0);

    send_raw(&mut stream, FrameType::PutData, request_id, 1, chunks[0]);
    expect_raw_ack(&mut stream, request_id, 1);

    send_raw(&mut stream, FrameType::PutData, request_id, 1, chunks[0]);
    expect_raw_ack(&mut stream, request_id, 1);

    send_raw(&mut stream, FrameType::PutData, request_id, 2, chunks[1]);
    expect_raw_ack(&mut stream, request_id, 2);

    let end = read_raw(&mut stream);
    assert_eq!(end.kind, FrameType::PutEnd, "unexpected frame: {end:?}");
    assert_eq!(end.request_id, request_id);
    assert_eq!(end.sequence, 0);
    let put_end: PutEnd = decode_payload(&end.payload).expect("failed to decode PUT_END");
    assert!(put_end.ok);
    assert_eq!(put_end.bytes, data.len() as u64);
    ack_raw(&mut stream, request_id, 0);

    let get_request_id = 1003;
    let payload = encode_payload(&GetRequest {
        path: path.to_owned(),
    })
    .expect("failed to encode the request");
    send_raw(&mut stream, FrameType::GetReq, get_request_id, 0, &payload);
    expect_raw_ack(&mut stream, get_request_id, 0);

    let mut downloaded = Vec::new();
    loop {
        let frame = read_raw(&mut stream);

        match frame.kind {
            FrameType::GetData => {
                downloaded.extend_from_slice(&frame.payload);
                ack_raw(&mut stream, get_request_id, frame.sequence);
            }
            FrameType::GetEnd => {
                let end: GetEnd = decode_payload(&frame.payload).expect("failed to decode GET_END");
                assert!(end.ok);
                assert_eq!(end.size, data.len() as u64);
                ack_raw(&mut stream, get_request_id, frame.sequence);
                break;
            }
            other => panic!("unexpected frame: {other:?}"),
        }
    }

    assert_eq!(downloaded, data);
}

#[test]
fn put_and_get_empty_file() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let path = "C:\\APP\\EMPTY.BIN";

    let end = client
        .put(path, &[], false)
        .expect("failed to upload the empty file");
    assert!(end.ok);
    assert_eq!(end.bytes, 0);

    let downloaded = client.get(path).expect("failed to download the empty file");
    assert!(downloaded.is_empty(), "the empty file contains data");

    let status = client.stat(path).expect("failed to stat the empty file");
    assert!(status.exists, "the empty file does not exist");
    assert_eq!(status.size, 0);
    assert!(!status.directory, "the empty file is a directory");
}

#[test]
fn overlong_transfer_paths_return_too_long() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut stream) = raw_start();
    let path = format!("C:\\APP\\{}", "A".repeat(122));
    assert!(path.len() > 127, "the path is not over the limit");

    let payload = encode_payload(&PutRequest {
        path: path.clone(),
        size: 1,
        overwrite: false,
    })
    .expect("failed to encode the request");
    send_raw(&mut stream, FrameType::PutReq, 1004, 0, &payload);
    expect_raw_error(&mut stream, 1004, 0, ErrorCode::TooLong);
    assert_eq!(path.len(), 129);

    let payload =
        encode_payload(&GetRequest { path: path.clone() }).expect("failed to encode the request");
    send_raw(&mut stream, FrameType::GetReq, 1005, 0, &payload);
    expect_raw_error(&mut stream, 1005, 0, ErrorCode::TooLong);

    let payload = encode_payload(&StatRequest { path }).expect("failed to encode the request");
    send_raw(&mut stream, FrameType::StatReq, 1006, 0, &payload);
    expect_raw_error(&mut stream, 1006, 0, ErrorCode::TooLong);
}
