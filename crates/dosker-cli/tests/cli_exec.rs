mod common;

use common::{CliRoot, stderr_text, stdout_text};

#[test]
fn exec_echo_returns_output_and_zero() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["exec", id.as_str(), "ECHO", "HELLO"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output).trim(), "HELLO");
}

#[test]
fn exec_passes_the_guest_exit_code_through() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["exec", id.as_str(), "EXIT", "7"]);

    assert_eq!(
        output.status.code(),
        Some(7),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn exec_timeout_exits_124() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["exec", "--timeout", "1", id.as_str(), "SLEEP", "5000"]);

    assert_eq!(
        output.status.code(),
        Some(124),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn exec_on_a_stopped_container_exits_125() {
    let root = CliRoot::new();
    let id = root.start();
    let stopped = root.run(&["stop", id.as_str()]);
    assert_eq!(stopped.status.code(), Some(0));

    let output = root.run(&["exec", id.as_str(), "ECHO", "HELLO"]);

    assert_eq!(
        output.status.code(),
        Some(125),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn exec_joins_multiple_words_with_spaces() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["exec", id.as_str(), "ECHO", "HELLO", "WORLD"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output).trim(), "HELLO WORLD");
}

#[test]
fn exec_invalid_env_exits_two() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["exec", "-e", "BAD", id.as_str(), "ECHO", "HELLO"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn exec_unknown_container_exits_one() {
    let root = CliRoot::new();

    let output = root.run(&["exec", "missing-container", "ECHO", "HELLO"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn exec_writes_guest_output_verbatim() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["exec", id.as_str(), "ECHO", "HELLO"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(output.stdout, b"HELLO\r\n");
}
