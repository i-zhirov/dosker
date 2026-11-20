#![cfg(unix)]

mod common;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use common::{
    CliRoot, import_image_for_dosbox_x, import_image_with_config, make_disk_image, run_captured,
    run_with_dosbox_x, stderr_text, stdout_text,
};
use dosker_core::ContainerId;
use dosker_test_support::fixtures::{dosbox_x_image, integration_enabled};

const CONNECT_RETRY: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const DOSBOX_X_VERSION: &str = "2026.10.01";
const DOSBOX_X_CAPABILITIES: &str = "boot,serial,stop,headless,debug_native";
const DEBUG_REQUEST: &[u8] = b"REQ 1 EXEC CPU\n";
const DEBUG_END: &str = "END 1";

// The integration tests boot real guests through DOSBox-X, so only one test
// may run at a time even when the test harness is parallel.
static INTEGRATION_LOCK: Mutex<()> = Mutex::new(());

fn integration_lock() -> MutexGuard<'static, ()> {
    INTEGRATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Returns the prepared DOSBox-X image when both gates are open.
///
/// A missing gate skips the test. A gate that names an unusable image is a
/// configuration error and fails.
fn prepared_image() -> Option<PathBuf> {
    if !integration_enabled() {
        return None;
    }

    match dosbox_x_image() {
        Ok(path) => Some(path),
        Err(error) => {
            panic!(
                "the integration tests are enabled, but the prepared image is unavailable: {error}"
            )
        }
    }
}

fn start_detached(root: &CliRoot, reference: &str) -> ContainerId {
    let output = run_with_dosbox_x(
        root,
        &[
            "run",
            "-d",
            "--engine",
            "dosbox-x",
            "--boot-timeout",
            "120",
            reference,
        ],
    );
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
    ContainerId::parse(stdout.trim()).expect("parse the container id")
}

fn stop(root: &CliRoot, id: &ContainerId) {
    let output = run_with_dosbox_x(root, &["stop", id.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
}

fn connect_tcp(address: &str) -> TcpStream {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match TcpStream::connect(address) {
            Ok(stream) => return stream,
            Err(error) if Instant::now() >= deadline => {
                panic!("cannot connect to {address}: {error}");
            }
            Err(_) => std::thread::sleep(CONNECT_RETRY),
        }
    }
}

fn read_until(stream: &mut TcpStream, marker: &str) -> String {
    let mut collected = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                collected.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&collected);
                if text.contains(marker) {
                    return text.into_owned();
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                panic!(
                    "the debugger did not answer before the read timeout: {:?}",
                    String::from_utf8_lossy(&collected)
                );
            }
            Err(error) => panic!("cannot read from the debugger: {error}"),
        }
    }
    String::from_utf8_lossy(&collected).into_owned()
}

#[test]
fn dosbox_x_boots_the_prepared_image_and_answers_exec() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import_image_for_dosbox_x(&root, &image, "dosbox:latest");
    let id = start_detached(&root, &reference);

    let hello = run_with_dosbox_x(&root, &["exec", id.as_str(), "ECHO", "HELLO"]);
    assert_eq!(
        hello.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&hello),
        stderr_text(&hello)
    );
    assert!(
        stdout_text(&hello).contains("HELLO"),
        "stdout: {}",
        stdout_text(&hello)
    );

    stop(&root, &id);
}

#[test]
fn dosbox_x_reports_its_capabilities() {
    if !integration_enabled() {
        return;
    }
    let _serial = integration_lock();
    let root = CliRoot::new();

    let mut command = root.base_command();
    command.env(
        "DOSKER_DOSBOX_X",
        std::env::var("DOSKER_DOSBOX_X").unwrap_or_else(|_| "dosbox-x".to_owned()),
    );
    command.arg("engines");
    let engines = run_captured(&mut command, COMMAND_TIMEOUT);
    assert_eq!(
        engines.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&engines)
    );

    let listing = stdout_text(&engines);
    let line = listing
        .lines()
        .find(|line| line.split_whitespace().next() == Some("dosbox-x"))
        .unwrap_or_else(|| panic!("engines did not report dosbox-x: {listing}"));
    let (head, capabilities) = line
        .rsplit_once(' ')
        .unwrap_or_else(|| panic!("the DOSBox-X line is malformed: {line:?}"));
    assert_eq!(
        capabilities, DOSBOX_X_CAPABILITIES,
        "the reported DOSBox-X capabilities differ: {line:?}"
    );
    let mut head_fields = head.splitn(3, ' ');
    assert_eq!(head_fields.next(), Some("dosbox-x"), "line: {line:?}");
    assert_eq!(
        head_fields.next(),
        Some(DOSBOX_X_VERSION),
        "the reported DOSBox-X version differs: {line:?}"
    );
    assert!(
        head_fields.next().is_some_and(|path| !path.is_empty()),
        "the reported DOSBox-X path is missing: {line:?}"
    );
}

#[test]
fn dosbox_x_rejects_a_required_capability_it_does_not_declare() {
    if !integration_enabled() {
        return;
    }
    let _serial = integration_lock();
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "req:latest",
        Some("dosbox-x"),
        BTreeMap::new(),
        vec!["gdbstub"],
    );

    let mut command = root.base_command();
    command.args(["create", &reference]);
    let output = run_captured(&mut command, COMMAND_TIMEOUT);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(stderr.contains("gdbstub"), "stderr: {stderr}");
    assert!(stderr.contains("qemu"), "stderr: {stderr}");
}

#[test]
fn dosbox_x_exposes_a_native_debugger_endpoint() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import_image_for_dosbox_x(&root, &image, "dosbox:latest");
    let id = start_detached(&root, &reference);

    let debug = run_with_dosbox_x(&root, &["debug", id.as_str()]);
    assert_eq!(
        debug.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&debug),
        stderr_text(&debug)
    );
    let printed = stdout_text(&debug);
    let address = printed
        .lines()
        .find_map(|line| line.trim().strip_prefix("native debugger "))
        .unwrap_or_else(|| panic!("debug did not print a native endpoint: {printed:?}"))
        .trim()
        .to_owned();

    let mut stream = connect_tcp(&address);
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .expect("set the debugger read timeout");
    stream
        .write_all(DEBUG_REQUEST)
        .expect("send the debugger command");
    let response = read_until(&mut stream, DEBUG_END);
    assert!(
        response.starts_with("BEGIN 1 OK"),
        "the debugger response did not start with an acknowledgement: {response:?}"
    );
    assert!(
        response.contains("cr0:") || response.contains("eflags:"),
        "the debugger response lacks a register dump: {response:?}"
    );

    stop(&root, &id);
}
