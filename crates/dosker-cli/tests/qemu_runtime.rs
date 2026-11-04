#![cfg(unix)]

mod common;

use std::fs;
use std::io::Read;
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use common::{CliRoot, run_captured, stderr_text, stdout_text};
use dosker_core::{ContainerId, Status};
use dosker_engine_qemu::QemuBinary;
use dosker_engine_qemu::command::free_vnc_port;
use dosker_test_support::fixtures::{integration_enabled, kill_process, qemu_binary, qemu_image};

const CONNECT_RETRY: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const PERSISTED_TEXT: &str = "ONE\r\nTWO\r\n";

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
            eprintln!(
                "the integration tests are enabled, but the prepared image is unavailable: {error}"
            );
            panic!("the prepared QEMU image is unavailable: {error}");
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

fn import_booter(root: &CliRoot, image: &Path, reference: &str) -> String {
    let output = root.run_with_qemu(&[
        "image",
        "import",
        "--booter",
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
        run_captured(&mut command, CLEANUP_TIMEOUT)
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
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => return stream,
            Err(error) if Instant::now() >= deadline => {
                panic!("cannot connect to 127.0.0.1:{port}: {error}");
            }
            Err(_) => std::thread::sleep(CONNECT_RETRY),
        }
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

#[test]
fn prepared_image_boots_execs_and_keeps_its_disk() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-test:latest");
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

    assert_running(&root, "boot-check");
}

#[test]
fn attach_provides_a_working_line_shell() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-test:latest");
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
fn restart_returns_a_stopped_container_to_running() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-test:latest");
    let container = run_detached(&root, &reference, "restart-check");

    let stopped = root.run_with_qemu(&["stop", container.id().as_str()]);
    assert_eq!(
        stopped.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&stopped)
    );

    let restarted = root.run_with_qemu(&["restart", container.id().as_str()]);
    assert_eq!(
        restarted.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&restarted),
        stderr_text(&restarted)
    );

    let hello = exec(&root, container.id(), &["ECHO", "HELLO"]);
    assert_exec_succeeds(&hello);
    assert!(
        stdout_text(&hello).contains("HELLO"),
        "stdout: {}",
        stdout_text(&hello)
    );
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

    let reference = import(&root, &image, "freedos-test:latest");
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

    let create = exec(
        &root,
        container.id(),
        &["COPY", "/B", r"C:\DOSKER\INPUT.TXT", r"C:\DOSKER\OUT.TXT"],
    );
    assert_exec_succeeds(&create);

    let destination = host.path().join("out.txt");
    let download = root.run_with_qemu(&[
        "cp",
        &format!("{id}:C:\\DOSKER\\OUT.TXT"),
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
fn booter_exec_returns_no_agent() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import_booter(&root, &image, "booter-check:latest");
    let container = run_detached(&root, &reference, "booter-check");

    let output = exec(&root, container.id(), &["ECHO", "HELLO"]);
    assert_eq!(
        output.status.code(),
        Some(125),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let combined = format!("{}{}", stdout_text(&output), stderr_text(&output));
    assert!(
        combined.to_ascii_lowercase().contains("no agent"),
        "output does not name the missing agent: {combined:?}"
    );
}

#[test]
fn booter_screenshot_writes_a_file() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let host = tempfile::tempdir().expect("create a host directory");
    let destination = host.path().join("booter.ppm");
    let reference = import_booter(&root, &image, "booter-screenshot:latest");
    let container = run_detached(&root, &reference, "booter-screen");

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
fn screenshot_writes_a_file() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let host = tempfile::tempdir().expect("create a host directory");
    let destination = host.path().join("screen.ppm");
    let reference = import(&root, &image, "freedos-test:latest");
    let container = run_detached(&root, &reference, "screen-check");

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
fn vnc_display_listens_on_the_requested_port() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-test:latest");
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
fn engines_and_doctor_report_qemu() {
    let Some(_image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let version = QemuBinary::from_path(PathBuf::from(qemu_binary()))
        .expect("probe QEMU")
        .version_string();

    let engines = root.run_with_qemu(&["engines"]);
    assert_eq!(
        engines.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&engines)
    );
    assert!(
        stdout_text(&engines).contains(&version),
        "engines did not name the version {version}: {}",
        stdout_text(&engines)
    );

    let doctor = root.run_with_qemu(&["doctor"]);
    assert_eq!(
        doctor.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&doctor)
    );
    assert!(
        stdout_text(&doctor).contains(&version),
        "doctor did not name the version {version}: {}",
        stdout_text(&doctor)
    );
}

#[test]
fn gdbstub_is_reachable() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-test:latest");
    let container = run_detached(&root, &reference, "gdb-check");

    let output = root.run_with_qemu(&["debug", "--gdb", container.id().as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let printed = stdout_text(&output);
    let path = printed
        .trim()
        .strip_prefix("target remote ")
        .unwrap_or_else(|| panic!("debug did not print a target command: {printed:?}"));

    UnixStream::connect(path).expect("connect to the gdbstub socket");
}

#[test]
fn logs_show_engine_and_serial_output() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import(&root, &image, "freedos-test:latest");
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
fn image_import_does_not_modify_the_source() {
    let Some(image) = image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let before = fs::read(&image).expect("read the source image");

    let output = root.run_with_qemu(&[
        "image",
        "import",
        image.to_str().expect("image path is utf-8"),
        "source-check:latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let after = fs::read(&image).expect("read the source image again");
    assert_eq!(before, after, "the import modified the source image");
}
