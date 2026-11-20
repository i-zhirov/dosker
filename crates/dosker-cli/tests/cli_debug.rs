mod common;

use common::{CliRoot, stderr_text};

#[cfg(unix)]
use common::stdout_text;
#[cfg(unix)]
use dosker_core::Status;
#[cfg(unix)]
use std::io::BufReader;
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use dosker_core::{ContainerId, ContainerName};
#[cfg(unix)]
use dosker_engine::{Capability, DebugKind, declared_capabilities};
#[cfg(unix)]
use dosker_platform::control::{ControlListener, bind};
#[cfg(unix)]
use dosker_platform::endpoint::control_endpoint;
#[cfg(unix)]
use dosker_runtime::protocol::{
    ErrorCode, FrameRead, Reply, Request, Response, read_frame, write_message,
};
#[cfg(unix)]
use dosker_runtime::{ContainerRecord, StateStore};

#[cfg(unix)]
fn seed_running(
    root: &CliRoot,
    id: &str,
    name: &str,
    engine: &str,
) -> (ContainerId, Box<dyn ControlListener>) {
    let id = ContainerId::parse(id).expect("parse the container id");
    let name = ContainerName::parse(name).expect("parse the container name");
    let endpoint = control_endpoint(root.path(), &id);
    let listener = bind(&endpoint).expect("bind the scripted endpoint");
    let mut record = ContainerRecord::new(
        id.clone(),
        name,
        String::from("freedos:latest"),
        engine.to_owned(),
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
    (id, listener)
}

#[cfg(unix)]
fn serve_debug(
    listener: Box<dyn ControlListener>,
    kind: DebugKind,
    endpoint: &'static str,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match listener.accept(Duration::from_millis(50)) {
                Ok(Some(connection)) => {
                    let mut reader = BufReader::new(connection);
                    // A liveness probe connects and closes without a request;
                    // skip it and wait for the real one.
                    let Ok(FrameRead::Line(line)) = read_frame(&mut reader) else {
                        continue;
                    };
                    let request: Request =
                        serde_json::from_str(&line).expect("parse the control request");
                    assert_eq!(
                        request,
                        Request::Debug { kind },
                        "the CLI asked for the wrong debugger kind"
                    );
                    let response = Response::ok(Reply::Debug {
                        kind,
                        endpoint: String::from(endpoint),
                    });
                    let _ = write_message(reader.get_mut(), &response);
                    return;
                }
                Ok(None) => {}
                Err(_) => return,
            }
        }
    })
}

#[cfg(unix)]
fn serve_debug_error(
    listener: Box<dyn ControlListener>,
    code: ErrorCode,
    message: &'static str,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match listener.accept(Duration::from_millis(50)) {
                Ok(Some(connection)) => {
                    let mut reader = BufReader::new(connection);
                    // A liveness probe connects and closes without a request;
                    // skip it and wait for the real one.
                    if !matches!(read_frame(&mut reader), Ok(FrameRead::Line(_))) {
                        continue;
                    }
                    let response = Response::error(code, message);
                    let _ = write_message(reader.get_mut(), &response);
                    return;
                }
                Ok(None) => {}
                Err(_) => return,
            }
        }
    })
}

#[cfg(unix)]
#[test]
fn debug_gdb_prints_a_target_command() {
    let root = CliRoot::new();
    let (id, listener) = seed_running(&root, "000000000301", "debug-gdb", "qemu");
    let server = serve_debug(listener, DebugKind::Gdb, "/tmp/dosker-gdb.sock");

    let output = root.run(&["debug", "--gdb", id.as_str()]);
    server.join().expect("the scripted server");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("target remote /tmp/dosker-gdb.sock"),
        "stdout: {}",
        stdout_text(&output)
    );
}

#[test]
fn debug_reports_unsupported_for_the_mock_engine() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["debug", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(stderr.contains("debug_native"), "stderr: {stderr}");
    assert!(stderr.contains("qemu"), "stderr: {stderr}");
    assert!(stderr.contains("dosbox-x"), "stderr: {stderr}");
    assert!(stderr.contains("bochs"), "stderr: {stderr}");
}

#[cfg(unix)]
#[test]
fn debug_rejects_gdb_when_the_engine_has_no_gdbstub() {
    let root = CliRoot::new();
    let (id, _listener) = seed_running(&root, "000000000302", "debug-dosbox-x", "dosbox-x");

    let output = root.run(&["debug", "--gdb", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains(
            "engine dosbox-x does not provide the gdbstub capability; the qemu engine provides it"
        ),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn debug_rejects_gdb_when_the_bochs_engine_has_no_gdbstub() {
    let root = CliRoot::new();
    let (id, _listener) = seed_running(&root, "000000000305", "debug-bochs", "bochs");

    let output = root.run(&["debug", "--gdb", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains(
            "engine bochs does not provide the gdbstub capability; the qemu engine provides it"
        ),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn debug_native_prints_the_endpoint() {
    if !declared_capabilities("dosbox-x")
        .is_some_and(|capabilities| capabilities.contains(Capability::DebugNative))
    {
        return;
    }

    let root = CliRoot::new();
    let (id, listener) = seed_running(&root, "000000000303", "debug-native", "dosbox-x");
    let server = serve_debug(listener, DebugKind::Native, "/tmp/dosker-native.sock");

    let output = root.run(&["debug", id.as_str()]);
    server.join().expect("the scripted server");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("native debugger /tmp/dosker-native.sock"),
        "stdout: {}",
        stdout_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn debug_reports_the_supervisor_message_when_the_endpoint_is_unavailable() {
    const MESSAGE: &str = "the debugger endpoint is unavailable";

    let root = CliRoot::new();
    let (id, listener) = seed_running(&root, "000000000304", "debug-unavailable", "qemu");
    let server = serve_debug_error(listener, ErrorCode::Unsupported, MESSAGE);

    let output = root.run(&["debug", id.as_str()]);
    server.join().expect("the scripted server");

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(stderr.contains(MESSAGE), "stderr: {stderr}");
    assert!(
        !stderr.contains("does not provide"),
        "the capability message was reported instead of the supervisor message: {stderr}"
    );
}

#[test]
fn debug_rejects_a_stopped_container() {
    let root = CliRoot::new();
    let id = root.start();
    let stopped = root.run(&["stop", id.as_str()]);
    assert_eq!(stopped.status.code(), Some(0));

    let output = root.run(&["debug", "--gdb", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(125),
        "stderr: {}",
        stderr_text(&output)
    );
}
