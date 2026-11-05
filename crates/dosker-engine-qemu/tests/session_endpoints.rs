use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::{Duration, Instant};

use dosker_core::ContainerId;
#[cfg(windows)]
use dosker_engine::{DebugKind, EngineError};
use dosker_engine::{EngineCommand, EngineSession, EngineSpec};
use dosker_engine_qemu::{QemuBinary, QemuEndpoints, QemuSession, QemuVersion};
use dosker_platform::process::system;

fn spec(root: &Path, booter: bool) -> EngineSpec {
    EngineSpec {
        root: root.to_path_buf(),
        container_id: ContainerId::generate(),
        workdir: "C:\\".to_owned(),
        env: BTreeMap::new(),
        cmd: None,
        expects_agent: !booter,
        log_dir: root.to_path_buf(),
        disk_path: root.join("disk.img"),
        booter,
        display: Default::default(),
        vnc_port: None,
    }
}

fn binary() -> QemuBinary {
    QemuBinary {
        path: PathBuf::from("qemu-system-i386"),
        version: QemuVersion::new(8, 0, 0),
    }
}

fn stand_in_program() -> (PathBuf, Vec<String>) {
    #[cfg(unix)]
    {
        (PathBuf::from("sleep"), vec!["30".to_owned()])
    }

    #[cfg(windows)]
    {
        (
            PathBuf::from("cmd"),
            vec!["/C".to_owned(), "ping -n 30 127.0.0.1 > NUL".to_owned()],
        )
    }
}

fn stand_in_session(spec: &EngineSpec) -> QemuSession {
    let endpoints = QemuEndpoints::for_spec(spec).expect("resolve the engine endpoints");
    let (program, args) = stand_in_program();
    let command = EngineCommand {
        program,
        args,
        env: Vec::new(),
        endpoints: endpoints.engine_endpoints(),
    };

    QemuSession::spawn(&binary(), spec, &system(), command).expect("start the stand-in child")
}

#[test]
fn session_reports_the_endpoints_from_its_command() {
    let directory = tempfile::tempdir().expect("create a temporary directory");
    let spec = spec(directory.path(), false);
    let session = stand_in_session(&spec);

    let names: Vec<String> = session
        .endpoints()
        .into_iter()
        .map(|endpoint| endpoint.name)
        .collect();
    assert_eq!(names, ["serial_agent", "qmp", "monitor", "gdb"]);
}

#[test]
fn session_omits_the_agent_endpoint_for_a_booter() {
    let directory = tempfile::tempdir().expect("create a temporary directory");
    let spec = spec(directory.path(), true);
    let session = stand_in_session(&spec);

    let names: Vec<String> = session
        .endpoints()
        .into_iter()
        .map(|endpoint| endpoint.name)
        .collect();
    assert_eq!(names, ["qmp", "monitor", "gdb"]);
}

#[test]
#[cfg(windows)]
fn windows_ready_reports_an_early_exit() {
    let directory = tempfile::tempdir().expect("create a temporary directory");
    let spec = spec(directory.path(), false);
    let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve the engine endpoints");
    let command = EngineCommand {
        program: PathBuf::from("cmd"),
        args: vec![
            "/C".to_owned(),
            "ping -n 2 127.0.0.1 > NUL & exit 1".to_owned(),
        ],
        env: Vec::new(),
        endpoints: endpoints.engine_endpoints(),
    };
    let mut session =
        QemuSession::spawn(&binary(), &spec, &system(), command).expect("start the stand-in child");

    let started = Instant::now();
    match session.ready(Duration::from_secs(3)) {
        Err(EngineError::Exited(1)) => {}
        other => panic!("expected an early exit with code 1, got {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "ready did not report the early exit promptly: {:?}",
        started.elapsed()
    );
}

#[test]
#[cfg(windows)]
fn debug_endpoint_reports_unsupported_for_a_missing_gdb_endpoint() {
    let directory = tempfile::tempdir().expect("create a temporary directory");
    let spec = spec(directory.path(), false);
    let mut session = stand_in_session(&spec);

    match session.debug_endpoint(DebugKind::Gdb) {
        Err(EngineError::Unsupported("debugger")) => {}
        other => panic!("expected an unsupported debugger, got {other:?}"),
    }
}
