#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dosker_core::ContainerId;
use dosker_engine::{
    DebugKind, EngineCommand, EngineError, EngineSession, EngineSpec, ExecRequest,
};
use dosker_engine_qemu::paths::QemuPaths;
use dosker_engine_qemu::{QemuBinary, QemuSession, command};
use dosker_platform::process::{ProcessSupervisor, system};

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const STOP_GRACE: Duration = Duration::from_secs(10);

fn integration_image() -> Option<PathBuf> {
    if !matches!(std::env::var("DOSKER_INTEGRATION").as_deref(), Ok("1")) {
        return None;
    }

    let value = std::env::var_os("DOSKER_QEMU_IMAGE")?;
    if value.is_empty() {
        return None;
    }

    let path = PathBuf::from(value);
    path.is_file().then_some(path)
}

fn setup(image: &Path, booter: bool) -> (tempfile::TempDir, EngineSpec) {
    let dir = tempfile::Builder::new()
        .prefix("dq-session-")
        .tempdir_in("/tmp")
        .expect("create temporary directory");
    let root = dir.path().to_path_buf();
    let id = ContainerId::generate();
    let container = root.join("containers").join(id.as_str());
    std::fs::create_dir_all(&container).expect("create container directory");
    let disk = container.join("disk.img");
    std::fs::copy(image, &disk).expect("copy the integration image");

    let spec = EngineSpec {
        root: root.clone(),
        container_id: id,
        workdir: "C:\\".to_owned(),
        env: BTreeMap::new(),
        cmd: None,
        expects_agent: !booter,
        log_dir: container,
        disk_path: disk,
        booter,
        display: Default::default(),
        vnc_port: None,
    };

    (dir, spec)
}

fn spawn(spec: &EngineSpec) -> QemuSession {
    let binary = QemuBinary::discover().expect("discover QEMU");
    let command = command::build(spec, &binary).expect("build the QEMU command");
    QemuSession::spawn(&binary, spec, &system(), command).expect("start QEMU")
}

fn exec_request() -> ExecRequest {
    ExecRequest {
        command: "ECHO HELLO".to_owned(),
        env: BTreeMap::new(),
        workdir: "C:\\".to_owned(),
        timeout_ms: 60_000,
    }
}

#[test]
fn prepared_image_session_boots_execs_and_stops() {
    let Some(image) = integration_image() else {
        return;
    };
    let (_dir, spec) = setup(&image, false);
    let mut session = spawn(&spec);

    session
        .ready(READY_TIMEOUT)
        .expect("the image becomes ready");
    let result = session.exec(&exec_request()).expect("execute ECHO HELLO");

    assert_eq!(result.exit_code, 0, "output: {:?}", result.output);
    assert!(
        result.output.contains("HELLO"),
        "output: {:?}",
        result.output
    );

    let code = session.stop(STOP_GRACE).expect("stop the session");
    assert_eq!(code, 0);
}

#[test]
fn booter_session_is_ready_without_an_agent() {
    let Some(image) = integration_image() else {
        return;
    };
    let (_dir, spec) = setup(&image, true);
    let mut session = spawn(&spec);

    session
        .ready(Duration::from_secs(10))
        .expect("a booter becomes ready");

    match session.exec(&exec_request()) {
        Err(EngineError::NoAgent) => {}
        other => panic!("expected no agent, got {other:?}"),
    }
}

#[test]
fn booter_session_stops_cleanly() {
    let Some(image) = integration_image() else {
        return;
    };
    let (_dir, spec) = setup(&image, true);
    let mut session = spawn(&spec);

    session
        .ready(Duration::from_secs(10))
        .expect("a booter becomes ready");

    match session.stop(STOP_GRACE) {
        Ok(code) => assert_eq!(code, 0),
        Err(error) => panic!("a booter did not stop cleanly: {error}"),
    }
}

#[test]
fn ready_reports_an_early_exit() {
    let Some(image) = integration_image() else {
        return;
    };
    let (_dir, spec) = setup(&image, false);
    let binary = QemuBinary::discover().expect("discover QEMU");
    let command = EngineCommand {
        program: binary.path.clone(),
        args: vec!["--not-a-qemu-option".to_owned()],
        env: Vec::new(),
    };
    let mut session = QemuSession::spawn(&binary, &spec, &system(), command).expect("start QEMU");

    let started = Instant::now();
    match session.ready(READY_TIMEOUT) {
        Err(EngineError::Exited(_)) => {}
        other => panic!("expected an early exit, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn session_exposes_sockets_for_screenshot_and_debug() {
    let Some(image) = integration_image() else {
        return;
    };
    let (_dir, spec) = setup(&image, false);
    let mut session = spawn(&spec);
    session
        .ready(READY_TIMEOUT)
        .expect("the image becomes ready");

    let names: Vec<String> = session
        .endpoints()
        .into_iter()
        .map(|endpoint| endpoint.name)
        .collect();
    assert_eq!(names, ["serial_agent", "qmp", "monitor", "gdb"]);

    let paths = QemuPaths::for_spec(&spec);
    assert!(paths.gdb.exists(), "the gdb socket is missing");

    let endpoint = session
        .debug_endpoint(DebugKind::Gdb)
        .expect("the gdb endpoint is exposed");
    assert_eq!(PathBuf::from(endpoint), paths.gdb);
}

#[test]
fn session_retains_the_private_disk() {
    let Some(image) = integration_image() else {
        return;
    };
    let (_dir, spec) = setup(&image, false);
    let mut session = spawn(&spec);
    session
        .ready(READY_TIMEOUT)
        .expect("the image becomes ready");
    session.stop(STOP_GRACE).expect("stop the session");

    assert!(
        spec.disk_path.is_file(),
        "the private disk was removed: {}",
        spec.disk_path.display()
    );
}

#[test]
fn dropping_a_running_session_kills_qemu() {
    let Some(image) = integration_image() else {
        return;
    };
    let (_dir, spec) = setup(&image, false);
    let mut session = spawn(&spec);
    session
        .ready(READY_TIMEOUT)
        .expect("the image becomes ready");

    let pid = session.pid().expect("the session has a pid");
    drop(session);

    let processes = system();
    let deadline = Instant::now() + Duration::from_secs(2);
    while processes.process_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "QEMU with pid {pid} is still alive"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn screenshot_connects_qmp_lazily() {
    let Some(image) = integration_image() else {
        return;
    };
    let (dir, spec) = setup(&image, false);
    let mut session = spawn(&spec);
    session
        .ready(READY_TIMEOUT)
        .expect("the image becomes ready");

    let destination = dir.path().join("screenshot.ppm");
    std::fs::write(&destination, b"stale screenshot").expect("write a stale screenshot");
    session
        .screenshot(&destination)
        .expect("capture the screen");

    let bytes = std::fs::read(&destination).expect("the screenshot exists");
    assert!(!bytes.is_empty(), "the screenshot is empty");
    assert!(
        bytes.starts_with(b"P6")
            || bytes.starts_with(b"P5")
            || bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "the screenshot does not have a PPM or PNG header: {:?}",
        &bytes[..bytes.len().min(8)]
    );
    assert!(QemuPaths::for_spec(&spec).qmp.exists());
}
