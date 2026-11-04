//! End-to-end checks for `dosker logs`.

mod common;

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use chrono::Utc;
use common::{CliRoot, stderr_text, stdout_text};
use dosker_core::{ContainerId, ContainerName};
use dosker_platform::endpoint::control_endpoint;
use dosker_runtime::ContainerRecord;

const FOLLOW_WAIT: Duration = Duration::from_secs(10);
const POLL_STEP: Duration = Duration::from_millis(20);

fn seed(root: &CliRoot, id: &str, name: &str) -> ContainerId {
    let id = ContainerId::parse(id).expect("parse the container id");
    let name = ContainerName::parse(name).expect("parse the container name");
    let endpoint = control_endpoint(root.path(), &id);
    let record = ContainerRecord::new(
        id.clone(),
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        Utc::now(),
    );
    root.store().create(&record).expect("create the record");
    id
}

fn write_log(path: PathBuf, contents: &[u8]) {
    fs::write(path, contents).expect("write the log");
}

fn wait_for_text(path: &Path, needle: &str, stderr: &Path) {
    let deadline = Instant::now() + FOLLOW_WAIT;
    loop {
        let text = fs::read_to_string(path).unwrap_or_default();
        if text.contains(needle) {
            return;
        }
        if Instant::now() >= deadline {
            let stderr = fs::read_to_string(stderr).unwrap_or_default();
            panic!("the captured stdout did not contain {needle:?}; stderr: {stderr}");
        }
        std::thread::sleep(POLL_STEP);
    }
}

fn append_line(path: &Path, line: &str) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open the log for append");
    writeln!(file, "{line}").expect("append to the log");
    file.flush().expect("flush the log");
}

#[test]
fn logs_prints_the_container_log() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000501", "logs-container");
    let contents = b"first line\nsecond line\n";
    write_log(root.store().paths().container_log(&id), contents);

    let output = root.run(&["logs", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(output.stdout, contents);
}

#[test]
fn logs_prints_binary_bytes_verbatim() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000506", "logs-binary");
    let contents = b"\x00\xff\xfe binary \x80\n\xc3\x28";
    write_log(root.store().paths().container_log(&id), contents);

    let output = root.run(&["logs", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(output.stdout, contents);
}

#[test]
fn logs_prints_the_engine_log() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000502", "logs-engine");
    let contents = b"engine qemu-system-i386 -machine pc\n";
    write_log(root.store().paths().engine_log(&id), contents);

    let output = root.run(&["logs", "--engine", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(output.stdout, contents);
}

#[test]
fn logs_prints_the_serial_log() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000503", "logs-serial");
    let contents = b"serial banner\n";
    write_log(root.store().paths().serial_log(&id), contents);

    let output = root.run(&["logs", "--serial", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(output.stdout, contents);
}

#[test]
fn logs_rejects_both_engine_and_serial() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000504", "logs-conflict");

    let output = root.run(&["logs", "--engine", "--serial", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
}

#[test]
fn logs_unknown_container_exits_one() {
    let root = CliRoot::new();

    let output = root.run(&["logs", "missing-container"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("container not found: missing-container"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn logs_follow_prints_appended_lines() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000505", "logs-follow");
    let log = root.store().paths().container_log(&id);
    write_log(log.clone(), b"first line\n");

    let (mut child, stdout_path, stderr_path, capture) =
        root.spawn_captured(root.command(), &["logs", "--follow", id.as_str()]);

    wait_for_text(&stdout_path, "first line", &stderr_path);
    append_line(&log, "appended line");
    wait_for_text(&stdout_path, "appended line", &stderr_path);

    let _ = child.kill();
    let _ = child.wait();

    let text = fs::read_to_string(&stdout_path).expect("read the captured stdout");
    assert!(text.contains("first line"), "stdout: {text}");
    assert!(text.contains("appended line"), "stdout: {text}");
    drop(capture);
}

// A process started with stdout already closed (`>&-`) is out of scope: the
// standard library reopens descriptors 0, 1, and 2 on `/dev/null` before
// `main`, so there is no closed descriptor left to poll. The tests below close
// the pipe after startup, which the write path and the hang-up probe detect.
#[test]
fn logs_follow_exits_when_stdout_closes() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000507", "logs-closed");
    let log = root.store().paths().container_log(&id);
    let initial = b"first line\n";
    write_log(log.clone(), initial);

    let mut command = root.command();
    command
        .args(["logs", "--follow", id.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("spawn logs --follow");
    let mut stdout = child.stdout.take().expect("stdout is piped");

    let mut received = vec![0u8; initial.len()];
    stdout
        .read_exact(&mut received)
        .expect("read the initial log bytes");
    assert_eq!(received, initial);
    drop(stdout);

    append_line(&log, "after the reader closed");

    let deadline = Instant::now() + FOLLOW_WAIT;
    loop {
        if let Some(status) = child.try_wait().expect("poll logs --follow") {
            assert_eq!(
                status.code(),
                Some(0),
                "logs --follow should exit successfully when stdout closes"
            );
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("logs --follow did not exit after stdout closed");
        }
        std::thread::sleep(POLL_STEP);
    }
}

#[cfg(unix)]
#[test]
fn logs_follow_exits_when_stdout_closes_without_new_output() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000508", "logs-closed-idle");
    let log = root.store().paths().container_log(&id);
    let initial = b"first line\n";
    write_log(log, initial);

    let mut command = root.command();
    command
        .args(["logs", "--follow", id.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("spawn logs --follow");
    let mut stdout = child.stdout.take().expect("stdout is piped");

    let mut received = vec![0u8; initial.len()];
    stdout
        .read_exact(&mut received)
        .expect("read the initial log bytes");
    assert_eq!(received, initial);
    drop(stdout);

    // No line is appended: the follow loop is idle when the reader closes.
    let deadline = Instant::now() + FOLLOW_WAIT;
    loop {
        if let Some(status) = child.try_wait().expect("poll logs --follow") {
            assert_eq!(
                status.code(),
                Some(0),
                "logs --follow should exit successfully when stdout closes while idle"
            );
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("logs --follow did not exit after stdout closed without new output");
        }
        std::thread::sleep(POLL_STEP);
    }
}
