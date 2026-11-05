//! Windows smoke tests for the real QEMU runtime.
//!
//! The suite drives the `dosker` binary with the QEMU engine enabled. It runs
//! only on Windows and only when `DOSKER_INTEGRATION=1` selects a prepared
//! image. The tests use real QEMU guests and fixed container names, so one
//! mutex serializes them.

#![cfg(all(windows, feature = "qemu-engine"))]

mod common;

use std::fs;
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use common::{CliRoot, stderr_text, stdout_text};
use dosker_core::{ContainerId, Status};
use dosker_engine_qemu::QemuBinary;
use dosker_engine_qemu::capabilities;
use dosker_engine_qemu::command::free_vnc_port;
use dosker_platform::EndpointRecord;
use dosker_platform::process::{ProcessSupervisor, system};
use dosker_test_support::fixtures::{
    QEMU_IMAGE_ENV, integration_enabled, kill_process, qemu_image, wait_until,
};

const PERSISTED_TEXT: &str = "ONE\r\nTWO\r\n";
const CONNECT_RETRY: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SHORT_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const CRASH_TIMEOUT: Duration = Duration::from_secs(30);
const CRASH_POLL: Duration = Duration::from_millis(20);

// The integration tests start real QEMU guests and use fixed container names,
// so only one test may run at a time even when the test harness is parallel.
static INTEGRATION_LOCK: Mutex<()> = Mutex::new(());

fn image() -> Option<PathBuf> {
    if !integration_enabled() {
        return None;
    }

    match qemu_image() {
        Ok(path) => Some(path),
        Err(error) => {
            let requested = std::env::var_os(QEMU_IMAGE_ENV)
                .map(|value| PathBuf::from(value).display().to_string())
                .unwrap_or_else(|| String::from("<unset>"));
            eprintln!(
                "the integration tests are enabled, but the prepared image at {requested} is unavailable: {error}"
            );
            panic!("the prepared QEMU image at {requested} is unavailable: {error}");
        }
    }
}

fn integration_lock() -> MutexGuard<'static, ()> {
    INTEGRATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn import(root: &CliRoot, image: &Path, reference: &str) -> String {
    let output = root.run_with_qemu(&[
        "image",
        "import",
        image.to_str().expect("image path is utf-8"),
        reference,
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    reference.to_owned()
}

fn run_detached<'a>(root: &'a CliRoot, image: &str, name: &str) -> Container<'a> {
    let output = root.run_with_qemu(&["run", "-d", "--name", name, image]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let stdout = stdout_text(&output);
    assert_eq!(
        stdout.lines().count(),
        1,
        "run -d did not print exactly one line: {stdout:?}"
    );
    let id = ContainerId::parse(stdout.trim()).expect("parse the container id");
    Container::new(root, id)
}

fn exec(root: &CliRoot, id: &ContainerId, command: &[&str]) -> Output {
    let mut args = vec!["exec", id.as_str()];
    args.extend_from_slice(command);
    root.run_with_qemu(&args)
}

fn assert_exec_succeeds(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(output),
        stderr_text(output)
    );
}

/// A container that tries to stop and remove itself when it goes out of scope.
struct Container<'a> {
    root: &'a CliRoot,
    id: ContainerId,
}

impl<'a> Container<'a> {
    fn new(root: &'a CliRoot, id: ContainerId) -> Self {
        Self { root, id }
    }

    fn id(&self) -> &ContainerId {
        &self.id
    }
}

impl Drop for Container<'_> {
    fn drop(&mut self) {
        let record = self.root.store().load(&self.id).ok();
        let id = self.id.as_str();
        let stopped = cleanup(self.root, &["stop", id]);
        let removed = cleanup(self.root, &["rm", id]);

        if stopped && removed {
            return;
        }

        if let Some(record) = record {
            if let Some(pid) = record.engine_pid {
                kill_process(pid);
            }
            if let Some(pid) = record.pid {
                kill_process(pid);
            }
        }
    }
}

/// Runs one cleanup command with the harness timeout.
///
/// A timeout panic is caught because this runs from `Drop`, where a second
/// panic would abort the test process.
fn cleanup(root: &CliRoot, args: &[&str]) -> bool {
    let mut command = root.command_with_qemu();
    command.args(args);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        common::run_captured(&mut command, CLEANUP_TIMEOUT)
    }))
    .map(|output| output.status.success())
    .unwrap_or(false)
}

fn assert_running(root: &CliRoot, name: &str) {
    let output = root.run_with_qemu(&["ps", "--all"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let listing = stdout_text(&output);
    let running = listing.lines().any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields.get(1) == Some(&name) && fields.get(3) == Some(&Status::Running.as_str())
    });
    assert!(running, "ps did not show {name} as running:\n{listing}");
}

fn connect_tcp(port: u16) -> TcpStream {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let deadline = Instant::now() + CONNECT_TIMEOUT;

    loop {
        match TcpStream::connect_timeout(&address, SHORT_CONNECT_TIMEOUT) {
            Ok(stream) => return stream,
            Err(error) if Instant::now() >= deadline => {
                panic!("cannot connect to 127.0.0.1:{port}: {error}");
            }
            Err(_) => std::thread::sleep(CONNECT_RETRY),
        }
    }
}

fn assert_loopback(endpoint: Option<&EndpointRecord>, name: &str) -> u16 {
    match endpoint {
        Some(EndpointRecord::Tcp { host, port }) => {
            assert_eq!(
                *host,
                Ipv4Addr::LOCALHOST,
                "{name} does not bind to the loopback address"
            );
            *port
        }
        other => panic!("{name} is not a TCP endpoint: {other:?}"),
    }
}

fn assert_screenshot(path: &Path) {
    let bytes = fs::read(path).expect("read the screenshot");
    assert!(!bytes.is_empty(), "the screenshot is empty");
    assert!(
        bytes.starts_with(b"P6")
            || bytes.starts_with(b"P3")
            || bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "the screenshot is not a P6/P3 PPM or PNG file: {:?}",
        &bytes[..bytes.len().min(8)]
    );
}

fn row_for<'a>(rows: &'a [Vec<String>], id: &ContainerId) -> &'a [String] {
    rows.iter()
        .find(|row| row.first().map(String::as_str) == Some(id.as_str()))
        .unwrap_or_else(|| panic!("no row for {id}"))
}

#[test]
fn prepared_image_boots_execs_and_restarts() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "boot-check");

    let hello = exec(&root, container.id(), &["ECHO", "HELLO"]);
    assert_exec_succeeds(&hello);
    assert!(
        stdout_text(&hello).contains("HELLO"),
        "stdout: {}",
        stdout_text(&hello)
    );

    let write = exec(
        &root,
        container.id(),
        &["COPY", r"C:\DOSKER\TWO.TXT", r"C:\DOSKER\STATE.TXT"],
    );
    assert_exec_succeeds(&write);

    let stopped = root.run_with_qemu(&["stop", container.id().as_str()]);
    assert_eq!(
        stopped.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&stopped)
    );

    let started = root.run_with_qemu(&["start", container.id().as_str()]);
    assert_eq!(
        started.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&started),
        stderr_text(&started)
    );

    let persisted = exec(&root, container.id(), &["TYPE", r"C:\DOSKER\STATE.TXT"]);
    assert_exec_succeeds(&persisted);
    assert_eq!(
        stdout_text(&persisted),
        PERSISTED_TEXT,
        "the persisted text differs"
    );

    let again = exec(&root, container.id(), &["ECHO", "HELLO"]);
    assert_exec_succeeds(&again);
    assert!(
        stdout_text(&again).contains("HELLO"),
        "stdout: {}",
        stdout_text(&again)
    );
    assert_running(&root, "boot-check");
}

#[test]
fn control_transport_is_a_named_pipe() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "pipe-check");

    let record = root
        .store()
        .load(container.id())
        .expect("load the container record");
    match record
        .control_endpoint()
        .expect("the record has a control endpoint")
    {
        EndpointRecord::NamedPipe { name } => assert_eq!(
            name,
            &format!(r"\\.\pipe\dosker-{}-control", container.id()),
            "the control endpoint is not the container named pipe"
        ),
        other => panic!("the control endpoint is not a named pipe: {other:?}"),
    }

    let hello = exec(&root, container.id(), &["ECHO", "HELLO"]);
    assert_exec_succeeds(&hello);
    assert!(
        stdout_text(&hello).contains("HELLO"),
        "stdout: {}",
        stdout_text(&hello)
    );
}

#[test]
fn attach_provides_a_line_shell() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "attach-check");

    let output = root.spawn_with_stdin(
        root.command_with_qemu(),
        &["attach", container.id().as_str()],
        "ECHO HELLO\nexit\n",
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("HELLO"),
        "stdout: {}",
        stdout_text(&output)
    );
    assert_running(&root, "attach-check");
}

#[test]
fn engine_endpoints_are_loopback_tcp() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "endpoints-check");

    let record = root
        .store()
        .load(container.id())
        .expect("load the container record");
    let agent = assert_loopback(record.sockets.get("serial_agent"), "serial_agent");
    let qmp = assert_loopback(record.sockets.get("qmp"), "qmp");
    let monitor = assert_loopback(record.sockets.get("monitor"), "monitor");
    let gdb = assert_loopback(record.sockets.get("gdb"), "gdb");

    for port in [agent, qmp, monitor, gdb] {
        assert_ne!(port, 0, "an engine endpoint has port zero");
    }

    connect_tcp(qmp);
}

#[test]
fn monitor_accepts_a_loopback_connection() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "monitor-check");

    let record = root
        .store()
        .load(container.id())
        .expect("load the container record");
    let port = assert_loopback(record.sockets.get("monitor"), "monitor");

    connect_tcp(port);
}

#[test]
fn debug_prints_a_loopback_target() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "debug-check");

    let output = root.run_with_qemu(&["debug", "--gdb", container.id().as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let printed = stdout_text(&output);
    let target = printed
        .trim()
        .strip_prefix("target remote ")
        .unwrap_or_else(|| panic!("debug did not print a target command: {printed:?}"));
    let Some((host, port)) = target.rsplit_once(':') else {
        panic!("debug did not print a host and port: {target:?}");
    };
    assert_eq!(host, "127.0.0.1", "debug did not print a loopback target");
    let port: u16 = port.parse().expect("debug printed a numeric port");

    connect_tcp(port);
}

#[test]
fn supervisor_crash_leaves_no_engine() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "crash-check");

    let record = root
        .store()
        .load(container.id())
        .expect("load the container record");
    let supervisor_pid = record.pid.expect("the supervisor pid is recorded");
    let engine_pid = record.engine_pid.expect("the engine pid is recorded");

    assert!(kill_process(supervisor_pid), "kill the supervisor");
    assert!(
        wait_until(CRASH_TIMEOUT, CRASH_POLL, || !system()
            .process_alive(engine_pid)),
        "the engine pid {engine_pid} is still alive"
    );

    let ps = root.run_with_qemu(&["ps", "--all"]);
    assert_eq!(ps.status.code(), Some(0), "stderr: {}", stderr_text(&ps));
    let rows: Vec<Vec<String>> = stdout_text(&ps)
        .lines()
        .skip(1)
        .map(|line| line.split_whitespace().map(str::to_owned).collect())
        .collect();
    let row = row_for(&rows, container.id());
    assert_eq!(
        row.get(3).map(String::as_str),
        Some(Status::Exited.as_str())
    );
}

#[test]
fn vnc_listens_on_loopback() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let port = free_vnc_port().expect("allocate a free VNC port");
    let port_text = port.to_string();

    let output = root.run_with_qemu(&[
        "run",
        "-d",
        "--name",
        "vnc-check",
        "--display",
        "vnc",
        "--vnc-port",
        &port_text,
        &reference,
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let id = ContainerId::parse(stdout_text(&output).trim()).expect("parse the container id");
    let _container = Container::new(&root, id);

    let mut stream = connect_tcp(port);
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .expect("set the VNC read timeout");
    let mut greeting = [0u8; 4];
    stream
        .read_exact(&mut greeting)
        .expect("read the RFB greeting");
    assert_eq!(&greeting, b"RFB ");
}

#[test]
fn cp_round_trips_a_small_file() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let host = tempfile::tempdir().expect("create a host directory");
    let source = host.path().join("input.txt");
    let contents = b"ROUND TRIP\r\n";
    fs::write(&source, contents).expect("write the host source file");

    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "cp-check");
    let id = container.id().as_str();

    let upload = root.run_with_qemu(&[
        "cp",
        source.to_str().expect("source path is utf-8"),
        &format!("{id}:C:\\DOSKER\\INPUT.TXT"),
    ]);
    assert_eq!(
        upload.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&upload),
        stderr_text(&upload)
    );

    let typed = exec(&root, container.id(), &["TYPE", r"C:\DOSKER\INPUT.TXT"]);
    assert_exec_succeeds(&typed);
    assert!(
        stdout_text(&typed).contains("ROUND TRIP"),
        "stdout: {}",
        stdout_text(&typed)
    );

    let destination = host.path().join("out.txt");
    let download = root.run_with_qemu(&[
        "cp",
        &format!("{id}:C:\\DOSKER\\INPUT.TXT"),
        destination.to_str().expect("destination path is utf-8"),
    ]);
    assert_eq!(
        download.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&download),
        stderr_text(&download)
    );

    let downloaded = fs::read(&destination).expect("read the downloaded file");
    assert_eq!(downloaded, contents);
}

#[test]
fn screenshot_writes_a_file() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let host = tempfile::tempdir().expect("create a host directory");
    let destination = host.path().join("screen.ppm");
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "screenshot-check");

    let output = root.run_with_qemu(&[
        "screenshot",
        container.id().as_str(),
        "-o",
        destination.to_str().expect("screenshot path is utf-8"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert_screenshot(&destination);
}

#[test]
fn engines_and_doctor_report_qemu() {
    if image().is_none() {
        return;
    }
    let _serial = integration_lock();
    let root = CliRoot::new();
    let binary = QemuBinary::discover().expect("discover the QEMU binary");
    let version = binary.version_string();
    let path = binary.path.display().to_string();
    let capability_names = capabilities().names().collect::<Vec<_>>().join(",");

    let engines = root.run_with_qemu(&["engines"]);
    assert_eq!(
        engines.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&engines)
    );
    let engines_text = stdout_text(&engines);
    assert!(
        engines_text.contains(&version),
        "engines did not name the version {version}: {engines_text}"
    );
    assert!(
        engines_text.contains(&path),
        "engines did not name the executable path {path}: {engines_text}"
    );
    assert!(
        engines_text.contains(&capability_names),
        "engines did not name the capabilities {capability_names}: {engines_text}"
    );

    let doctor = root.run_with_qemu(&["doctor"]);
    assert_eq!(
        doctor.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&doctor)
    );
    let doctor_text = stdout_text(&doctor);
    assert!(
        doctor_text.contains(&version),
        "doctor did not name the version {version}: {doctor_text}"
    );
    assert!(
        doctor_text.contains(&path),
        "doctor did not name the executable path {path}: {doctor_text}"
    );
    assert!(
        doctor_text.contains(&capability_names),
        "doctor did not name the capabilities {capability_names}: {doctor_text}"
    );
}

#[test]
fn logs_show_engine_and_serial_output() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "logs-check");
    let id = container.id().as_str();

    let hello = exec(&root, container.id(), &["ECHO", "HELLO"]);
    assert_exec_succeeds(&hello);

    let engine = root.run_with_qemu(&["logs", "--engine", id]);
    assert_eq!(
        engine.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&engine)
    );
    let engine_log = stdout_text(&engine);
    let record = root
        .store()
        .load(container.id())
        .expect("load the container record");
    let rendered = record
        .engine_command
        .expect("the record has the rendered engine command");
    assert!(
        engine_log
            .lines()
            .any(|line| line == format!("engine {rendered}")),
        "the engine log lacks the rendered command {rendered:?}: {engine_log:?}"
    );

    let serial = root.run_with_qemu(&["logs", "--serial", id]);
    assert_eq!(
        serial.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&serial)
    );
    let serial_log = String::from_utf8_lossy(&serial.stdout);
    assert!(
        !serial_log.is_empty(),
        "the serial log is empty after a boot and an exec"
    );
    assert!(
        serial_log.contains("HELLO"),
        "the serial log lacks the executed command: {serial_log:?}"
    );
}

#[test]
fn state_and_logs_live_under_the_data_root() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-windows:latest");
    let container = run_detached(&root, &reference, "state-check");

    let container_dir = root.path().join("containers").join(container.id().as_str());
    for name in ["state.json", "engine.log", "serial.log", "container.log"] {
        let path = container_dir.join(name);
        assert!(path.is_file(), "missing {}", path.display());
    }
}
