mod common;

use std::time::Duration;

use common::{CliRoot, stderr_text, stdout_text};

fn last_field(output: &str) -> String {
    output
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn stop_running_container_reports_stopped() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["stop", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(last_field(&stdout_text(&output)), "stopped");
}

#[test]
fn stop_is_idempotent() {
    let root = CliRoot::new();
    let id = root.start();
    let first = root.run(&["stop", id.as_str()]);
    assert_eq!(first.status.code(), Some(0));

    let output = root.run(&["stop", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(last_field(&stdout_text(&output)), "stopped");
}

#[test]
fn stop_dead_supervisor_recovers_to_exited() {
    let root = CliRoot::new();
    let id = root.start();
    let engine = root.engine_pid(&id);
    root.kill_supervisor(&id);
    std::thread::sleep(Duration::from_millis(100));

    let output = root.run(&["stop", id.as_str()]);

    root.kill(engine);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(last_field(&stdout_text(&output)), "exited");
}

#[test]
fn stop_unknown_container_exits_one() {
    let root = CliRoot::new();

    let output = root.run(&["stop", "missing-container"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
}
