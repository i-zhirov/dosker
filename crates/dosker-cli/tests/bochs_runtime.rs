#![cfg(unix)]

mod common;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use common::{
    CliRoot, import_image_for_bochs, import_image_with_config, make_disk_image, run_captured,
    run_with_bochs, stderr_text, stdout_text,
};
use dosker_core::{ContainerId, Status};
use dosker_test_support::fixtures::{bochs_image, integration_enabled};

const CONNECT_RETRY: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const DUMP_TIMEOUT: Duration = Duration::from_secs(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const BOCHS_VERSION: &str = "3.1";
const BOCHS_CAPABILITIES: &str = "boot,serial,stop,headless,debug_native";

// The integration tests boot real guests through Bochs, so only one test may
// run at a time even when the test harness is parallel.
static INTEGRATION_LOCK: Mutex<()> = Mutex::new(());

fn integration_lock() -> MutexGuard<'static, ()> {
    INTEGRATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Returns the prepared Bochs image when both gates are open.
///
/// A missing gate skips the test. A gate that names an unusable image is a
/// configuration error and fails.
fn prepared_image() -> Option<PathBuf> {
    if !integration_enabled() {
        return None;
    }

    match bochs_image() {
        Ok(path) => Some(path),
        Err(error) => {
            panic!(
                "the integration tests are enabled, but the prepared image is unavailable: {error}"
            )
        }
    }
}

fn start_detached(root: &CliRoot, reference: &str) -> ContainerId {
    let output = run_with_bochs(
        root,
        &[
            "run",
            "-d",
            "--engine",
            "bochs",
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
    let output = run_with_bochs(root, &["stop", id.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
}

fn connect_unix(path: &str) -> UnixStream {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return stream,
            Err(error) if Instant::now() >= deadline => {
                panic!("cannot connect to the debugger socket {path}: {error}");
            }
            Err(_) => std::thread::sleep(CONNECT_RETRY),
        }
    }
}

fn read_until(stream: &mut UnixStream, marker: &str) -> String {
    let deadline = Instant::now() + DUMP_TIMEOUT;
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
                if Instant::now() >= deadline {
                    break;
                }
            }
            Err(error) => panic!("cannot read from the debugger: {error}"),
        }

        if Instant::now() >= deadline {
            break;
        }
    }
    String::from_utf8_lossy(&collected).into_owned()
}

#[test]
fn bochs_boots_the_prepared_image_and_answers_exec() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import_image_for_bochs(&root, &image, "bochs:latest");
    let id = start_detached(&root, &reference);

    let hello = run_with_bochs(&root, &["exec", id.as_str(), "ECHO", "HELLO"]);
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
fn bochs_stops_with_the_recorded_exit_code() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import_image_for_bochs(&root, &image, "bochs:latest");
    let id = start_detached(&root, &reference);

    stop(&root, &id);

    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Stopped, "record: {record:?}");
    assert_eq!(record.exit_code, Some(143), "record: {record:?}");
}

#[test]
fn bochs_reports_its_capabilities() {
    if !integration_enabled() {
        return;
    }
    let _serial = integration_lock();
    let root = CliRoot::new();

    let mut command = root.base_command();
    command.env(
        "DOSKER_BOCHS",
        std::env::var("DOSKER_BOCHS").unwrap_or_else(|_| "bochs".to_owned()),
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
        .find(|line| line.split_whitespace().next() == Some("bochs"))
        .unwrap_or_else(|| panic!("engines did not report bochs: {listing}"));
    let (head, capabilities) = line
        .rsplit_once(' ')
        .unwrap_or_else(|| panic!("the Bochs line is malformed: {line:?}"));
    assert_eq!(
        capabilities, BOCHS_CAPABILITIES,
        "the reported Bochs capabilities differ: {line:?}"
    );
    let mut head_fields = head.splitn(3, ' ');
    assert_eq!(head_fields.next(), Some("bochs"), "line: {line:?}");
    assert_eq!(
        head_fields.next(),
        Some(BOCHS_VERSION),
        "the reported Bochs version differs: {line:?}"
    );
    assert!(
        head_fields.next().is_some_and(|path| !path.is_empty()),
        "the reported Bochs path is missing: {line:?}"
    );
}

#[test]
fn bochs_exposes_a_working_internal_debugger_endpoint() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    let reference = import_image_for_bochs(&root, &image, "bochs:latest");
    let id = start_detached(&root, &reference);

    let debug = run_with_bochs(&root, &["debug", id.as_str()]);
    assert_eq!(
        debug.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&debug),
        stderr_text(&debug)
    );
    let printed = stdout_text(&debug);
    let socket = printed
        .lines()
        .find_map(|line| line.trim().strip_prefix("native debugger "))
        .unwrap_or_else(|| panic!("debug did not print a native endpoint: {printed:?}"))
        .trim()
        .to_owned();

    let mut stream = connect_unix(&socket);
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .expect("set the debugger read timeout");
    stream.write_all(b"r\n").expect("send the register command");
    stream.flush().expect("flush the register command");
    let dump = read_until(&mut stream, "rax:");
    assert!(
        dump.contains("rax:"),
        "the debugger did not print a register dump: {dump:?}"
    );

    stream.write_all(b"c\n").expect("send the continue command");
    stream.flush().expect("flush the continue command");

    stop(&root, &id);
}

#[test]
fn bochs_rejects_a_required_capability_it_does_not_declare() {
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
        Some("bochs"),
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
