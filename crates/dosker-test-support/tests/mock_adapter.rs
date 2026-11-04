//! The mock adapter's engine contract.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dosker_core::ContainerId;
use dosker_engine::{
    Capability, CapabilitySet, EngineAdapter, EngineError, EngineSession, EngineSpec, ExecRequest,
};
use dosker_platform::process::{ProcessSupervisor, SystemProcesses};
use dosker_test_support::fixtures::mock_engine_path;
use dosker_test_support::mock_adapter::MockAdapter;

fn processes() -> Arc<dyn ProcessSupervisor> {
    Arc::new(SystemProcesses)
}

fn adapter(env: Vec<(String, String)>) -> MockAdapter {
    let path: PathBuf = mock_engine_path().expect("the mock engine binary is available");
    MockAdapter::new(path, processes()).with_child_env(env)
}

fn spec(log_dir: &Path) -> EngineSpec {
    EngineSpec {
        root: log_dir.to_path_buf(),
        container_id: ContainerId::generate(),
        workdir: String::from(r"C:\"),
        env: BTreeMap::new(),
        cmd: None,
        expects_agent: false,
        log_dir: log_dir.to_path_buf(),
        disk_path: log_dir.join("disk.img"),
        booter: false,
        display: dosker_engine::DisplayMode::None,
        vnc_port: None,
    }
}

fn start(adapter: &mut MockAdapter, log_dir: &Path) -> Box<dyn EngineSession> {
    adapter.probe().expect("probe the mock engine");
    adapter
        .start(&spec(log_dir))
        .expect("start the mock engine")
}

fn exec(command: &str, timeout_ms: u64) -> ExecRequest {
    ExecRequest {
        command: String::from(command),
        env: BTreeMap::new(),
        workdir: String::from(r"C:\"),
        timeout_ms,
    }
}

#[cfg(unix)]
#[test]
fn start_reports_pid_and_process_group() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let mut adapter = adapter(Vec::new());
    let mut session = start(&mut adapter, root.path());

    let pid = session.pid().expect("the started session has a pid");
    assert_eq!(
        session.process_group(),
        Some(pid),
        "the engine leads its own process group"
    );

    session.kill().expect("kill the mock engine");
}

#[test]
fn ready_returns_after_ready_event() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let mut adapter = adapter(Vec::new());
    let mut session = start(&mut adapter, root.path());

    session
        .ready(Duration::from_secs(2))
        .expect("the engine becomes ready");

    session.kill().expect("kill the mock engine");
}

#[test]
fn ready_times_out_when_delay_exceeds_timeout() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let mut adapter = adapter(vec![(
        String::from("MOCK_READY_DELAY_MS"),
        String::from("2000"),
    )]);
    let mut session = start(&mut adapter, root.path());

    match session.ready(Duration::from_millis(200)) {
        Err(EngineError::ReadyTimeout(_)) => {}
        other => panic!("expected ReadyTimeout, got {other:?}"),
    }
}

#[test]
fn exec_returns_output_and_exit_code() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let mut adapter = adapter(Vec::new());
    let mut session = start(&mut adapter, root.path());
    session
        .ready(Duration::from_secs(2))
        .expect("the engine becomes ready");

    let result = session
        .exec(&exec("ECHO HELLO", 2000))
        .expect("run the command");

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.output, "HELLO\r\n");
    assert!(!result.timed_out);
    assert!(!result.truncated);

    session.kill().expect("kill the mock engine");
}

#[test]
fn exec_timeout_is_reported() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let mut adapter = adapter(Vec::new());
    let mut session = start(&mut adapter, root.path());
    session
        .ready(Duration::from_secs(2))
        .expect("the engine becomes ready");

    match session.exec(&exec("SLEEP 5000", 100)) {
        Err(EngineError::ExecTimeout(_)) => {}
        other => panic!("expected ExecTimeout, got {other:?}"),
    }

    session.kill().expect("kill the mock engine");
}

#[test]
fn stop_returns_exit_code_zero() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let mut adapter = adapter(Vec::new());
    let mut session = start(&mut adapter, root.path());
    session
        .ready(Duration::from_secs(2))
        .expect("the engine becomes ready");

    let code = session
        .stop(Duration::from_secs(2))
        .expect("stop the mock engine");

    assert_eq!(code, 0);
}

#[test]
fn kill_reaps_the_child() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let mut adapter = adapter(Vec::new());
    let mut session = start(&mut adapter, root.path());
    session
        .ready(Duration::from_secs(2))
        .expect("the engine becomes ready");

    let code = session.kill().expect("kill the mock engine");

    #[cfg(unix)]
    assert_eq!(code, 137, "a killed engine reports the signal exit code");
    assert_eq!(
        session.try_exit().expect("poll the reaped engine"),
        Some(code)
    );
}

#[test]
fn try_exit_is_none_while_running() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let mut adapter = adapter(Vec::new());
    let mut session = start(&mut adapter, root.path());
    session
        .ready(Duration::from_secs(2))
        .expect("the engine becomes ready");

    assert_eq!(session.try_exit().expect("poll the running engine"), None);

    session.kill().expect("kill the mock engine");
}

#[test]
fn capabilities_are_boot_serial_and_stop() {
    let expected: CapabilitySet = [Capability::Boot, Capability::Serial, Capability::Stop]
        .into_iter()
        .collect();
    let adapter = adapter(Vec::new());

    assert_eq!(adapter.capabilities(), expected);
    assert!(adapter.capabilities().contains(Capability::Boot));
    assert!(adapter.capabilities().contains(Capability::Serial));
    assert!(adapter.capabilities().contains(Capability::Stop));
    assert!(!adapter.capabilities().contains(Capability::Headless));
}

#[test]
fn mock_session_reports_no_endpoints() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let mut adapter = adapter(Vec::new());
    let mut session = start(&mut adapter, root.path());

    assert!(session.endpoints().is_empty());

    session.kill().expect("kill the mock engine");
}

#[test]
fn mock_adapter_builds_a_command_for_inspection() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let env = vec![(String::from("MOCK_READY_DELAY_MS"), String::from("0"))];
    let mut adapter = adapter(env.clone());
    let spec = spec(root.path());

    let command = adapter
        .build_command(&spec)
        .expect("build the mock command");

    assert_eq!(
        command.program,
        mock_engine_path().expect("the mock engine binary is available")
    );
    assert!(command.args.is_empty());
    assert_eq!(command.env, env);

    let mut session = adapter
        .start_with_command(&spec, &command)
        .expect("start the mock engine");
    assert_eq!(session.command(), Some(&command));

    session.kill().expect("kill the mock engine");
}

#[test]
fn start_fails_when_the_binary_is_missing() {
    let root = tempfile::tempdir().expect("create a temp directory");
    let missing = root.path().join("missing-engine");
    let mut adapter = MockAdapter::new(missing.clone(), processes());

    match adapter.start(&spec(root.path())) {
        Err(EngineError::NotFound(path)) => assert_eq!(path, missing),
        Err(other) => panic!("expected NotFound, got {other:?}"),
        Ok(_) => panic!("expected NotFound, but the engine started"),
    }
}
