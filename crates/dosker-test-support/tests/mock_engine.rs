//! Behaviour of the mock engine child process.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use dosker_test_support::fixtures::mock_engine_path;
use serde_json::Value;

fn binary() -> PathBuf {
    mock_engine_path().expect("the mock engine binary is available")
}

fn spawn(env: &[(&str, &str)]) -> Child {
    let mut command = Command::new(binary());
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    command.spawn().expect("spawn the mock engine")
}

fn read_line(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read a line");
    assert!(!line.is_empty(), "expected a line, got end of stream");
    serde_json::from_str(&line).expect("parse a JSON line")
}

fn send(stdin: &mut impl Write, value: &str) {
    writeln!(stdin, "{value}").expect("write a request");
    stdin.flush().expect("flush a request");
}

fn finish(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn mock_engine_reports_ready() {
    let mut child = spawn(&[]);
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));

    let ready = read_line(&mut stdout);
    assert_eq!(ready["event"], "ready");
    assert_eq!(
        ready["pid"].as_u64(),
        Some(u64::from(child.id())),
        "the ready line names the engine process"
    );

    finish(child);
}

#[test]
fn mock_engine_echoes_command() {
    let mut child = spawn(&[]);
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut stdin = child.stdin.take().expect("stdin is piped");
    read_line(&mut stdout);

    send(&mut stdin, r#"{"op":"exec","command":"ECHO HELLO"}"#);
    let response = read_line(&mut stdout);

    assert_eq!(response["exit_code"], 0);
    assert_eq!(response["output"], "HELLO\r\n");

    finish(child);
}

#[test]
fn mock_engine_returns_requested_exit_code() {
    let mut child = spawn(&[]);
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut stdin = child.stdin.take().expect("stdin is piped");
    read_line(&mut stdout);

    send(&mut stdin, r#"{"op":"exec","command":"EXIT 7"}"#);
    let response = read_line(&mut stdout);

    assert_eq!(response["exit_code"], 7);
    assert_eq!(response["output"], "");
    assert_eq!(child.wait().expect("wait for the engine").code(), Some(7));
}

#[test]
fn mock_engine_unknown_command_succeeds() {
    let mut child = spawn(&[]);
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut stdin = child.stdin.take().expect("stdin is piped");
    read_line(&mut stdout);

    send(&mut stdin, r#"{"op":"exec","command":"FOO BAR"}"#);
    let response = read_line(&mut stdout);

    assert_eq!(response["exit_code"], 0);
    assert_eq!(response["output"], "");

    finish(child);
}

#[test]
fn mock_engine_stops_on_stop() {
    let mut child = spawn(&[]);
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut stdin = child.stdin.take().expect("stdin is piped");
    read_line(&mut stdout);

    send(&mut stdin, r#"{"op":"stop"}"#);
    let response = read_line(&mut stdout);

    assert_eq!(response["event"], "stopped");
    assert_eq!(child.wait().expect("wait for the engine").code(), Some(0));
}

#[test]
fn mock_engine_survives_stdin_eof() {
    let mut child = spawn(&[]);
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    read_line(&mut stdout);

    drop(child.stdin.take());
    std::thread::sleep(Duration::from_millis(200));

    assert!(
        child.try_wait().expect("poll the engine").is_none(),
        "the engine must not exit when stdin closes"
    );

    finish(child);
}

#[test]
fn mock_engine_forced_start_failure_exits_three() {
    let mut child = spawn(&[("MOCK_FAIL_START", "1")]);
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("stdout is piped")
        .read_to_string(&mut stdout)
        .expect("read the engine output");

    let status = child.wait().expect("wait for the engine");

    assert_eq!(stdout, "", "a failed start writes no ready line");
    assert_eq!(status.code(), Some(3));
}
