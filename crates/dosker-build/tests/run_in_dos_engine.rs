use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dosker_build::BuildError;
use dosker_build::run_in_dos::{
    EngineInDosRunner, EngineOpener, InDosRunRequest, InDosRunner, RUN_BOOT_TIMEOUT, RUN_STOP_GRACE,
};
#[cfg(unix)]
use dosker_core::ContainerId;
use dosker_core::EngineName;
use dosker_engine::{
    CapabilitySet, DisplayMode, EngineAdapter, EngineError, EngineSession, EngineSpec, ExecRequest,
    ExecResult,
};
use dosker_image::DosPath;

#[derive(Default)]
struct FakeScript {
    probe: Option<EngineError>,
    ready: Option<EngineError>,
    exec: Option<Result<ExecResult, EngineError>>,
    stop: Option<EngineError>,
}

#[derive(Default)]
struct FakeState {
    opened: Option<EngineName>,
    spec: Option<EngineSpec>,
    ready_timeout: Option<Duration>,
    stop_grace: Option<Duration>,
    exec: Option<ExecRequest>,
    calls: Vec<&'static str>,
    killed: bool,
    exit_code: Option<i32>,
    script: FakeScript,
}

struct FakeOpener {
    state: Arc<Mutex<FakeState>>,
}

impl EngineOpener for FakeOpener {
    fn open(&mut self, engine: EngineName) -> Result<Box<dyn EngineAdapter>, BuildError> {
        self.state.lock().expect("lock the fake state").opened = Some(engine);
        Ok(Box::new(FakeAdapter {
            state: self.state.clone(),
        }))
    }
}

struct FakeAdapter {
    state: Arc<Mutex<FakeState>>,
}

impl EngineAdapter for FakeAdapter {
    fn name(&self) -> &str {
        "fake"
    }

    fn version(&self) -> &str {
        "1.0.0"
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
    }

    fn probe(&mut self) -> Result<(), EngineError> {
        let mut state = self.state.lock().expect("lock the fake state");
        state.calls.push("probe");
        match state.script.probe.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn start(&mut self, spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError> {
        let mut state = self.state.lock().expect("lock the fake state");
        state.calls.push("start");
        state.spec = Some(spec.clone());
        Ok(Box::new(FakeSession {
            state: self.state.clone(),
        }))
    }
}

struct FakeSession {
    state: Arc<Mutex<FakeState>>,
}

impl EngineSession for FakeSession {
    fn pid(&self) -> Option<u32> {
        Some(1)
    }

    fn process_group(&self) -> Option<u32> {
        Some(1)
    }

    fn ready(&mut self, timeout: Duration) -> Result<(), EngineError> {
        let mut state = self.state.lock().expect("lock the fake state");
        state.calls.push("ready");
        state.ready_timeout = Some(timeout);
        match state.script.ready.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn exec(&mut self, request: &ExecRequest) -> Result<ExecResult, EngineError> {
        let mut state = self.state.lock().expect("lock the fake state");
        state.calls.push("exec");
        state.exec = Some(request.clone());
        match state.script.exec.take() {
            Some(result) => result,
            None => Ok(ExecResult {
                exit_code: 0,
                output: String::new(),
                timed_out: false,
                truncated: false,
            }),
        }
    }

    fn stop(&mut self, grace: Duration) -> Result<i32, EngineError> {
        let mut state = self.state.lock().expect("lock the fake state");
        state.calls.push("stop");
        state.stop_grace = Some(grace);
        match state.script.stop.take() {
            Some(error) => Err(error),
            None => Ok(0),
        }
    }

    fn kill(&mut self) -> Result<i32, EngineError> {
        let mut state = self.state.lock().expect("lock the fake state");
        state.calls.push("kill");
        state.killed = true;
        Ok(0)
    }

    fn try_exit(&mut self) -> Result<Option<i32>, EngineError> {
        Ok(self.state.lock().expect("lock the fake state").exit_code)
    }
}

fn runner() -> (EngineInDosRunner, Arc<Mutex<FakeState>>) {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let opener = FakeOpener {
        state: state.clone(),
    };

    (EngineInDosRunner::with_opener(Box::new(opener)), state)
}

struct Fixture {
    dir: tempfile::TempDir,
    workdir: DosPath,
    env: BTreeMap<String, String>,
    disk: PathBuf,
    log_dir: PathBuf,
    serial_ports: u8,
    control_port: u8,
    timeout: Duration,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create a temporary directory");
        let disk = dir.path().join("disk.img");
        let log_dir = dir.path().join("logs");
        Self {
            dir,
            workdir: DosPath::parse("C:\\APP").expect("parse the workdir"),
            env: BTreeMap::from([("NAME".to_owned(), "VALUE".to_owned())]),
            disk,
            log_dir,
            serial_ports: 2,
            control_port: 2,
            timeout: Duration::from_secs(30),
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    fn disk(&self) -> &Path {
        &self.disk
    }

    fn request<'a>(&'a self, command: &'a str) -> InDosRunRequest<'a> {
        self.request_with(command, self.root(), self.disk(), self.log_dir())
    }

    fn request_with<'a>(
        &'a self,
        command: &'a str,
        root: &'a Path,
        disk: &'a Path,
        log_dir: &'a Path,
    ) -> InDosRunRequest<'a> {
        InDosRunRequest {
            line: 7,
            command,
            workdir: &self.workdir,
            env: &self.env,
            timeout: self.timeout,
            boot_timeout: RUN_BOOT_TIMEOUT,
            disk_path: disk,
            disk_interface: "ide",
            serial_ports: self.serial_ports,
            control_port: self.control_port,
            log_dir,
            root,
            engine: EngineName::Qemu,
        }
    }
}

fn recorded_spec(state: &Arc<Mutex<FakeState>>) -> EngineSpec {
    state
        .lock()
        .expect("lock the fake state")
        .spec
        .clone()
        .expect("the adapter recorded a spec")
}

#[test]
fn runner_boots_headless_with_reboot_disabled() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    let request = fixture.request("ECHO HELLO");

    runner.run(&request).expect("run the command");

    let spec = recorded_spec(&state);
    assert_eq!(spec.display, DisplayMode::None);
    assert!(!spec.allow_reboot);
    assert!(!spec.booter);
    assert!(spec.expects_agent);
    assert_eq!(spec.cmd, None);
    assert_eq!(spec.root, fixture.root());
    assert_eq!(spec.log_dir.as_path(), fixture.log_dir());
}

#[test]
fn runner_passes_the_serial_layout() {
    let mut fixture = Fixture::new();
    fixture.serial_ports = 3;
    fixture.control_port = 3;
    let (mut runner, state) = runner();
    let request = fixture.request("ECHO HELLO");

    runner.run(&request).expect("run the command");

    let spec = recorded_spec(&state);
    assert_eq!(spec.serial_ports, 3);
    assert_eq!(spec.control_port, 3);
}

#[test]
fn runner_passes_the_workdir_environment_and_disk() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    let request = fixture.request("ECHO HELLO");

    runner.run(&request).expect("run the command");

    let spec = recorded_spec(&state);
    assert_eq!(spec.workdir, request.workdir.to_dos_string());
    assert_eq!(spec.env, *request.env);
    assert_eq!(spec.disk_path.as_path(), fixture.disk());
    assert_eq!(spec.disk_interface, "ide");

    let exec = state
        .lock()
        .expect("lock the fake state")
        .exec
        .clone()
        .expect("the session recorded the exec request");
    assert_eq!(exec.command, "ECHO HELLO");
    assert_eq!(exec.workdir, "C:\\APP");
    assert_eq!(exec.env, *request.env);
}

#[test]
fn runner_uses_the_command_timeout() {
    let mut fixture = Fixture::new();
    fixture.timeout = Duration::from_secs(45);
    let (mut runner, state) = runner();
    let request = fixture.request("ECHO HELLO");

    runner.run(&request).expect("run the command");

    let exec = state
        .lock()
        .expect("lock the fake state")
        .exec
        .clone()
        .expect("the session recorded the exec request");
    assert_eq!(exec.timeout_ms, 45_000);

    let boot_timeout = state
        .lock()
        .expect("lock the fake state")
        .ready_timeout
        .expect("the session recorded the boot timeout");
    assert_eq!(boot_timeout, RUN_BOOT_TIMEOUT);
}

#[test]
fn runner_returns_the_captured_output() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    state.lock().expect("lock the fake state").script.exec = Some(Ok(ExecResult {
        exit_code: 3,
        output: "HELLO\r\n".to_owned(),
        timed_out: false,
        truncated: true,
    }));
    let request = fixture.request("ECHO HELLO");

    let outcome = runner.run(&request).expect("run the command");

    assert_eq!(outcome.exit_code, 3);
    assert_eq!(outcome.output, "HELLO\r\n");
    assert!(outcome.truncated);
}

#[test]
fn runner_stops_the_guest_after_the_command() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    let request = fixture.request("ECHO HELLO");

    runner.run(&request).expect("run the command");

    let state = state.lock().expect("lock the fake state");
    assert_eq!(state.calls, ["probe", "start", "ready", "exec", "stop"]);
    assert_eq!(state.stop_grace, Some(RUN_STOP_GRACE));
    assert!(!state.killed);
}

#[test]
fn runner_maps_a_command_timeout() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    state.lock().expect("lock the fake state").script.exec =
        Some(Err(EngineError::ExecTimeout(Duration::from_secs(30))));
    let request = fixture.request("ECHO HELLO");

    match runner.run(&request) {
        Err(BuildError::RunTimeout { line }) => assert_eq!(line, request.line),
        other => panic!("expected a RUN timeout, got {other:?}"),
    }

    assert!(state.lock().expect("lock the fake state").killed);
}

#[test]
fn runner_maps_a_guest_reboot() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    state.lock().expect("lock the fake state").script.exec = Some(Err(EngineError::GuestRebooted));
    let request = fixture.request("REBOOT");

    match runner.run(&request) {
        Err(BuildError::GuestRebooted { line }) => assert_eq!(line, request.line),
        other => panic!("expected a guest reboot, got {other:?}"),
    }

    assert!(state.lock().expect("lock the fake state").killed);
}

#[test]
fn runner_maps_a_clean_engine_exit_to_a_guest_reboot() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    {
        let mut state = state.lock().expect("lock the fake state");
        state.script.exec = Some(Err(EngineError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "guest reset",
        ))));
        state.exit_code = Some(0);
    }
    let request = fixture.request("REBOOT");

    match runner.run(&request) {
        Err(BuildError::GuestRebooted { line }) => assert_eq!(line, request.line),
        other => panic!("expected a guest reboot, got {other:?}"),
    }

    assert!(state.lock().expect("lock the fake state").killed);
}

#[test]
fn runner_maps_a_boot_timeout_and_kills_the_vm() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    state.lock().expect("lock the fake state").script.ready =
        Some(EngineError::ReadyTimeout(RUN_BOOT_TIMEOUT));
    let request = fixture.request("ECHO HELLO");

    match runner.run(&request) {
        Err(BuildError::RunBootFailed { line, reason }) => {
            assert_eq!(line, request.line);
            assert!(
                reason.contains("did not boot"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("expected a boot failure, got {other:?}"),
    }

    assert!(state.lock().expect("lock the fake state").killed);
}

#[test]
fn runner_maps_an_engine_exit_before_ready() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    state.lock().expect("lock the fake state").script.ready = Some(EngineError::Exited(3));
    let request = fixture.request("ECHO HELLO");

    match runner.run(&request) {
        Err(BuildError::RunBootFailed { line, reason }) => {
            assert_eq!(line, request.line);
            assert!(reason.contains("exited with code 3"), "reason: {reason}");
        }
        other => panic!("expected a boot failure, got {other:?}"),
    }

    assert!(state.lock().expect("lock the fake state").killed);
}

#[test]
fn runner_maps_a_stop_failure() {
    let fixture = Fixture::new();
    let (mut runner, state) = runner();
    state.lock().expect("lock the fake state").script.stop =
        Some(EngineError::StopTimeout(RUN_STOP_GRACE));
    let request = fixture.request("ECHO HELLO");

    match runner.run(&request) {
        Err(BuildError::RunBootFailed { line, reason }) => {
            assert_eq!(line, request.line);
            assert!(
                reason.contains("did not shut down cleanly"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("expected a boot failure, got {other:?}"),
    }

    assert!(state.lock().expect("lock the fake state").killed);
}

#[test]
fn runner_rejects_a_non_qemu_engine() {
    let fixture = Fixture::new();
    let mut runner = EngineInDosRunner::new();
    let mut request = fixture.request("ECHO HELLO");
    request.engine = EngineName::DosboxX;

    match runner.run(&request) {
        Err(BuildError::Engine { engine, reason }) => {
            assert_eq!(engine, "dosbox-x");
            assert!(reason.contains("qemu"), "unexpected reason: {reason}");
        }
        other => panic!("expected an engine failure, got {other:?}"),
    }
}

#[cfg(unix)]
fn find_root(directory: &Path, length: usize) -> PathBuf {
    let base = directory.to_string_lossy();
    assert!(
        base.len() < length,
        "the temporary directory is already longer than {length} bytes"
    );
    let root = directory.join("r".repeat(length - base.len() - 1));
    assert_eq!(root.to_string_lossy().len(), length);

    root
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .expect("read the directory metadata")
        .permissions()
        .mode()
        & 0o777
}

#[cfg(unix)]
#[test]
fn runner_creates_the_short_socket_directory() {
    let fixture = Fixture::new();
    let base = tempfile::Builder::new()
        .prefix("dosker-engine")
        .tempdir_in("/tmp")
        .expect("create a short temporary directory");
    let root = find_root(base.path(), 200);
    std::fs::create_dir_all(&root).expect("create the long root");
    let disk = root.join("disk.img");
    let log_dir = root.join("logs");
    let (mut runner, state) = runner();
    let request = fixture.request_with("ECHO HELLO", &root, &disk, &log_dir);

    runner.run(&request).expect("run the command");

    let spec = recorded_spec(&state);
    let container = root.join("containers").join(spec.container_id.as_str());
    assert!(container.is_dir(), "the container directory must exist");

    let short = dosker_platform::short_socket_dir();
    assert!(short.is_dir(), "the short socket directory must exist");
    assert_eq!(mode(&short), 0o700, "the short socket directory is private");

    for name in ["agent", "qmp", "monitor", "gdb"] {
        let path = dosker_platform::engine_socket_path(&root, &spec.container_id, name);
        assert!(
            path.starts_with(&short),
            "the {name} endpoint must fall back to the short directory"
        );
    }
}

#[cfg(unix)]
#[test]
fn runner_creates_the_short_socket_directory_for_the_longest_endpoint() {
    let fixture = Fixture::new();
    let base = tempfile::Builder::new()
        .prefix("dosker-engine")
        .tempdir_in("/tmp")
        .expect("create a short temporary directory");
    // The agent path fits, but the monitor path exceeds the limit.
    let root = find_root(base.path(), 64);
    std::fs::create_dir_all(&root).expect("create the root");
    let disk = root.join("disk.img");
    let log_dir = root.join("logs");
    let (mut runner, state) = runner();
    let request = fixture.request_with("ECHO HELLO", &root, &disk, &log_dir);
    let id = ContainerId::generate();

    // Prove the sizing assumption before the run uses a generated id.
    assert!(
        !dosker_platform::engine_socket_path(&root, &id, "agent")
            .starts_with(dosker_platform::short_socket_dir())
    );
    assert!(
        dosker_platform::engine_socket_path(&root, &id, "monitor")
            .starts_with(dosker_platform::short_socket_dir())
    );

    runner.run(&request).expect("run the command");

    let spec = recorded_spec(&state);
    let short = dosker_platform::short_socket_dir();
    assert!(short.is_dir(), "the short socket directory must exist");
    assert_eq!(mode(&short), 0o700, "the short socket directory is private");
    assert!(
        dosker_platform::engine_socket_path(&root, &spec.container_id, "monitor")
            .starts_with(&short),
        "the monitor endpoint must fall back to the short directory"
    );
}
