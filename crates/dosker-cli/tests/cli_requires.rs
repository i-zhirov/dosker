//! End-to-end checks for `REQUIRES` enforcement in `create` and `run`.

mod common;

use std::collections::BTreeMap;
use std::process::Output;
use std::time::Duration;

use common::{
    CliRoot, disable_image_network, import_image_with_config, make_disk_image, run_captured,
    stderr_text,
};

/// Runs a command without a mock engine.
///
/// `create` only records the engine and never builds an adapter, and `run -d`
/// fails before it starts a container, so no engine binary is required;
/// `root.base_command()` avoids the mock engine lookup.
fn run_command(root: &CliRoot, args: &[&str]) -> Output {
    let mut command = root.base_command();
    command.args(args);
    run_captured(&mut command, Duration::from_secs(45))
}

fn import_requiring(root: &CliRoot, reference: &str, requires: Vec<&str>) -> String {
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    import_image_with_config(
        root,
        &source,
        reference,
        Some("dosbox-x"),
        BTreeMap::new(),
        requires,
    )
}

#[test]
fn create_rejects_a_required_capability_the_engine_lacks() {
    let root = CliRoot::new();
    let reference = import_requiring(&root, "req:latest", vec!["gdbstub"]);

    let output = run_command(&root, &["create", &reference]);

    assert_eq!(
        output.status.code(),
        Some(2),
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

#[test]
fn create_accepts_a_required_capability_the_engine_declares() {
    let root = CliRoot::new();
    let reference = import_requiring(&root, "req:latest", vec!["headless"]);
    disable_image_network(&root, &reference);

    let output = run_command(&root, &["create", &reference]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn run_rejects_a_required_capability_before_the_disk_is_created() {
    let root = CliRoot::new();
    let reference = import_requiring(&root, "req:latest", vec!["gdbstub"]);

    let output = run_command(&root, &["run", "-d", &reference]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        root.store().list().expect("list the containers").is_empty(),
        "a rejected requirement records no container"
    );
}

#[test]
fn create_rejects_an_unknown_required_capability() {
    let root = CliRoot::new();
    let reference = import_requiring(&root, "req:latest", vec!["nope"]);

    let output = run_command(&root, &["create", &reference]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("unknown capability: nope"),
        "stderr: {}",
        stderr_text(&output)
    );
}
