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
use dosker_engine::DebugKind;
#[cfg(unix)]
use dosker_platform::control::bind;
#[cfg(unix)]
use dosker_platform::endpoint::control_endpoint;
#[cfg(unix)]
use dosker_runtime::protocol::{FrameRead, Reply, Response, read_frame, write_message};
#[cfg(unix)]
use dosker_runtime::{ContainerRecord, StateStore};

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
#[test]
fn debug_gdb_prints_a_target_command() {
    let root = CliRoot::new();
    let id = seed_running(&root, "000000000301", "debug-gdb");
    let endpoint = control_endpoint(root.path(), &id);
    let listener = bind(&endpoint).expect("bind the scripted endpoint");
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match listener.accept(Duration::from_millis(50)) {
                Ok(Some(connection)) => {
                    let mut reader = BufReader::new(connection);
                    if let Ok(FrameRead::Line(_)) = read_frame(&mut reader) {
                        let response = Response::ok(Reply::Debug {
                            kind: DebugKind::Gdb,
                            endpoint: String::from("/tmp/dosker-gdb.sock"),
                        });
                        let _ = write_message(reader.get_mut(), &response);
                    }
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });

    let output = root.run(&["debug", "--gdb", id.as_str()]);

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

    let output = root.run(&["debug", "--gdb", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("mock"),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("QEMU"),
        "the capability provider is missing: {}",
        stderr_text(&output)
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
