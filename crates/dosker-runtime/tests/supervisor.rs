//! The per-container supervisor and its control surface.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::Utc;
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_platform::Endpoint;
use dosker_platform::control::{ControlConnection, connect};
use dosker_platform::process::SystemProcesses;
use dosker_runtime::supervisor::{DEFAULT_READY_TIMEOUT, Supervisor, SupervisorConfig};
use dosker_runtime::{
    ContainerRecord, ErrorCode, FrameRead, Reply, Request, Response, StateStore, read_frame,
    write_message,
};
use dosker_test_support::fixtures::{kill_process, mock_engine_path, wait_until};
use dosker_test_support::mock_adapter::MockAdapter;
use tempfile::TempDir;

type Client = BufReader<Box<dyn ControlConnection>>;

struct Harness {
    _dir: TempDir,
    store: StateStore,
    record: ContainerRecord,
    endpoint: Endpoint,
    handle: Option<JoinHandle<u8>>,
}

impl Harness {
    fn start(ready_timeout: Duration, env: Vec<(String, String)>) -> Self {
        let dir = tempfile::tempdir().expect("create a temp directory");
        let id = ContainerId::generate();
        let name = ContainerName::parse("test-container").expect("valid name");
        let endpoint = dosker_platform::endpoint::control_endpoint(dir.path(), &id);
        let record = ContainerRecord::new(
            id.clone(),
            name,
            "freedos:latest".to_owned(),
            "mock".to_owned(),
            endpoint.record(),
            Utc::now(),
        );
        let store = StateStore::new(dir.path().to_path_buf());
        store.create(&record).expect("create the record");
        for path in [
            store.paths().engine_log(&record.id),
            store.paths().serial_log(&record.id),
            store.paths().container_log(&record.id),
        ] {
            let _ = fs::remove_file(path);
        }

        let root: PathBuf = dir.path().to_path_buf();
        let engine = mock_engine_path().expect("the mock engine binary is available");
        let handle = std::thread::spawn(move || {
            let adapter = MockAdapter::new(engine, Arc::new(SystemProcesses)).with_child_env(env);
            Supervisor::new(
                SupervisorConfig {
                    root,
                    container_id: id,
                    ready_timeout,
                },
                Box::new(adapter),
            )
            .run()
            .expect("the supervisor runs")
        });

        Self {
            _dir: dir,
            store,
            record,
            endpoint,
            handle: Some(handle),
        }
    }

    fn connect(&self) -> Client {
        let connection =
            connect(&self.endpoint, Duration::from_secs(2)).expect("connect to the supervisor");
        BufReader::new(connection)
    }

    fn join(&mut self) -> u8 {
        self.handle
            .take()
            .expect("the supervisor has not been joined")
            .join()
            .expect("the supervisor thread finishes")
    }

    fn loaded(&self) -> ContainerRecord {
        self.store
            .load(&self.record.id)
            .expect("load the container record")
    }
}

fn send(client: &mut Client, request: &Request) {
    write_message(client.get_mut(), request).expect("write a request");
}

fn send_raw(client: &mut Client, line: &str) {
    client
        .get_mut()
        .write_all(line.as_bytes())
        .expect("write raw bytes");
    client.get_mut().flush().expect("flush the connection");
}

fn receive(client: &mut Client) -> Response {
    match read_frame(client).expect("read a frame") {
        FrameRead::Line(line) => serde_json::from_str(&line).expect("parse a response"),
        FrameRead::Eof => panic!("the connection closed before a response"),
    }
}

fn closed(client: &mut Client) -> bool {
    match read_frame(client) {
        Ok(FrameRead::Eof) => true,
        Ok(FrameRead::Line(_)) => false,
        Err(_) => true,
    }
}

fn ping(client: &mut Client) -> Response {
    send(client, &Request::Ping);
    receive(client)
}

fn stop(client: &mut Client) -> Response {
    send(client, &Request::Stop { grace_ms: 2000 });
    receive(client)
}

fn exec(client: &mut Client, command: &str, timeout_ms: u64) -> Response {
    send(
        client,
        &Request::Exec {
            command: command.to_owned(),
            env: BTreeMap::new(),
            workdir: String::from(r"C:\"),
            timeout_ms,
        },
    );
    receive(client)
}

#[test]
fn supervisor_answers_ping_status_exec_and_stop() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    match receive_after(&mut client, &Request::Status) {
        Some(Reply::Status { container }) => assert_eq!(container.status, Status::Running),
        other => panic!("expected a running status, got {other:?}"),
    }

    match exec(&mut client, "ECHO HELLO", 2000).result {
        Some(Reply::Exec {
            exit_code,
            output,
            timed_out,
            truncated,
        }) => {
            assert_eq!(exit_code, 0);
            assert_eq!(output, "HELLO\r\n");
            assert!(!timed_out);
            assert!(!truncated);
        }
        other => panic!("expected an exec reply, got {other:?}"),
    }

    match stop(&mut client).result {
        Some(Reply::Stop { status }) => assert_eq!(status, Status::Stopped),
        other => panic!("expected a stop reply, got {other:?}"),
    }

    assert_eq!(harness.join(), 0);
    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Stopped);
    assert_eq!(loaded.pid, None);
}

fn receive_after(client: &mut Client, request: &Request) -> Option<Reply> {
    send(client, request);
    receive(client).result
}

#[test]
fn supervisor_records_running_state_before_serving() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Running);
    assert_eq!(loaded.pid, Some(std::process::id()));
    assert!(loaded.engine_pid.is_some());
    assert_eq!(loaded.engine_version, "0.1.0");

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_creates_engine_serial_and_container_logs() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();
    ping(&mut client);

    let paths = harness.store.paths();
    assert!(paths.engine_log(&harness.record.id).is_file());
    assert!(paths.serial_log(&harness.record.id).is_file());
    assert!(paths.container_log(&harness.record.id).is_file());

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_marks_failed_when_engine_never_ready() {
    let mut harness = Harness::start(
        Duration::from_millis(200),
        vec![(String::from("MOCK_READY_DELAY_MS"), String::from("2000"))],
    );

    assert_eq!(harness.join(), 1);

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Failed);
    assert!(loaded.finished_at.is_some());
}

#[test]
fn supervisor_marks_exited_when_engine_process_dies() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    let engine_pid = harness.loaded().engine_pid.expect("the engine has a pid");
    drop(client);
    assert!(kill_process(engine_pid), "the engine process is killed");

    let store = harness.store.clone();
    let id = harness.record.id.clone();
    let exited = wait_until(Duration::from_secs(2), Duration::from_millis(25), || {
        store.load(&id).map(|record| record.status).ok() == Some(Status::Exited)
    });
    assert!(exited, "the supervisor records the engine exit");

    let loaded = harness.loaded();
    assert!(loaded.dirty);
    assert_eq!(loaded.engine_pid, None);
    assert!(loaded.finished_at.is_some());

    assert_eq!(harness.join(), 0);
}

#[test]
fn malformed_frame_closes_connection_without_state_change() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();
    ping(&mut client);

    let before = harness.loaded().status;
    send_raw(&mut client, "{\n");
    assert!(
        closed(&mut client),
        "a malformed frame closes the connection"
    );
    assert_eq!(harness.loaded().status, before);

    let mut client = harness.connect();
    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn unknown_message_returns_unknown_message_and_keeps_connection() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    send_raw(&mut client, "{\"type\":\"attach\"}\n");
    match receive(&mut client) {
        Response {
            ok: false,
            error: Some(error),
            ..
        } => assert_eq!(error.code, ErrorCode::UnknownMessage),
        other => panic!("expected an unknown-message error, got {other:?}"),
    }

    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn one_connection_serves_multiple_requests() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    assert_eq!(ping(&mut client).result, Some(Reply::Pong));
    match receive_after(&mut client, &Request::Status) {
        Some(Reply::Status { container }) => assert_eq!(container.status, Status::Running),
        other => panic!("expected a status reply, got {other:?}"),
    }
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn invalid_body_returns_invalid_message_and_closes() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    send_raw(&mut client, "{\"type\":\"exec\"}\n");
    match receive(&mut client) {
        Response {
            ok: false,
            error: Some(error),
            ..
        } => assert_eq!(error.code, ErrorCode::InvalidMessage),
        other => panic!("expected an invalid-message error, got {other:?}"),
    }
    assert!(closed(&mut client), "an invalid body closes the connection");

    let mut client = harness.connect();
    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn exec_timeout_marks_container_exited_and_replies_timed_out() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    match exec(&mut client, "SLEEP 5000", 100).result {
        Some(Reply::Exec {
            exit_code,
            output,
            timed_out,
            truncated,
        }) => {
            assert_eq!(exit_code, 124);
            assert_eq!(output, "");
            assert!(timed_out);
            assert!(!truncated);
        }
        other => panic!("expected a timed-out exec reply, got {other:?}"),
    }

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Exited);
    assert!(loaded.dirty);

    assert_eq!(harness.join(), 0);
}

#[test]
fn exec_appends_a_line_to_container_log() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    exec(&mut client, "ECHO HELLO", 2000);

    let log_path = harness.store.paths().container_log(&harness.record.id);
    let log = std::fs::read_to_string(log_path).expect("read the container log");
    assert!(
        log.contains("exec exit=0 timed_out=false command=ECHO HELLO"),
        "the container log records the exec: {log:?}"
    );

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}
