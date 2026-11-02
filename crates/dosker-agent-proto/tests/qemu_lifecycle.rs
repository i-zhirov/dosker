#![cfg(unix)]

mod support;

use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use dosker_agent_proto::{
    AgentClient, DEFAULT_TIMEOUT_MS, ErrorCode, FrameType, MAX_RETRIES, ProtoError, read_frame,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const SHUTDOWN_GRACE_MS: u64 = 10_000;

fn start() -> (support::QemuAgent, AgentClient<UnixStream>) {
    let mut qemu = support::QemuAgent::start().expect("failed to start QEMU");
    let mut client = qemu.connect().expect("failed to connect to the agent");
    client
        .handshake(HANDSHAKE_TIMEOUT)
        .expect("failed to complete the handshake");

    (qemu, client)
}

#[test]
fn ping_returns_agent_version_and_busy_false() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let pong = client.ping().expect("failed to ping the agent");

    assert_eq!(pong.agent_version, "1.0.0");
    assert!(!pong.busy, "the agent reported busy");
}

#[test]
fn shutdown_returns_ready() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let ack = client
        .shutdown(SHUTDOWN_GRACE_MS)
        .expect("failed to shut down the agent");

    assert!(ack.ready, "the agent is not ready to stop");
}

#[test]
fn shutdown_stops_the_agent_loop() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let ack = client
        .shutdown(SHUTDOWN_GRACE_MS)
        .expect("failed to shut down the agent");
    assert!(ack.ready, "the agent is not ready to stop");

    let mut stream = client.into_inner();
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("failed to set the read timeout");

    match read_frame(&mut stream) {
        Err(ProtoError::Io(error))
            if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
        other => panic!("unexpected result after shutdown: {other:?}"),
    }
}

#[test]
fn unknown_frame_type_returns_unsupported() {
    if !support::integration_enabled() {
        return;
    }

    let (_qemu, mut client) = start();
    let session = client.session_mut();
    session.begin_request();

    match session.send(FrameType::Unknown(0x55), b"") {
        Err(ProtoError::Agent(error)) => assert_eq!(error.code, ErrorCode::Unsupported),
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn retry_constants_match_the_contract() {
    assert_eq!(DEFAULT_TIMEOUT_MS, 500);
    assert_eq!(MAX_RETRIES, 3);
}
