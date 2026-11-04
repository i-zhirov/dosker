//! End-to-end checks for `dosker inspect`.

mod common;

use std::fs;

use chrono::Utc;
use common::{CliRoot, stderr_text, stdout_text};
use dosker_core::{ContainerId, ContainerName};
use dosker_engine::DisplayMode;
use dosker_platform::endpoint::control_endpoint;
use dosker_runtime::ContainerRecord;

const ENGINE_COMMAND: &str = "qemu-system-i386 -machine pc -cpu 486 -m 16";

fn seed(root: &CliRoot, id: &str, name: &str) -> ContainerId {
    let id = ContainerId::parse(id).expect("parse the container id");
    let name = ContainerName::parse(name).expect("parse the container name");
    let endpoint = control_endpoint(root.path(), &id);
    let disk = root
        .path()
        .join("containers")
        .join(id.as_str())
        .join("disk.img");
    let mut record = ContainerRecord::new(
        id.clone(),
        name,
        String::from("freedos:latest"),
        String::from("qemu"),
        endpoint.record(),
        Utc::now(),
    )
    .with_image(
        String::from("sha256:0123456789abcdef"),
        false,
        Some(String::from("APP.EXE")),
    )
    .with_display(DisplayMode::None, None)
    .with_disk(disk.clone());
    record.engine_command = Some(String::from(ENGINE_COMMAND));
    root.store().create(&record).expect("create the record");
    fs::write(&disk, b"a private disk").expect("write the private disk");
    id
}

#[test]
fn inspect_prints_the_record_json() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000401", "inspect-json");

    let output = root.run(&["inspect", id.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let record = root.store().load(&id).expect("load the record");
    let expected = format!(
        "{}\n",
        serde_json::to_string_pretty(&record).expect("serialize the record")
    );
    assert_eq!(stdout_text(&output), expected);
}

#[test]
fn inspect_includes_the_effective_engine_command() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000402", "inspect-command");

    let output = root.run(&["inspect", id.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let value: serde_json::Value =
        serde_json::from_str(&stdout_text(&output)).expect("parse the inspect output");
    assert_eq!(
        value["engine_command"].as_str(),
        Some(ENGINE_COMMAND),
        "stdout: {}",
        stdout_text(&output)
    );
}

#[test]
fn inspect_includes_the_private_disk_path() {
    let root = CliRoot::new();
    let id = seed(&root, "000000000403", "inspect-disk");

    let output = root.run(&["inspect", id.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let record = root.store().load(&id).expect("load the record");
    let disk = record.disk_path.expect("the record has a private disk");
    let value: serde_json::Value =
        serde_json::from_str(&stdout_text(&output)).expect("parse the inspect output");
    assert_eq!(
        value["disk_path"].as_str(),
        disk.to_str(),
        "stdout: {}",
        stdout_text(&output)
    );
}

#[test]
fn inspect_unknown_container_exits_one() {
    let root = CliRoot::new();

    let output = root.run(&["inspect", "missing-container"]);

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
