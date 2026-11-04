use std::collections::BTreeMap;
use std::io::{self, BufReader, Cursor, Read};
use std::path::PathBuf;

use dosker_core::{ContainerId, ContainerName, Status};
use dosker_engine::DebugKind;
use dosker_runtime::{
    ContainerSummary, CpDirection, DEFAULT_EXEC_TIMEOUT_MS, DEFAULT_STOP_GRACE_MS, ErrorCode,
    FrameError, FrameRead, MAX_MESSAGE_BYTES, Reply, Request, RequestParse, Response,
    parse_request, read_frame, write_message,
};

const EXEC_DEFAULTS: &str = r#"{"type":"exec","command":"ECHO HELLO"}"#;

fn read(bytes: &[u8]) -> Result<FrameRead, FrameError> {
    let mut reader = BufReader::new(Cursor::new(bytes.to_vec()));
    read_frame(&mut reader)
}

fn line(text: &str) -> FrameRead {
    FrameRead::Line(text.to_owned())
}

struct OneByteReader<R>(R);

impl<R: Read> Read for OneByteReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.0.read(&mut buffer[..1])
    }
}

fn sample_id() -> ContainerId {
    ContainerId::parse("0123456789ab").unwrap()
}

fn sample_name() -> ContainerName {
    ContainerName::parse("dos-app").unwrap()
}

fn sample_summary() -> ContainerSummary {
    ContainerSummary {
        id: sample_id(),
        name: sample_name(),
        image: "freedos:latest".to_owned(),
        status: Status::Running,
        pid: Some(4321),
        engine_pid: Some(4322),
        exit_code: None,
    }
}

fn round_trip(response: &Response) -> Response {
    let mut bytes = Vec::new();
    write_message(&mut bytes, response).unwrap();
    assert_eq!(bytes.last(), Some(&b'\n'));
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn read_frame_returns_one_line() {
    assert_eq!(read(b"ping\n").unwrap(), line("ping"));
}

#[test]
fn read_frame_accepts_crlf() {
    assert_eq!(read(b"ping\r\n").unwrap(), line("ping"));
}

#[test]
fn read_frame_reads_each_of_two_lines() {
    let mut reader = BufReader::new(Cursor::new(b"ping\nstatus\n".to_vec()));

    assert_eq!(read_frame(&mut reader).unwrap(), line("ping"));
    assert_eq!(read_frame(&mut reader).unwrap(), line("status"));
    assert_eq!(read_frame(&mut reader).unwrap(), FrameRead::Eof);
}

#[test]
fn read_frame_handles_split_reads() {
    let source = OneByteReader(Cursor::new(b"ping\n".to_vec()));
    let mut reader = BufReader::new(source);

    assert_eq!(read_frame(&mut reader).unwrap(), line("ping"));
}

#[test]
fn read_frame_reports_clean_eof() {
    assert_eq!(read(b"").unwrap(), FrameRead::Eof);
}

#[test]
fn read_frame_skips_blank_lines() {
    assert_eq!(read(b"\n\r\nping\n").unwrap(), line("ping"));
}

#[test]
fn read_frame_accepts_line_at_limit() {
    let mut bytes = vec![b'a'; MAX_MESSAGE_BYTES];
    bytes.push(b'\n');

    match read(&bytes).unwrap() {
        FrameRead::Line(text) => assert_eq!(text.len(), MAX_MESSAGE_BYTES),
        FrameRead::Eof => panic!("expected a line"),
    }
}

#[test]
fn read_frame_accepts_crlf_line_at_limit() {
    let mut bytes = vec![b'a'; MAX_MESSAGE_BYTES];
    bytes.extend_from_slice(b"\r\n");

    match read(&bytes).unwrap() {
        FrameRead::Line(text) => assert_eq!(text.len(), MAX_MESSAGE_BYTES),
        FrameRead::Eof => panic!("expected a line"),
    }
}

#[test]
fn read_frame_rejects_line_over_limit() {
    let mut bytes = vec![b'a'; MAX_MESSAGE_BYTES + 1];
    bytes.push(b'\n');

    assert!(matches!(read(&bytes), Err(FrameError::TooLarge)));
}

#[test]
fn read_frame_rejects_invalid_utf8() {
    assert!(matches!(read(&[0xff, b'\n']), Err(FrameError::NotUtf8)));
}

#[test]
fn parse_request_accepts_each_known_type() {
    assert_eq!(
        parse_request(r#"{"type":"ping"}"#),
        RequestParse::Known(Request::Ping)
    );
    assert_eq!(
        parse_request(r#"{"type":"status"}"#),
        RequestParse::Known(Request::Status)
    );
    assert_eq!(
        parse_request(EXEC_DEFAULTS),
        RequestParse::Known(Request::Exec {
            command: "ECHO HELLO".to_owned(),
            env: BTreeMap::new(),
            workdir: "C:\\".to_owned(),
            timeout_ms: DEFAULT_EXEC_TIMEOUT_MS,
        })
    );
    assert_eq!(
        parse_request(r#"{"type":"stop"}"#),
        RequestParse::Known(Request::Stop {
            grace_ms: DEFAULT_STOP_GRACE_MS,
        })
    );
}

#[test]
fn parse_request_applies_exec_defaults() {
    match parse_request(EXEC_DEFAULTS) {
        RequestParse::Known(Request::Exec {
            env,
            workdir,
            timeout_ms,
            ..
        }) => {
            assert!(env.is_empty());
            assert_eq!(workdir, "C:\\");
            assert_eq!(timeout_ms, 30_000);
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn parse_request_applies_stop_default() {
    match parse_request(r#"{"type":"stop"}"#) {
        RequestParse::Known(Request::Stop { grace_ms }) => assert_eq!(grace_ms, 10_000),
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn parse_request_reports_unknown_type() {
    assert_eq!(parse_request("{}"), RequestParse::UnknownType);
    assert_eq!(
        parse_request(r#"{"type":"fork"}"#),
        RequestParse::UnknownType
    );
    assert_eq!(
        parse_request(r#"{"type":"nope"}"#),
        RequestParse::UnknownType
    );
}

#[test]
fn parse_request_accepts_attach() {
    assert_eq!(
        parse_request(r#"{"type":"attach"}"#),
        RequestParse::Known(Request::Attach)
    );
}

#[test]
fn parse_request_accepts_cp_in_and_out() {
    assert_eq!(
        parse_request(
            r#"{"type":"cp","direction":"in","source":"a.txt","destination":"C:\\A.TXT"}"#
        ),
        RequestParse::Known(Request::Cp {
            direction: CpDirection::In,
            source: "a.txt".to_owned(),
            destination: r"C:\A.TXT".to_owned(),
        })
    );
    assert_eq!(
        parse_request(
            r#"{"type":"cp","direction":"out","source":"C:\\A.TXT","destination":"a.txt"}"#
        ),
        RequestParse::Known(Request::Cp {
            direction: CpDirection::Out,
            source: r"C:\A.TXT".to_owned(),
            destination: "a.txt".to_owned(),
        })
    );
}

#[test]
fn parse_request_accepts_debug_gdb() {
    assert_eq!(
        parse_request(r#"{"type":"debug","kind":"gdb"}"#),
        RequestParse::Known(Request::Debug {
            kind: DebugKind::Gdb,
        })
    );
}

#[test]
fn parse_request_accepts_screenshot() {
    assert_eq!(
        parse_request(r#"{"type":"screenshot","path":"shot.ppm"}"#),
        RequestParse::Known(Request::Screenshot {
            path: PathBuf::from("shot.ppm"),
        })
    );
}

#[test]
fn parse_request_reports_malformed() {
    assert_eq!(parse_request("{"), RequestParse::Malformed);
    assert_eq!(parse_request("[1]"), RequestParse::Malformed);
}

#[test]
fn parse_request_reports_invalid_body() {
    assert_eq!(
        parse_request(r#"{"type":"exec"}"#),
        RequestParse::InvalidBody
    );
    assert_eq!(
        parse_request(r#"{"type":"stop","grace_ms":"x"}"#),
        RequestParse::InvalidBody
    );
}

#[test]
fn response_round_trip_for_each_reply() {
    let replies = [
        Reply::Pong,
        Reply::Status {
            container: sample_summary(),
        },
        Reply::Exec {
            exit_code: 0,
            output: "HELLO\r\n".to_owned(),
            timed_out: false,
            truncated: false,
        },
        Reply::Stop {
            status: Status::Stopped,
        },
    ];

    for reply in replies {
        let response = Response::ok(reply);
        assert_eq!(round_trip(&response), response);
    }
}

#[test]
fn response_round_trip_for_attach_cp_debug_screenshot() {
    let replies = [
        Reply::Attach,
        Reply::Cp { bytes: 5 },
        Reply::Debug {
            kind: DebugKind::Gdb,
            endpoint: "unix:/tmp/gdb.sock".to_owned(),
        },
        Reply::Screenshot {
            path: PathBuf::from("shot.ppm"),
        },
    ];

    for reply in replies {
        let response = Response::ok(reply);
        assert_eq!(round_trip(&response), response);
    }
}

#[test]
fn no_agent_unsupported_too_large_no_file_guest_rebooted_and_busy_codes_are_snake_case() {
    let cases = [
        (ErrorCode::NoAgent, "no_agent"),
        (ErrorCode::Unsupported, "unsupported"),
        (ErrorCode::TooLarge, "too_large"),
        (ErrorCode::NoFile, "no_file"),
        (ErrorCode::GuestRebooted, "guest_rebooted"),
        (ErrorCode::Busy, "busy"),
    ];

    for (code, expected) in cases {
        let value = serde_json::to_value(Response::error(code, "reason")).unwrap();
        assert_eq!(value["error"]["code"], expected);
    }
}

#[test]
fn response_error_omits_result() {
    let response = Response::error(ErrorCode::InvalidMessage, "bad body");
    let text = serde_json::to_string(&response).unwrap();

    assert!(text.contains("\"ok\":false"));
    assert!(!text.contains("\"result\":"));

    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["error"]["code"], "invalid_message");
    assert_eq!(value["error"]["message"], "bad body");
}

#[test]
fn unknown_message_response_uses_unknown_message_code() {
    let response = Response::error(ErrorCode::UnknownMessage, "unknown message type");
    let value = serde_json::to_value(&response).unwrap();

    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "unknown_message");
}

#[test]
fn exec_output_with_newlines_stays_one_physical_line() {
    let response = Response::ok(Reply::Exec {
        exit_code: 0,
        output: "A\r\nB\r\n".to_owned(),
        timed_out: false,
        truncated: false,
    });

    let mut bytes = Vec::new();
    write_message(&mut bytes, &response).unwrap();

    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);

    let text = std::str::from_utf8(&bytes).unwrap();
    let value: serde_json::Value = serde_json::from_str(text.trim_end_matches('\n')).unwrap();
    assert_eq!(value["result"]["output"], "A\r\nB\r\n");
}
