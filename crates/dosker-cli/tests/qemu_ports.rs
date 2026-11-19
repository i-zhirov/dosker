#![cfg(unix)]

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use common::{CliRoot, run_captured, stderr_text, stdout_text};
use dosker_core::ContainerId;
use dosker_engine_qemu::command::free_tcp_port;
use dosker_test_support::fixtures::{integration_enabled, kill_process, net_image};

const CONNECT_RETRY: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST: &[u8] = b"GET / HTTP/1.0\r\n\r\n";

// A real guest boots here and the tests use fixed container names, so only one
// test may run at a time even when the test harness is parallel.
static INTEGRATION_LOCK: Mutex<()> = Mutex::new(());

fn prepared_net_image() -> Option<PathBuf> {
    if !integration_enabled() {
        return None;
    }

    match net_image() {
        Ok(path) => Some(path),
        Err(error) => {
            eprintln!(
                "the integration tests are enabled, but the network image is unavailable: {error}"
            );
            panic!("the network QEMU image is unavailable: {error}");
        }
    }
}

fn integration_lock() -> MutexGuard<'static, ()> {
    INTEGRATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn import_booter(root: &CliRoot, image: &Path, reference: &str) {
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
}

fn run_publishing<'a>(root: &'a CliRoot, name: &str, mapping: &str, image: &str) -> Container<'a> {
    let output = root.run_with_qemu(&["run", "-d", "--name", name, "-p", mapping, image]);
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

/// A container that tries to stop and remove itself when it goes out of scope.
struct Container<'a> {
    root: &'a CliRoot,
    id: ContainerId,
}

impl<'a> Container<'a> {
    fn new(root: &'a CliRoot, id: ContainerId) -> Self {
        Self { root, id }
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

#[test]
fn published_port_reaches_a_guest_tcp_service() {
    let Some(image) = prepared_net_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    import_booter(&root, &image, "net-booter:latest");
    let port = free_tcp_port().expect("allocate a free host port");
    let mapping = format!("{port}:80");

    let _container = run_publishing(&root, "pub", &mapping, "net-booter:latest");

    let mut stream = connect_tcp(port);
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .expect("set the response read timeout");
    stream.write_all(REQUEST).expect("send the HTTP request");
    let mut response = [0u8; 256];
    let read = stream.read(&mut response).expect("read the HTTP response");
    assert!(
        read > 0,
        "the guest service returned no response on the published port"
    );

    let printed = root.run_with_qemu(&["port", "pub"]);
    assert_eq!(
        printed.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&printed)
    );
    let expected = format!("127.0.0.1:{port}->80/tcp");
    assert_eq!(
        stdout_text(&printed).lines().collect::<Vec<_>>(),
        vec![expected.as_str()],
        "port did not print the published mapping"
    );

    let stopped = root.run_with_qemu(&["stop", "pub"]);
    assert_eq!(
        stopped.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&stopped),
        stderr_text(&stopped)
    );
}

#[test]
fn a_duplicate_host_port_leaves_the_first_running() {
    let Some(image) = prepared_net_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    import_booter(&root, &image, "net-booter:latest");
    let port = free_tcp_port().expect("allocate a free host port");
    let mapping = format!("{port}:80");

    let _container = run_publishing(&root, "pub", &mapping, "net-booter:latest");

    let second = root.run_with_qemu(&[
        "run",
        "-d",
        "--name",
        "pub2",
        "-p",
        &mapping,
        "net-booter:latest",
    ]);
    assert_eq!(
        second.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&second),
        stderr_text(&second)
    );
    let combined = format!("{}{}", stdout_text(&second), stderr_text(&second));
    assert!(
        combined.contains("already published"),
        "the second start does not name the conflict: {combined:?}"
    );

    let inspected = root.run_with_qemu(&["inspect", "pub"]);
    assert_eq!(
        inspected.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&inspected)
    );
    let value: serde_json::Value =
        serde_json::from_str(&stdout_text(&inspected)).expect("parse the inspect output");
    assert_eq!(
        value["status"], "running",
        "the first container is no longer running: {value}"
    );

    let removed_second = root.run_with_qemu(&["rm", "pub2"]);
    assert_eq!(
        removed_second.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&removed_second),
        stderr_text(&removed_second)
    );

    let stopped = root.run_with_qemu(&["stop", "pub"]);
    assert_eq!(
        stopped.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&stopped),
        stderr_text(&stopped)
    );
}
