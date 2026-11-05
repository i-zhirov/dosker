use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::thread;
use std::time::{Duration, Instant};

use dosker_agent_proto::Transport;
use dosker_engine::{EngineEndpointKind, EngineError};
use dosker_engine_qemu::EngineStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

fn tcp_kind(port: u16) -> EngineEndpointKind {
    EngineEndpointKind::Tcp {
        host: Ipv4Addr::LOCALHOST,
        port,
    }
}

fn closed_loopback_port() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
    listener.local_addr().expect("read address").port()
}

#[test]
fn stream_connects_to_a_loopback_listener() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
    let port = listener.local_addr().expect("read address").port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept the client");
        stream.write_all(b"greeting").expect("write greeting");
    });

    let mut stream = EngineStream::connect(&tcp_kind(port), Instant::now() + CONNECT_TIMEOUT)
        .expect("connect to the listener");
    let mut buffer = [0u8; 8];
    stream.read_exact(&mut buffer).expect("read the greeting");

    server.join().expect("join the server");
    assert_eq!(&buffer, b"greeting");
}

#[test]
fn stream_connect_stops_at_the_deadline() {
    let port = closed_loopback_port();
    let deadline = Instant::now() + Duration::from_millis(200);
    let started = Instant::now();

    let error = EngineStream::connect(&tcp_kind(port), deadline)
        .expect_err("nothing listens on the closed port");
    let elapsed = started.elapsed();

    match error {
        EngineError::Io(error) => assert_eq!(error.kind(), io::ErrorKind::TimedOut),
        other => panic!("expected an I/O timeout, got {other:?}"),
    }
    assert!(
        elapsed >= Duration::from_millis(200),
        "connect returned before the deadline: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(1200),
        "connect passed the deadline by more than a second: {elapsed:?}"
    );
}

#[test]
fn stream_sets_read_and_write_timeouts() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
    let port = listener.local_addr().expect("read address").port();
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().expect("accept the client");
        thread::sleep(Duration::from_secs(2));
    });

    let mut stream = EngineStream::connect(&tcp_kind(port), Instant::now() + CONNECT_TIMEOUT)
        .expect("connect to the listener");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set the read timeout");
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .expect("set the write timeout");

    let started = Instant::now();
    let mut buffer = [0u8; 1];
    let error = stream
        .read(&mut buffer)
        .expect_err("the server sends nothing");
    let elapsed = started.elapsed();

    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {}
        other => panic!("expected a read timeout, got {other:?}"),
    }
    assert!(
        elapsed >= Duration::from_millis(100),
        "the read timeout was not applied: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "the read timeout did not bound the read: {elapsed:?}"
    );

    stream
        .write_all(b"x")
        .expect("the write timeout permits a small write");
    drop(server);
}

#[test]
fn probe_reports_a_live_tcp_listener() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
    let port = listener.local_addr().expect("read address").port();

    assert!(EngineStream::probe(&tcp_kind(port), Duration::from_secs(1)));
}

#[test]
fn probe_reports_a_closed_tcp_port() {
    let port = closed_loopback_port();

    assert!(!EngineStream::probe(
        &tcp_kind(port),
        Duration::from_millis(250)
    ));
}

#[cfg(unix)]
#[test]
fn probe_reports_a_missing_unix_path() {
    let directory = tempfile::tempdir().expect("create temporary directory");
    let path = directory.path().join("missing.sock");
    let kind = EngineEndpointKind::UnixSocket(path);

    assert!(!EngineStream::probe(&kind, Duration::from_millis(250)));
}
