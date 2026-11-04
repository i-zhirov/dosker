mod support;

use std::time::{Duration, Instant};

use dosker_agent_proto::{
    AGENT_VERSION, AgentClient, ErrorCode, ErrorPayload, ExecEnd, ExecRequest, FrameType, GetEnd,
    GetRequest, HelloAck, MAX_COMMAND_CHARS, MAX_TRANSFER_BYTES, Pong, ProtoError, PutRequest,
    ShutdownAck, ShutdownRequest, StatRequest, decode_payload,
};
use support::mock_agent::{self, MockAgent};

fn client() -> (MockAgent, AgentClient<MockAgent>) {
    let agent = MockAgent::new();
    let probe = agent.clone();
    (probe, AgentClient::new(agent))
}

fn handshaken() -> (MockAgent, AgentClient<MockAgent>) {
    let (agent, mut client) = client();
    agent.queue_hello(mock_agent::hello());
    client
        .handshake(Duration::from_secs(1))
        .expect("handshake succeeds");
    (agent, client)
}

#[test]
fn handshake_accepts_unsolicited_hello() {
    let (agent, mut client) = client();
    let expected = mock_agent::hello();
    agent.queue_hello(expected.clone());

    let hello = client
        .handshake(Duration::from_secs(1))
        .expect("handshake succeeds");

    assert_eq!(hello, expected);
    assert!(client.session_id().is_some());
    assert_eq!(agent.host_frames_of(FrameType::HelloAck).len(), 1);
}

#[test]
fn handshake_pings_when_no_hello_arrives() {
    let (agent, mut client) = client();
    let expected = mock_agent::hello();
    agent.set_ping_response_hello(expected.clone());

    let hello = client
        .handshake(Duration::from_secs(10))
        .expect("handshake succeeds");

    assert_eq!(hello, expected);
    assert_eq!(agent.host_frames_of(FrameType::Ping).len(), 1);
    assert_eq!(agent.host_frames_of(FrameType::HelloAck).len(), 1);
}

#[test]
fn handshake_rejects_unsupported_protocol_version() {
    let (agent, mut client) = client();
    let mut hello = mock_agent::hello();
    hello.protocol_version = 2;
    agent.queue_hello(hello);

    match client.handshake(Duration::from_secs(1)) {
        Err(ProtoError::UnsupportedProtocol(2)) => {}
        other => panic!("expected an unsupported protocol error, got {other:?}"),
    }

    assert!(client.session_id().is_none());
}

#[test]
fn handshake_rejects_a_second_hello() {
    let (agent, mut client) = handshaken();
    agent.queue_hello(mock_agent::hello());

    match client.handshake(Duration::from_secs(1)) {
        Err(ProtoError::GuestRebooted) => {}
        other => panic!("expected a guest reboot, got {other:?}"),
    }
}

#[test]
fn handshake_returns_six_hex_session_id() {
    let (agent, mut client) = client();
    agent.queue_hello(mock_agent::hello());
    client
        .handshake(Duration::from_secs(1))
        .expect("handshake succeeds");

    let session_id = client.session_id().expect("session id is set");
    assert_eq!(session_id.len(), 6);
    assert!(
        session_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );

    let hello_acks = agent.host_frames_of(FrameType::HelloAck);
    let ack: HelloAck = decode_payload(&hello_acks[0].payload).expect("hello ack decodes");
    assert_eq!(ack.session_id, session_id);
}

#[test]
fn handshake_returns_already_handshaken_on_pong() {
    let (agent, mut client) = client();
    agent.queue_pong(Pong {
        agent_version: AGENT_VERSION.to_owned(),
        busy: false,
    });

    match client.handshake(Duration::from_secs(1)) {
        Err(ProtoError::AlreadyHandshaken) => {}
        other => panic!("expected an already handshaken error, got {other:?}"),
    }
}

#[test]
fn handshake_times_out_without_overshooting_the_deadline() {
    let (agent, mut client) = client();
    agent.set_ping_silent();

    let started = Instant::now();
    match client.handshake(Duration::from_millis(100)) {
        Err(ProtoError::HandshakeTimeout) => {}
        other => panic!("expected a handshake timeout, got {other:?}"),
    }

    assert!(started.elapsed() < Duration::from_millis(900));
}

#[test]
fn handshake_ping_path_respects_the_budget() {
    let (agent, mut client) = client();
    agent.set_ping_silent();
    agent.set_read_delay(Duration::from_secs(10));

    let started = Instant::now();
    match client.handshake(Duration::from_millis(2_200)) {
        Err(ProtoError::HandshakeTimeout) => {}
        other => panic!("expected a handshake timeout, got {other:?}"),
    }

    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_secs(2));
    assert!(elapsed < Duration::from_secs(4));
    assert_eq!(agent.host_frames_of(FrameType::Ping).len(), 1);
}

#[test]
fn exec_collects_data_until_end() {
    let (agent, mut client) = handshaken();
    agent.set_exec_output(
        vec![b"hello ".to_vec(), b"world".to_vec()],
        mock_agent::exec_end(0, 11, 340),
    );

    let outcome = client
        .exec(&mock_agent::exec_request(), Duration::from_secs(1))
        .expect("exec succeeds");

    assert_eq!(outcome.output, b"hello world");

    let requests = agent.host_frames_of(FrameType::ExecReq);
    let sent: ExecRequest = decode_payload(&requests[0].payload).expect("exec request decodes");
    assert_eq!(sent.command, "TEST.EXE");
    assert_eq!(
        sent.env.get("PATH").map(String::as_str),
        Some("C:\\APP;C:\\DOS")
    );
    assert_eq!(sent.workdir, "C:\\APP");
    assert_eq!(sent.max_output_bytes, 1_048_576);
}

#[test]
fn exec_returns_the_exit_code_and_duration() {
    let (agent, mut client) = handshaken();
    agent.set_exec_output(
        vec![b"x".to_vec()],
        ExecEnd {
            exit_code: 3,
            output_bytes: 1,
            truncated: false,
            duration_ms: 340,
            command_not_found: false,
        },
    );

    let outcome = client
        .exec(&mock_agent::exec_request(), Duration::from_secs(1))
        .expect("exec succeeds");

    assert_eq!(outcome.exit_code, 3);
    assert_eq!(outcome.duration_ms, 340);
}

#[test]
fn exec_rejects_an_end_output_count_mismatch() {
    let (agent, mut client) = handshaken();
    agent.set_exec_output(
        vec![b"abc".to_vec()],
        ExecEnd {
            exit_code: 0,
            output_bytes: 99,
            truncated: false,
            duration_ms: 1,
            command_not_found: false,
        },
    );

    match client.exec(&mock_agent::exec_request(), Duration::from_secs(1)) {
        Err(ProtoError::ByteCountMismatch { expected, actual }) => {
            assert_eq!(expected, 3);
            assert_eq!(actual, 99);
        }
        other => panic!("expected a byte count mismatch, got {other:?}"),
    }
}

#[test]
fn exec_surfaces_agent_error() {
    let (agent, mut client) = handshaken();
    agent.set_exec_error(ErrorPayload {
        code: ErrorCode::Busy,
        message: "a command is already running".to_owned(),
        request_id: 2,
    });

    match client.exec(&mock_agent::exec_request(), Duration::from_secs(1)) {
        Err(ProtoError::Agent(error)) => {
            assert_eq!(error.code, ErrorCode::Busy);
            assert_eq!(error.message, "a command is already running");
        }
        other => panic!("expected an agent error, got {other:?}"),
    }
}

#[test]
fn command_response_hello_returns_guest_rebooted() {
    let (agent, mut client) = handshaken();
    agent.set_hello_instead_of_ack(mock_agent::hello());

    match client.ping() {
        Err(ProtoError::GuestRebooted) => {}
        other => panic!("expected a guest reboot, got {other:?}"),
    }
}

#[test]
fn exec_rejects_a_command_over_127_chars() {
    let (agent, mut client) = client();
    let mut request = mock_agent::exec_request();
    request.command = "A".repeat(MAX_COMMAND_CHARS + 1);

    match client.exec(&request, Duration::from_secs(1)) {
        Err(ProtoError::CommandTooLong { length, max }) => {
            assert_eq!(length, MAX_COMMAND_CHARS + 1);
            assert_eq!(max, MAX_COMMAND_CHARS);
        }
        other => panic!("expected a command length error, got {other:?}"),
    }

    assert!(agent.host_frames().is_empty());
}

#[test]
fn exec_timeout_returns_timeout() {
    let (agent, mut client) = handshaken();
    agent.set_exec_silent();

    match client.exec(&mock_agent::exec_request(), Duration::from_millis(20)) {
        Err(ProtoError::Timeout) => {}
        other => panic!("expected a timeout, got {other:?}"),
    }
}

#[test]
fn exec_stops_at_the_total_deadline() {
    let (agent, mut client) = handshaken();
    agent.set_exec_output(
        vec![b"one".to_vec(), b"two".to_vec()],
        mock_agent::exec_end(0, 6, 10),
    );
    agent.set_read_delay(Duration::from_millis(40));

    let started = Instant::now();
    match client.exec(&mock_agent::exec_request(), Duration::from_millis(60)) {
        Err(ProtoError::Timeout) => {}
        other => panic!("expected a timeout, got {other:?}"),
    }

    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn exec_send_stops_at_the_deadline() {
    let (agent, mut client) = handshaken();
    agent.set_silent_ack(true);
    agent.set_read_delay(Duration::from_secs(10));

    let started = Instant::now();
    match client.exec(&mock_agent::exec_request(), Duration::from_millis(100)) {
        Err(ProtoError::Timeout) => {}
        other => panic!("expected a timeout, got {other:?}"),
    }

    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn exec_send_write_stops_at_the_deadline() {
    let (agent, mut client) = handshaken();
    agent.set_block_writes();

    let started = Instant::now();
    match client.exec(&mock_agent::exec_request(), Duration::from_millis(100)) {
        Err(ProtoError::Timeout) => {}
        other => panic!("expected a timeout, got {other:?}"),
    }

    assert!(started.elapsed() < Duration::from_millis(900));
}

#[test]
fn exec_rejects_output_over_the_request_limit() {
    let (agent, mut client) = handshaken();
    agent.set_exec_output(
        vec![b"abc".to_vec(), b"def".to_vec()],
        mock_agent::exec_end(0, 6, 10),
    );
    let mut request = mock_agent::exec_request();
    request.max_output_bytes = 4;

    match client.exec(&request, Duration::from_secs(1)) {
        Err(ProtoError::OutputTooLarge { length, max }) => {
            assert_eq!(length, 6);
            assert_eq!(max, 4);
        }
        other => panic!("expected an output size error, got {other:?}"),
    }
}

#[test]
fn put_streams_in_4096_byte_frames() {
    let (agent, mut client) = handshaken();
    let data = vec![0xA5u8; 9000];

    let end = client
        .put("C:\\APP\\DATA.BIN", &data, true)
        .expect("put succeeds");

    assert_eq!(end.bytes, data.len() as u64);
    let frames = agent.host_frames_of(FrameType::PutData);
    assert!(!frames.is_empty());
    assert!(frames.iter().all(|frame| frame.payload.len() <= 4096));
    let total: usize = frames.iter().map(|frame| frame.payload.len()).sum();
    assert_eq!(total, data.len());
}

#[test]
fn put_round_trips_the_size() {
    let (agent, mut client) = handshaken();
    let data = b"0123456789";

    let end = client
        .put("C:\\APP\\DATA.BIN", data, true)
        .expect("put succeeds");

    assert!(end.ok);
    assert_eq!(end.bytes, data.len() as u64);

    let requests = agent.host_frames_of(FrameType::PutReq);
    let request: PutRequest = decode_payload(&requests[0].payload).expect("put request decodes");
    assert_eq!(request.size, data.len() as u64);
    assert!(request.overwrite);
    assert_eq!(request.path, "C:\\APP\\DATA.BIN");
}

#[test]
fn put_rejects_data_over_16_mib() {
    let (agent, mut client) = client();
    let data = vec![0u8; MAX_TRANSFER_BYTES as usize + 1];

    match client.put("C:\\APP\\BIG.BIN", &data, false) {
        Err(ProtoError::TransferTooLarge { size, max }) => {
            assert_eq!(size, MAX_TRANSFER_BYTES + 1);
            assert_eq!(max, MAX_TRANSFER_BYTES);
        }
        other => panic!("expected a transfer size error, got {other:?}"),
    }

    assert!(agent.host_frames().is_empty());
}

#[test]
fn put_rejects_a_path_over_127_chars() {
    let (agent, mut client) = client();
    let path = format!("C:\\{}", "A".repeat(MAX_COMMAND_CHARS));

    match client.put(&path, b"abc", true) {
        Err(ProtoError::CommandTooLong { length, max }) => {
            assert_eq!(length, MAX_COMMAND_CHARS + 3);
            assert_eq!(max, MAX_COMMAND_CHARS);
        }
        other => panic!("expected a path length error, got {other:?}"),
    }

    assert!(agent.host_frames().is_empty());
}

#[test]
fn put_surfaces_an_agent_internal_error() {
    let (agent, mut client) = handshaken();
    agent.set_put_end_ok(false);

    match client.put("C:\\APP\\DATA.BIN", b"abc", true) {
        Err(ProtoError::Agent(error)) => assert_eq!(error.code, ErrorCode::Internal),
        other => panic!("expected an agent error, got {other:?}"),
    }
}

#[test]
fn put_rejects_an_end_size_mismatch() {
    let (agent, mut client) = handshaken();
    agent.set_put_end_bytes(99);

    match client.put("C:\\APP\\DATA.BIN", b"abc", true) {
        Err(ProtoError::ByteCountMismatch { expected, actual }) => {
            assert_eq!(expected, 3);
            assert_eq!(actual, 99);
        }
        other => panic!("expected a byte count mismatch, got {other:?}"),
    }
}

#[test]
fn get_returns_the_file_bytes() {
    let (agent, mut client) = handshaken();
    agent.set_get_output(
        vec![b"abc".to_vec(), b"def".to_vec()],
        GetEnd { ok: true, size: 6 },
    );

    let bytes = client.get("C:\\APP\\OUT.BIN").expect("get succeeds");

    assert_eq!(bytes, b"abcdef");

    let requests = agent.host_frames_of(FrameType::GetReq);
    let sent: GetRequest = decode_payload(&requests[0].payload).expect("get request decodes");
    assert_eq!(sent.path, "C:\\APP\\OUT.BIN");
}

#[test]
fn get_rejects_a_file_over_16_mib() {
    let (agent, mut client) = handshaken();
    let chunk = vec![0u8; 4096];
    let count = (MAX_TRANSFER_BYTES as usize / 4096) + 1;
    let size = (count * 4096) as u64;
    agent.set_get_output(vec![chunk; count], GetEnd { ok: true, size });

    match client.get("C:\\APP\\BIG.BIN") {
        Err(ProtoError::TransferTooLarge { size, max }) => {
            assert!(size > max);
            assert_eq!(max, MAX_TRANSFER_BYTES);
        }
        other => panic!("expected a transfer size error, got {other:?}"),
    }
}

#[test]
fn get_surfaces_an_agent_internal_error() {
    let (agent, mut client) = handshaken();
    agent.set_get_output(vec![b"abc".to_vec()], GetEnd { ok: false, size: 3 });

    match client.get("C:\\APP\\OUT.BIN") {
        Err(ProtoError::Agent(error)) => assert_eq!(error.code, ErrorCode::Internal),
        other => panic!("expected an agent error, got {other:?}"),
    }
}

#[test]
fn get_rejects_an_end_size_mismatch() {
    let (agent, mut client) = handshaken();
    agent.set_get_output(vec![b"abc".to_vec()], GetEnd { ok: true, size: 99 });

    match client.get("C:\\APP\\OUT.BIN") {
        Err(ProtoError::ByteCountMismatch { expected, actual }) => {
            assert_eq!(expected, 3);
            assert_eq!(actual, 99);
        }
        other => panic!("expected a byte count mismatch, got {other:?}"),
    }
}

#[test]
fn get_rejects_a_path_over_127_chars() {
    let (_agent, mut client) = client();
    let path = format!("C:\\{}", "A".repeat(MAX_COMMAND_CHARS));

    match client.get(&path) {
        Err(ProtoError::CommandTooLong { length, max }) => {
            assert_eq!(length, MAX_COMMAND_CHARS + 3);
            assert_eq!(max, MAX_COMMAND_CHARS);
        }
        other => panic!("expected a path length error, got {other:?}"),
    }
}

#[test]
fn stat_returns_size_attributes_and_directory_false() {
    let (agent, mut client) = handshaken();
    let expected = mock_agent::stat_response();
    agent.set_stat_response(expected.clone());

    let actual = client.stat("C:\\APP\\APP.EXE").expect("stat succeeds");

    assert_eq!(actual, expected);
    assert_eq!(actual.size, 12_345);
    assert!(!actual.directory);
    assert!(!actual.attributes.read_only);
    assert!(!actual.attributes.hidden);
    assert!(!actual.attributes.system);
    assert!(actual.attributes.archive);

    let requests = agent.host_frames_of(FrameType::StatReq);
    let sent: StatRequest = decode_payload(&requests[0].payload).expect("stat request decodes");
    assert_eq!(sent.path, "C:\\APP\\APP.EXE");
}

#[test]
fn stat_rejects_a_path_over_127_chars() {
    let (_agent, mut client) = client();
    let path = format!("C:\\{}", "A".repeat(MAX_COMMAND_CHARS));

    match client.stat(&path) {
        Err(ProtoError::CommandTooLong { length, max }) => {
            assert_eq!(length, MAX_COMMAND_CHARS + 3);
            assert_eq!(max, MAX_COMMAND_CHARS);
        }
        other => panic!("expected a path length error, got {other:?}"),
    }
}

#[test]
fn ping_returns_agent_version_and_busy() {
    let (_agent, mut client) = handshaken();

    let pong = client.ping().expect("ping succeeds");

    assert_eq!(pong.agent_version, "1.0.0");
    assert!(!pong.busy);
}

#[test]
fn shutdown_returns_ready() {
    let (agent, mut client) = handshaken();
    agent.set_shutdown_ack(ShutdownAck { ready: true });

    let ack = client.shutdown(10_000).expect("shutdown succeeds");

    assert!(ack.ready);
    let frames = agent.host_frames_of(FrameType::Shutdown);
    assert_eq!(frames.len(), 1);
    let request: ShutdownRequest =
        decode_payload(&frames[0].payload).expect("shutdown request decodes");
    assert_eq!(request.grace_ms, 10_000);
}

#[test]
fn shutdown_with_timeout_waits_for_a_delayed_ack() {
    let (agent, mut client) = handshaken();
    agent.set_shutdown_ack_delay(Duration::from_secs(1));

    let ack = client
        .shutdown_with_timeout(10_000, Duration::from_secs(10))
        .expect("shutdown succeeds");

    assert!(ack.ready);
}

#[test]
fn shutdown_with_timeout_fails_when_the_ack_misses_the_deadline() {
    let (agent, mut client) = handshaken();
    agent.set_shutdown_ack_delay(Duration::from_secs(1));

    let started = Instant::now();
    match client.shutdown_with_timeout(10_000, Duration::from_millis(100)) {
        Err(ProtoError::Timeout) => {}
        other => panic!("expected a shutdown timeout, got {other:?}"),
    }

    assert!(started.elapsed() < Duration::from_millis(900));
}

#[test]
fn agent_error_codes_are_uppercase() {
    let codes = [
        ErrorCode::Busy,
        ErrorCode::NoFile,
        ErrorCode::NoSpace,
        ErrorCode::BadPath,
        ErrorCode::TooLong,
        ErrorCode::TooLarge,
        ErrorCode::Unsupported,
        ErrorCode::Crc,
        ErrorCode::Session,
        ErrorCode::Internal,
    ];

    for code in codes {
        let text = code.as_str();
        assert_eq!(text, text.to_ascii_uppercase());
        assert!(!text.bytes().any(|byte| byte.is_ascii_lowercase()));
        let json = serde_json::to_string(&code).expect("code serializes");
        assert_eq!(json, format!("\"{text}\""));
    }
}
