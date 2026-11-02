#![cfg(unix)]

mod support;

use std::time::Duration;

#[test]
fn handshake_returns_hello() {
    if !support::integration_enabled() {
        return;
    }

    let mut qemu = support::QemuAgent::start().expect("failed to start QEMU");
    let mut client = qemu.connect().expect("failed to connect to the agent");
    let hello = client
        .handshake(Duration::from_secs(60))
        .expect("failed to complete the handshake");

    assert_eq!(hello.protocol_version, 1);
    assert_eq!(hello.agent_version, "1.0.0");
    assert_eq!(hello.control_port, 2);
    assert_eq!(hello.max_payload, 4096);

    for feature in ["exec", "put", "get", "stat"] {
        assert!(
            hello.features.iter().any(|value| value == feature),
            "HELLO lacks the {feature} feature: {:?}",
            hello.features
        );
    }
}

#[test]
fn hello_ack_stores_a_six_hex_session_id() {
    if !support::integration_enabled() {
        return;
    }

    let mut qemu = support::QemuAgent::start().expect("failed to start QEMU");
    let mut client = qemu.connect().expect("failed to connect to the agent");
    client
        .handshake(Duration::from_secs(60))
        .expect("failed to complete the handshake");

    let session_id = client
        .session_id()
        .expect("the handshake did not store a session id");

    assert_eq!(session_id.len(), 6, "session id is {session_id:?}");

    for character in session_id.chars() {
        assert!(
            character.is_ascii_hexdigit() && !character.is_ascii_uppercase(),
            "session id is not lowercase hexadecimal: {session_id:?}"
        );
    }
}

#[test]
fn ping_after_handshake_returns_pong() {
    if !support::integration_enabled() {
        return;
    }

    let mut qemu = support::QemuAgent::start().expect("failed to start QEMU");
    let mut client = qemu.connect().expect("failed to connect to the agent");
    client
        .handshake(Duration::from_secs(60))
        .expect("failed to complete the handshake");

    let pong = client.ping().expect("failed to ping the agent");

    assert_eq!(pong.agent_version, "1.0.0");
    assert!(!pong.busy, "the agent reported busy");
}
