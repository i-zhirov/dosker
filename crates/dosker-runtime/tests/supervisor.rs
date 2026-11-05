//! The per-container supervisor and its control surface.

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, Write};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::Utc;
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_engine::{
    Capability, CapabilitySet, DebugKind, EngineAdapter, EngineCommand, EngineEndpoint,
    EngineEndpointKind, EngineError, EngineSession, EngineSpec, ExecRequest, ExecResult,
};
use dosker_platform::control::{ControlConnection, connect};
use dosker_platform::process::SystemProcesses;
use dosker_platform::{Endpoint, EndpointRecord};
use dosker_runtime::supervisor::{DEFAULT_READY_TIMEOUT, Supervisor, SupervisorConfig};
use dosker_runtime::{
    ATTACH_POLL, ContainerRecord, CpDirection, ErrorCode, FrameRead, MAX_CP_BYTES, Reply, Request,
    Response, StateStore, read_frame, write_message,
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
        Self::start_with(ready_timeout, false, move |engine| {
            Box::new(MockAdapter::new(engine, Arc::new(SystemProcesses)).with_child_env(env))
        })
    }

    fn start_booter(ready_timeout: Duration) -> Self {
        Self::start_with(ready_timeout, true, |engine| {
            Box::new(MockAdapter::new(engine, Arc::new(SystemProcesses)))
        })
    }

    fn start_with_files(files: BTreeMap<String, Vec<u8>>) -> Self {
        Self::start_with(DEFAULT_READY_TIMEOUT, false, move |engine| {
            Box::new(MockAdapter::new(engine, Arc::new(SystemProcesses)).with_files(files))
        })
    }

    fn start_with_endpoints(make: impl FnOnce(&std::path::Path) -> Vec<EngineEndpoint>) -> Self {
        let dir = tempfile::tempdir().expect("create a temp directory");
        let endpoints = make(dir.path());
        Self::start_full_in(
            dir,
            DEFAULT_READY_TIMEOUT,
            false,
            None,
            true,
            None,
            move |engine| {
                Box::new(
                    MockAdapter::new(engine, Arc::new(SystemProcesses)).with_endpoints(endpoints),
                )
            },
        )
    }

    fn start_with_adapter(adapter: StubAdapter) -> Self {
        Self::start_full(
            DEFAULT_READY_TIMEOUT,
            false,
            None,
            true,
            None,
            move |_engine| Box::new(adapter),
        )
    }

    fn start_booter_adapter(adapter: StubAdapter) -> Self {
        Self::start_full(
            DEFAULT_READY_TIMEOUT,
            true,
            None,
            true,
            None,
            move |_engine| Box::new(adapter),
        )
    }

    fn start_with_command(command: &str, adapter: Option<StubAdapter>) -> Self {
        let command = command.to_owned();
        Self::start_full(
            DEFAULT_READY_TIMEOUT,
            false,
            Some(command),
            true,
            None,
            move |engine| match adapter {
                Some(adapter) => Box::new(adapter),
                None => Box::new(MockAdapter::new(engine, Arc::new(SystemProcesses))),
            },
        )
    }

    fn start_with_serial(
        ready_timeout: Duration,
        serial: &[u8],
        env: Vec<(String, String)>,
    ) -> Self {
        Self::start_full(
            ready_timeout,
            false,
            None,
            true,
            Some(serial),
            move |engine| {
                Box::new(MockAdapter::new(engine, Arc::new(SystemProcesses)).with_child_env(env))
            },
        )
    }

    fn start_without_disk() -> Self {
        Self::start_full(DEFAULT_READY_TIMEOUT, false, None, false, None, |engine| {
            Box::new(MockAdapter::new(engine, Arc::new(SystemProcesses)))
        })
    }

    fn start_with(
        ready_timeout: Duration,
        booter: bool,
        make_adapter: impl FnOnce(PathBuf) -> Box<dyn EngineAdapter>,
    ) -> Self {
        Self::start_full(ready_timeout, booter, None, true, None, make_adapter)
    }

    fn start_full(
        ready_timeout: Duration,
        booter: bool,
        cmd: Option<String>,
        create_disk: bool,
        serial: Option<&[u8]>,
        make_adapter: impl FnOnce(PathBuf) -> Box<dyn EngineAdapter>,
    ) -> Self {
        let dir = tempfile::tempdir().expect("create a temp directory");
        Self::start_full_in(
            dir,
            ready_timeout,
            booter,
            cmd,
            create_disk,
            serial,
            make_adapter,
        )
    }

    fn start_full_in(
        dir: TempDir,
        ready_timeout: Duration,
        booter: bool,
        cmd: Option<String>,
        create_disk: bool,
        serial: Option<&[u8]>,
        make_adapter: impl FnOnce(PathBuf) -> Box<dyn EngineAdapter>,
    ) -> Self {
        let id = ContainerId::generate();
        let name = ContainerName::parse("test-container").expect("valid name");
        let endpoint = dosker_platform::endpoint::control_endpoint(dir.path(), &id);
        let mut record = ContainerRecord::new(
            id.clone(),
            name,
            "freedos:latest".to_owned(),
            "mock".to_owned(),
            endpoint.record(),
            Utc::now(),
        )
        .with_image(String::new(), booter, cmd);
        if create_disk {
            record = record.with_disk(support::make_test_disk(dir.path(), &id));
        }
        let store = StateStore::new(dir.path().to_path_buf());
        store.create(&record).expect("create the record");
        for path in [
            store.paths().engine_log(&record.id),
            store.paths().serial_log(&record.id),
            store.paths().container_log(&record.id),
        ] {
            let _ = fs::remove_file(path);
        }
        if let Some(serial) = serial {
            let mut bytes = serial.to_vec();
            bytes.push(b'\n');
            fs::write(store.paths().serial_log(&record.id), bytes).expect("seed the serial log");
        }

        let root: PathBuf = dir.path().to_path_buf();
        let engine = mock_engine_path().expect("the mock engine binary is available");
        let adapter = make_adapter(engine);
        let handle = std::thread::spawn(move || {
            Supervisor::new(
                SupervisorConfig {
                    root,
                    container_id: id,
                    ready_timeout,
                },
                adapter,
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

    fn root(&self) -> &std::path::Path {
        self._dir.path()
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

fn attach(client: &mut Client) -> Response {
    send(client, &Request::Attach);
    receive(client)
}

fn cp(client: &mut Client, direction: CpDirection, source: &str, destination: &str) -> Response {
    send(
        client,
        &Request::Cp {
            direction,
            source: source.to_owned(),
            destination: destination.to_owned(),
        },
    );
    receive(client)
}

fn assert_error(response: Response, expected: ErrorCode) {
    match response {
        Response {
            ok: false,
            error: Some(error),
            ..
        } => assert_eq!(error.code, expected),
        other => panic!("expected a {expected:?} error, got {other:?}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StubMode {
    ExecTimeout,
    ExecGuestRebooted,
    ExecProtocolError,
    ExecBusy,
    StopTimeout,
    CleanStop,
}

struct StubSession {
    mode: StubMode,
    stop_delay: Duration,
    exec_error: Option<EngineError>,
    put_file_error: Option<EngineError>,
    killed: Arc<AtomicBool>,
}

impl StubSession {
    fn new() -> Self {
        Self::with_mode(StubMode::CleanStop)
    }

    fn with_mode(mode: StubMode) -> Self {
        Self {
            mode,
            stop_delay: Duration::ZERO,
            exec_error: None,
            put_file_error: None,
            killed: Arc::new(AtomicBool::new(false)),
        }
    }
}

struct StubAdapter {
    session: Option<StubSession>,
    killed: Arc<AtomicBool>,
}

impl StubAdapter {
    fn new(session: StubSession) -> Self {
        let killed = Arc::clone(&session.killed);
        Self {
            session: Some(session),
            killed,
        }
    }

    fn kill_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.killed)
    }
}

impl EngineAdapter for StubAdapter {
    fn name(&self) -> &str {
        "stub"
    }

    fn version(&self) -> &str {
        "0.0.0"
    }

    fn capabilities(&self) -> CapabilitySet {
        [Capability::Boot, Capability::Serial, Capability::Stop]
            .into_iter()
            .collect()
    }

    fn probe(&mut self) -> Result<(), EngineError> {
        Ok(())
    }

    fn build_command(&self, _spec: &EngineSpec) -> Result<EngineCommand, EngineError> {
        Ok(EngineCommand {
            program: PathBuf::from("stub-engine"),
            args: vec![String::from("--stub")],
            env: Vec::new(),
        })
    }

    fn start(&mut self, _spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError> {
        let session = self
            .session
            .take()
            .ok_or_else(|| EngineError::Setup(String::from("the stub session already started")))?;
        Ok(Box::new(session))
    }
}

impl EngineSession for StubSession {
    fn pid(&self) -> Option<u32> {
        None
    }

    fn process_group(&self) -> Option<u32> {
        None
    }

    fn ready(&mut self, _timeout: Duration) -> Result<(), EngineError> {
        Ok(())
    }

    fn exec(&mut self, _request: &ExecRequest) -> Result<ExecResult, EngineError> {
        match self.mode {
            StubMode::ExecTimeout => {
                return Err(EngineError::ExecTimeout(Duration::from_millis(1)));
            }
            StubMode::ExecGuestRebooted => return Err(EngineError::GuestRebooted),
            StubMode::ExecProtocolError => {
                return Err(EngineError::Protocol(String::from("boom")));
            }
            StubMode::ExecBusy => {
                return Err(EngineError::Agent {
                    code: String::from("BUSY"),
                    message: String::from("the guest is busy"),
                });
            }
            StubMode::StopTimeout | StubMode::CleanStop => {}
        }
        if let Some(error) = self.exec_error.take() {
            return Err(error);
        }
        Ok(ExecResult {
            exit_code: 0,
            output: String::new(),
            timed_out: false,
            truncated: false,
        })
    }

    fn stop(&mut self, _grace: Duration) -> Result<i32, EngineError> {
        if self.mode == StubMode::StopTimeout {
            return Err(EngineError::StopTimeout(Duration::from_millis(1)));
        }
        std::thread::sleep(self.stop_delay);
        Ok(0)
    }

    fn kill(&mut self) -> Result<i32, EngineError> {
        self.killed.store(true, Ordering::SeqCst);
        Ok(0)
    }

    fn try_exit(&mut self) -> Result<Option<i32>, EngineError> {
        Ok(None)
    }

    fn put_file(&mut self, _path: &str, data: &[u8]) -> Result<u64, EngineError> {
        if let Some(error) = self.put_file_error.take() {
            return Err(error);
        }
        Ok(data.len() as u64)
    }
}

struct RecordingAdapter {
    inner: MockAdapter,
    builds: Arc<AtomicUsize>,
    built_commands: Arc<Mutex<Vec<EngineCommand>>>,
    session_commands: Arc<Mutex<Vec<String>>>,
}

impl EngineAdapter for RecordingAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn version(&self) -> &str {
        self.inner.version()
    }

    fn capabilities(&self) -> CapabilitySet {
        self.inner.capabilities()
    }

    fn probe(&mut self) -> Result<(), EngineError> {
        self.inner.probe()
    }

    fn build_command(&self, spec: &EngineSpec) -> Result<EngineCommand, EngineError> {
        if self.builds.fetch_add(1, Ordering::SeqCst) != 0 {
            return Err(EngineError::Setup(String::from(
                "the engine command was built more than once",
            )));
        }
        let command = self.inner.build_command(spec)?;
        self.built_commands
            .lock()
            .expect("lock the built commands")
            .push(command.clone());
        Ok(command)
    }

    fn start_with_command(
        &mut self,
        spec: &EngineSpec,
        command: &EngineCommand,
    ) -> Result<Box<dyn EngineSession>, EngineError> {
        let session = self.inner.start_with_command(spec, command)?;
        if let Some(rendered) = session.command().map(EngineCommand::render) {
            self.session_commands
                .lock()
                .expect("lock the session commands")
                .push(rendered);
        }
        Ok(session)
    }

    fn start(&mut self, _spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError> {
        Err(EngineError::Setup(String::from(
            "the supervisor must start from the recorded command",
        )))
    }
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
    let engine_log =
        fs::read_to_string(paths.engine_log(&harness.record.id)).expect("read the engine log");
    assert!(
        engine_log.lines().any(|line| line.starts_with("engine ")),
        "the engine log records the command: {engine_log:?}"
    );

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

    send_raw(&mut client, "{\"type\":\"fork\"}\n");
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
fn terminal_exec_reply_is_flushed_before_the_supervisor_exits() {
    for _ in 0..5 {
        let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
        let mut client = harness.connect();

        match exec(&mut client, "SLEEP 5000", 100).result {
            Some(Reply::Exec {
                exit_code,
                timed_out,
                ..
            }) => {
                assert_eq!(exit_code, 124);
                assert!(timed_out);
            }
            other => panic!("expected the terminal exec reply, got {other:?}"),
        }

        assert_eq!(harness.join(), 0);
    }
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

#[test]
fn exec_on_a_booter_returns_no_agent() {
    let mut harness = Harness::start_booter(DEFAULT_READY_TIMEOUT);
    let mut client = harness.connect();

    assert_error(exec(&mut client, "ECHO HELLO", 2000), ErrorCode::NoAgent);

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn attach_rejects_a_booter() {
    let mut harness = Harness::start_booter(DEFAULT_READY_TIMEOUT);
    let mut client = harness.connect();

    assert_error(attach(&mut client), ErrorCode::NoAgent);

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn cp_rejects_a_booter() {
    let mut harness = Harness::start_booter(DEFAULT_READY_TIMEOUT);
    let mut client = harness.connect();

    assert_error(
        cp(&mut client, CpDirection::In, "host.txt", r"C:\GUEST.TXT"),
        ErrorCode::NoAgent,
    );

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn attach_rejects_a_stopped_container() {
    let session = StubSession {
        stop_delay: Duration::from_millis(1500),
        ..StubSession::new()
    };
    let mut harness = Harness::start_with_adapter(StubAdapter::new(session));
    let mut stopper = harness.connect();
    let mut attacher = harness.connect();

    send(&mut stopper, &Request::Stop { grace_ms: 5000 });
    assert!(
        wait_until(Duration::from_secs(2), Duration::from_millis(10), || {
            harness.loaded().status == Status::Stopping
        }),
        "the stop request starts"
    );

    assert_error(attach(&mut attacher), ErrorCode::NotRunning);
    assert_eq!(
        stop_response(&mut stopper),
        Some(Status::Stopped),
        "the stop completes"
    );

    assert_eq!(harness.join(), 0);
}

fn stop_response(client: &mut Client) -> Option<Status> {
    match receive(client).result {
        Some(Reply::Stop { status }) => Some(status),
        other => panic!("expected a stop reply, got {other:?}"),
    }
}

#[test]
fn attach_idle_connection_is_not_closed() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    assert_eq!(attach(&mut client).result, Some(Reply::Attach));
    std::thread::sleep(Duration::from_secs(6));

    match exec(&mut client, "ECHO HELLO", 2000).result {
        Some(Reply::Exec {
            exit_code,
            output,
            timed_out,
            ..
        }) => {
            assert_eq!(exit_code, 0);
            assert_eq!(output, "HELLO\r\n");
            assert!(!timed_out);
        }
        other => panic!("expected an exec reply, got {other:?}"),
    }

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn attach_preserves_a_partial_request_across_a_read_timeout() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    assert_eq!(attach(&mut client).result, Some(Reply::Attach));

    send_raw(&mut client, "{\"type\":\"pi");
    std::thread::sleep(ATTACH_POLL + Duration::from_millis(300));
    send_raw(&mut client, "ng\"}\n");

    assert_eq!(receive(&mut client).result, Some(Reply::Pong));

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn attach_runs_a_slow_command() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    assert_eq!(attach(&mut client).result, Some(Reply::Attach));

    match exec(&mut client, "SLEEP 1000", 5000).result {
        Some(Reply::Exec {
            exit_code,
            timed_out,
            ..
        }) => {
            assert_eq!(exit_code, 0);
            assert!(!timed_out);
        }
        other => panic!("expected an exec reply, got {other:?}"),
    }

    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn stop_succeeds_while_attach_connection_is_idle() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut attached = harness.connect();
    let mut stopper = harness.connect();

    assert_eq!(attach(&mut attached).result, Some(Reply::Attach));

    let started = Instant::now();
    send(&mut stopper, &Request::Stop { grace_ms: 2000 });
    assert_eq!(stop_response(&mut stopper), Some(Status::Stopped));
    let elapsed = started.elapsed();
    assert!(
        elapsed < ATTACH_POLL / 2,
        "stop replied in {elapsed:?}, which must not wait for ATTACH_POLL ({ATTACH_POLL:?})"
    );

    assert!(
        closed(&mut attached),
        "an attached connection closes after stop"
    );

    assert_eq!(harness.join(), 0);
}

#[test]
fn cp_uploads_a_small_file() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();
    let host = tempfile::tempdir().expect("create a temp directory");
    let source = host.path().join("hello.txt");
    fs::write(&source, b"HELLO").expect("write the source file");

    match cp(
        &mut client,
        CpDirection::In,
        source.to_str().expect("a utf-8 path"),
        r"C:\HELLO.TXT",
    )
    .result
    {
        Some(Reply::Cp { bytes }) => assert_eq!(bytes, 5),
        other => panic!("expected a cp reply, got {other:?}"),
    }

    let destination = host.path().join("round-trip.txt");
    match cp(
        &mut client,
        CpDirection::Out,
        r"C:\HELLO.TXT",
        destination.to_str().expect("a utf-8 path"),
    )
    .result
    {
        Some(Reply::Cp { bytes }) => assert_eq!(bytes, 5),
        other => panic!("expected a cp reply, got {other:?}"),
    }
    assert_eq!(fs::read(&destination).expect("read the download"), b"HELLO");

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn cp_downloads_a_small_file() {
    let mut files = BTreeMap::new();
    files.insert(r"C:\HELLO.TXT".to_owned(), b"HELLO".to_vec());
    let mut harness = Harness::start_with_files(files);
    let mut client = harness.connect();
    let host = tempfile::tempdir().expect("create a temp directory");
    let destination = host.path().join("hello.txt");

    match cp(
        &mut client,
        CpDirection::Out,
        r"C:\HELLO.TXT",
        destination.to_str().expect("a utf-8 path"),
    )
    .result
    {
        Some(Reply::Cp { bytes }) => assert_eq!(bytes, 5),
        other => panic!("expected a cp reply, got {other:?}"),
    }
    assert_eq!(fs::read(&destination).expect("read the download"), b"HELLO");

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn cp_rejects_an_upload_over_the_limit() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();
    let host = tempfile::tempdir().expect("create a temp directory");
    let source = host.path().join("large.bin");
    let file = fs::File::create(&source).expect("create the source file");
    file.set_len(MAX_CP_BYTES + 1)
        .expect("extend the source file");

    assert_error(
        cp(
            &mut client,
            CpDirection::In,
            source.to_str().expect("a utf-8 path"),
            r"C:\LARGE.BIN",
        ),
        ErrorCode::TooLarge,
    );

    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn cp_rejects_a_download_over_the_limit() {
    let mut files = BTreeMap::new();
    files.insert(
        r"C:\LARGE.BIN".to_owned(),
        vec![0u8; (MAX_CP_BYTES + 1) as usize],
    );
    let mut harness = Harness::start_with_files(files);
    let mut client = harness.connect();
    let host = tempfile::tempdir().expect("create a temp directory");
    let destination = host.path().join("large.bin");

    assert_error(
        cp(
            &mut client,
            CpDirection::Out,
            r"C:\LARGE.BIN",
            destination.to_str().expect("a utf-8 path"),
        ),
        ErrorCode::TooLarge,
    );
    assert!(!destination.exists(), "the download is not written");

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn cp_maps_an_agent_too_large_error() {
    let session = StubSession {
        put_file_error: Some(EngineError::Agent {
            code: String::from("TOO_LARGE"),
            message: String::from("file is too large"),
        }),
        ..StubSession::new()
    };
    let mut harness = Harness::start_with_adapter(StubAdapter::new(session));
    let mut client = harness.connect();
    let host = tempfile::tempdir().expect("create a temp directory");
    let source = host.path().join("hello.txt");
    fs::write(&source, b"HELLO").expect("write the source file");

    assert_error(
        cp(
            &mut client,
            CpDirection::In,
            source.to_str().expect("a utf-8 path"),
            r"C:\HELLO.TXT",
        ),
        ErrorCode::TooLarge,
    );

    assert_eq!(harness.loaded().status, Status::Running);

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn cp_missing_guest_file_keeps_the_container_running() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();
    let host = tempfile::tempdir().expect("create a temp directory");
    let destination = host.path().join("missing.txt");

    assert_error(
        cp(
            &mut client,
            CpDirection::Out,
            r"C:\MISSING.TXT",
            destination.to_str().expect("a utf-8 path"),
        ),
        ErrorCode::NoFile,
    );

    assert_eq!(harness.loaded().status, Status::Running);
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn cp_host_destination_write_failure_keeps_the_container_running() {
    let mut files = BTreeMap::new();
    files.insert(r"C:\HELLO.TXT".to_owned(), b"HELLO".to_vec());
    let mut harness = Harness::start_with_files(files);
    let mut client = harness.connect();
    let host = tempfile::tempdir().expect("create a temp directory");
    let destination = host.path().join("missing").join("hello.txt");

    match cp(
        &mut client,
        CpDirection::Out,
        r"C:\HELLO.TXT",
        destination.to_str().expect("a utf-8 path"),
    ) {
        Response {
            ok: false,
            error: Some(error),
            ..
        } => {
            assert_eq!(error.code, ErrorCode::Internal);
            assert!(
                error.message.contains("hello.txt"),
                "the error names the path: {error:?}"
            );
        }
        other => panic!("expected an internal error, got {other:?}"),
    }

    assert_eq!(harness.loaded().status, Status::Running);
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn exec_surfaces_guest_rebooted() {
    let session = StubSession {
        exec_error: Some(EngineError::GuestRebooted),
        ..StubSession::new()
    };
    let mut harness = Harness::start_with_adapter(StubAdapter::new(session));
    let mut client = harness.connect();

    assert_error(
        exec(&mut client, "ECHO HELLO", 2000),
        ErrorCode::GuestRebooted,
    );

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Exited);
    assert!(loaded.dirty);

    assert_eq!(harness.join(), 0);
}

#[test]
fn exec_surfaces_agent_busy() {
    let session = StubSession {
        exec_error: Some(EngineError::Agent {
            code: String::from("BUSY"),
            message: String::from("the guest is busy"),
        }),
        ..StubSession::new()
    };
    let mut harness = Harness::start_with_adapter(StubAdapter::new(session));
    let mut client = harness.connect();

    assert_error(exec(&mut client, "ECHO HELLO", 2000), ErrorCode::Busy);
    assert_eq!(harness.loaded().status, Status::Running);

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn screenshot_returns_unsupported() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();
    let host = tempfile::tempdir().expect("create a temp directory");
    let destination = host.path().join("shot.ppm");

    send(
        &mut client,
        &Request::Screenshot {
            path: destination.clone(),
        },
    );
    assert_error(receive(&mut client), ErrorCode::Unsupported);

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn debug_returns_unsupported() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();

    send(
        &mut client,
        &Request::Debug {
            kind: DebugKind::Gdb,
        },
    );
    assert_error(receive(&mut client), ErrorCode::Unsupported);

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_writes_config_before_boot() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    let disk = harness
        .record
        .disk_path
        .clone()
        .expect("the container has a disk");
    let config = support::read_config(&disk);
    assert_eq!(config, b"CONTROL_PORT=2\r\nBAUD=115200\r\n");

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_skips_config_for_a_booter() {
    let mut harness = Harness::start_booter(DEFAULT_READY_TIMEOUT);
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    let disk = harness
        .record
        .disk_path
        .clone()
        .expect("the container has a disk");
    assert!(
        dosker_image::read_file(&disk, dosker_image::CONFIG_PATH).is_err(),
        "a booter disk has no agent configuration"
    );

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_records_engine_endpoints() {
    let mut harness = Harness::start_with_endpoints(|root| {
        vec![
            EngineEndpoint {
                name: String::from("serial_agent"),
                kind: EngineEndpointKind::UnixSocket(root.join("serial_agent.sock")),
            },
            EngineEndpoint {
                name: String::from("qmp"),
                kind: EngineEndpointKind::UnixSocket(root.join("qmp.sock")),
            },
        ]
    });
    let agent = harness.root().join("serial_agent.sock");
    let qmp = harness.root().join("qmp.sock");
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    let sockets = harness.loaded().sockets;
    assert_eq!(
        sockets.get("serial_agent"),
        Some(&EndpointRecord::UnixSocket { path: agent })
    );
    assert_eq!(
        sockets.get("qmp"),
        Some(&EndpointRecord::UnixSocket { path: qmp })
    );

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_records_tcp_engine_endpoints() {
    let mut harness = Harness::start_with_endpoints(|_root| {
        vec![EngineEndpoint {
            name: String::from("qmp"),
            kind: EngineEndpointKind::Tcp {
                host: Ipv4Addr::LOCALHOST,
                port: 45_000,
            },
        }]
    });
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    let sockets = harness.loaded().sockets;
    assert_eq!(
        sockets.get("qmp"),
        Some(&EndpointRecord::Tcp {
            host: Ipv4Addr::LOCALHOST,
            port: 45_000,
        })
    );

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_records_the_engine_command() {
    let mut harness = Harness::start(DEFAULT_READY_TIMEOUT, Vec::new());
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    let loaded = harness.loaded();
    let command = loaded
        .engine_command
        .clone()
        .expect("the record has a rendered command");
    let engine = mock_engine_path().expect("the mock engine binary is available");
    let expected = EngineCommand {
        program: engine,
        args: Vec::new(),
        env: Vec::new(),
    }
    .render();
    assert_eq!(command, expected);
    let engine_log = fs::read_to_string(harness.store.paths().engine_log(&harness.record.id))
        .expect("read the engine log");
    assert!(
        engine_log
            .lines()
            .any(|line| line == format!("engine {command}")),
        "the engine log records the rendered command: {engine_log:?}"
    );

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_executes_the_recorded_command() {
    let builds = Arc::new(AtomicUsize::new(0));
    let built_commands = Arc::new(Mutex::new(Vec::new()));
    let session_commands = Arc::new(Mutex::new(Vec::new()));
    let adapter_builds = Arc::clone(&builds);
    let adapter_built = Arc::clone(&built_commands);
    let adapter_sessions = Arc::clone(&session_commands);

    let mut harness = Harness::start_full(
        DEFAULT_READY_TIMEOUT,
        false,
        None,
        true,
        None,
        move |engine| {
            Box::new(RecordingAdapter {
                inner: MockAdapter::new(engine, Arc::new(SystemProcesses)),
                builds: adapter_builds,
                built_commands: adapter_built,
                session_commands: adapter_sessions,
            })
        },
    );
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    assert_eq!(
        builds.load(Ordering::SeqCst),
        1,
        "the command is built exactly once"
    );
    let loaded = harness.loaded();
    let recorded = loaded
        .engine_command
        .clone()
        .expect("the record has a rendered command");
    let built = built_commands.lock().expect("lock the built commands");
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].render(), recorded);
    let sessions = session_commands.lock().expect("lock the session commands");
    assert_eq!(sessions.first(), Some(&recorded));

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_sends_cmd_after_ready() {
    let mut harness = Harness::start_with_command("ECHO HELLO", None);
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));
    assert_eq!(harness.loaded().status, Status::Running);

    let container_log = fs::read_to_string(harness.store.paths().container_log(&harness.record.id))
        .expect("read the container log");
    let command_lines: Vec<_> = container_log
        .lines()
        .filter(|line| line.starts_with("cmd "))
        .collect();
    assert_eq!(
        command_lines,
        ["cmd exit=0 timed_out=false command=ECHO HELLO"]
    );

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_logs_cmd_busy_and_keeps_running() {
    let adapter = StubAdapter::new(StubSession::with_mode(StubMode::ExecBusy));
    let mut harness = Harness::start_with_command("ECHO HELLO", Some(adapter));
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));
    assert_eq!(harness.loaded().status, Status::Running);

    let container_log = fs::read_to_string(harness.store.paths().container_log(&harness.record.id))
        .expect("read the container log");
    assert!(
        container_log.contains("cmd busy command=ECHO HELLO"),
        "the container log records the busy command: {container_log:?}"
    );

    stop(&mut client);
    assert_eq!(harness.join(), 0);
}

#[test]
fn supervisor_marks_exited_when_cmd_times_out() {
    let mut harness = Harness::start_with_command(
        "SLEEP 5000",
        Some(StubAdapter::new(StubSession::with_mode(
            StubMode::ExecTimeout,
        ))),
    );
    let disk = harness
        .record
        .disk_path
        .clone()
        .expect("the container has a disk");

    assert_eq!(harness.join(), 0);

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Exited);
    assert!(loaded.dirty);
    assert_eq!(loaded.exit_code, Some(124));
    assert_eq!(loaded.pid, None);
    assert_eq!(loaded.engine_pid, None);
    assert_eq!(loaded.engine_pgid, None);
    assert!(loaded.finished_at.is_some());
    assert!(disk.is_file(), "the private disk is kept");
}

#[test]
fn supervisor_marks_exited_dirty_when_stop_must_kill() {
    let adapter = StubAdapter::new(StubSession::with_mode(StubMode::StopTimeout));
    let killed = adapter.kill_flag();
    let mut harness = Harness::start_with_adapter(adapter);
    let disk = harness
        .record
        .disk_path
        .clone()
        .expect("the container has a disk");
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    send(&mut client, &Request::Stop { grace_ms: 2000 });
    assert_eq!(stop_response(&mut client), Some(Status::Exited));
    assert_eq!(harness.join(), 0);

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Exited);
    assert!(loaded.dirty);
    assert_eq!(loaded.pid, None);
    assert_eq!(loaded.engine_pid, None);
    assert_eq!(loaded.engine_pgid, None);
    assert!(
        killed.load(Ordering::SeqCst),
        "the stop timeout kills the engine"
    );
    assert!(disk.is_file(), "the private disk is kept");
}

#[test]
fn supervisor_stops_a_booter_cleanly() {
    let mut harness = Harness::start_booter_adapter(StubAdapter::new(StubSession::with_mode(
        StubMode::CleanStop,
    )));
    let disk = harness
        .record
        .disk_path
        .clone()
        .expect("the container has a disk");
    let mut client = harness.connect();
    assert_eq!(ping(&mut client).result, Some(Reply::Pong));

    send(&mut client, &Request::Stop { grace_ms: 2000 });
    assert_eq!(stop_response(&mut client), Some(Status::Stopped));
    assert_eq!(harness.join(), 0);

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Stopped);
    assert!(!loaded.dirty);
    assert!(disk.is_file(), "the private disk is kept");
}

#[test]
fn supervisor_marks_exited_when_cmd_reboots_the_guest() {
    let mut harness = Harness::start_with_command(
        "ECHO HELLO",
        Some(StubAdapter::new(StubSession::with_mode(
            StubMode::ExecGuestRebooted,
        ))),
    );
    let disk = harness
        .record
        .disk_path
        .clone()
        .expect("the container has a disk");

    assert_eq!(harness.join(), 0);

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Exited);
    assert!(loaded.dirty);
    assert_eq!(loaded.pid, None);
    assert_eq!(loaded.engine_pid, None);
    assert_eq!(loaded.engine_pgid, None);
    assert!(disk.is_file(), "the private disk is kept");
}

#[test]
fn exec_generic_error_kills_the_engine() {
    let adapter = StubAdapter::new(StubSession::with_mode(StubMode::ExecProtocolError));
    let killed = adapter.kill_flag();
    let mut harness = Harness::start_with_command("ECHO HELLO", Some(adapter));

    assert_eq!(harness.join(), 0);

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Exited);
    assert!(loaded.dirty);
    assert!(killed.load(Ordering::SeqCst), "the stub engine is killed");
}

#[test]
fn supervisor_reports_the_last_serial_line_on_boot_timeout() {
    let mut harness = Harness::start_with_serial(
        Duration::from_millis(200),
        b"DOS IS BOOTING",
        vec![(String::from("MOCK_READY_DELAY_MS"), String::from("2000"))],
    );

    assert_eq!(harness.join(), 1);

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Failed);
    let container_log = fs::read_to_string(harness.store.paths().container_log(&harness.record.id))
        .expect("read the container log");
    assert!(
        container_log
            .contains("last serial line: DOS IS BOOTING; retry with a larger --boot-timeout"),
        "the container log names the last serial line: {container_log:?}"
    );
}

#[test]
fn supervisor_reports_a_lossy_last_serial_line_for_binary_data() {
    let mut harness = Harness::start_with_serial(
        Duration::from_millis(200),
        b"DKR1\xff\xfe\x00BINARY",
        vec![(String::from("MOCK_READY_DELAY_MS"), String::from("2000"))],
    );

    assert_eq!(harness.join(), 1);

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Failed);
    assert_eq!(loaded.pid, None);
    assert_eq!(loaded.engine_pid, None);
    let container_log = fs::read_to_string(harness.store.paths().container_log(&harness.record.id))
        .expect("read the container log");
    assert!(
        container_log.contains("last serial line: DKR1"),
        "the container log lossily converts the binary serial line: {container_log:?}"
    );
    assert!(
        container_log.contains('\u{FFFD}'),
        "the invalid UTF-8 is replaced rather than rejecting the line: {container_log:?}"
    );
    assert!(
        !container_log.contains("last serial line: <none>"),
        "the binary serial line is not treated as missing: {container_log:?}"
    );
}

#[test]
fn supervisor_fails_when_the_disk_is_missing() {
    let mut harness = Harness::start_without_disk();

    assert_eq!(harness.join(), 1);

    let loaded = harness.loaded();
    assert_eq!(loaded.status, Status::Failed);
    assert_eq!(loaded.pid, None);
    let container_log = fs::read_to_string(harness.store.paths().container_log(&harness.record.id))
        .expect("read the container log");
    assert!(
        container_log.contains("start failed:"),
        "the container log records the failed start: {container_log:?}"
    );
    assert!(
        container_log.contains(&format!(
            "the container {} has no disk path",
            harness.record.id
        )),
        "the failure names the missing disk: {container_log:?}"
    );
}
