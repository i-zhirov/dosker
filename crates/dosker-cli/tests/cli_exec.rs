mod common;

use std::fs;

use common::{CliRoot, stderr_text, stdout_text};
use dosker_core::ContainerId;

#[cfg(unix)]
use dosker_core::Status;
#[cfg(unix)]
use std::io::BufReader;
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use dosker_core::ContainerName;
#[cfg(unix)]
use dosker_platform::control::bind;
#[cfg(unix)]
use dosker_platform::endpoint::control_endpoint;
#[cfg(unix)]
use dosker_runtime::protocol::{ErrorCode, FrameRead, Response, read_frame, write_message};
#[cfg(unix)]
use dosker_runtime::{ContainerRecord, StateStore};

fn import_booter(root: &CliRoot) -> String {
    let source = root.path().join("booter.img");
    fs::write(&source, b"not a filesystem").expect("write a booter blob");
    let output = root.run(&[
        "image",
        "import",
        "--booter",
        source.to_str().expect("source path is utf-8"),
        "booter-test:latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    String::from("booter-test:latest")
}

fn start_booter(root: &CliRoot) -> ContainerId {
    let image = import_booter(root);
    let output = root.run(&["run", "-d", image.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    ContainerId::parse(stdout_text(&output).trim()).expect("parse the container id")
}

#[cfg(unix)]
fn seed_running(root: &CliRoot, id: &str, name: &str) -> ContainerId {
    let id = ContainerId::parse(id).expect("parse the container id");
    let name = ContainerName::parse(name).expect("parse the container name");
    let endpoint = control_endpoint(root.path(), &id);
    let mut record = ContainerRecord::new(
        id.clone(),
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        chrono::Utc::now(),
    );
    record.status = Status::Running;
    record.pid = Some(
        Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("start a stand-in supervisor")
            .id(),
    );
    StateStore::new(root.path().to_path_buf())
        .create(&record)
        .expect("create the running record");
    id
}

#[cfg(unix)]
fn spawn_stub(endpoint: &dosker_platform::Endpoint, code: ErrorCode, message: &str) {
    let listener = bind(endpoint).expect("bind the scripted endpoint");
    let message = message.to_owned();
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match listener.accept(Duration::from_millis(50)) {
                Ok(Some(connection)) => {
                    let mut reader = BufReader::new(connection);
                    if let Ok(FrameRead::Line(_)) = read_frame(&mut reader) {
                        let _ = write_message(reader.get_mut(), &Response::error(code, &message));
                    }
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });
}

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

#[test]
fn exec_on_a_booter_exits_125_with_no_agent() {
    let root = CliRoot::new();
    let id = start_booter(&root);

    let output = root.run(&["exec", id.as_str(), "ECHO", "HELLO"]);

    assert_eq!(
        output.status.code(),
        Some(125),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("the image has no agent"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn exec_prints_guest_rebooted_and_exits_one() {
    let root = CliRoot::new();
    let id = seed_running(&root, "000000000401", "exec-rebooted");
    let endpoint = control_endpoint(root.path(), &id);
    spawn_stub(
        &endpoint,
        ErrorCode::GuestRebooted,
        "the guest rebooted during the command",
    );

    let output = root.run(&["exec", id.as_str(), "ECHO", "HELLO"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("the guest rebooted during the command"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn exec_prints_an_engine_error_and_exits_125() {
    let root = CliRoot::new();
    let id = seed_running(&root, "000000000403", "exec-engine-error");
    let endpoint = control_endpoint(root.path(), &id);
    spawn_stub(
        &endpoint,
        ErrorCode::EngineError,
        "the disk could not be read",
    );

    let output = root.run(&["exec", id.as_str(), "ECHO", "HELLO"]);

    assert_eq!(
        output.status.code(),
        Some(125),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("the disk could not be read"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn exec_prints_busy_and_exits_one() {
    let root = CliRoot::new();
    let id = seed_running(&root, "000000000402", "exec-busy");
    let endpoint = control_endpoint(root.path(), &id);
    spawn_stub(&endpoint, ErrorCode::Busy, "the guest is busy");

    let output = root.run(&["exec", id.as_str(), "ECHO", "HELLO"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("the guest is busy"),
        "stderr: {}",
        stderr_text(&output)
    );
}
