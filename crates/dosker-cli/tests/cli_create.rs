//! End-to-end checks for the engine recorded by `create`.

mod common;

use std::collections::BTreeMap;
use std::process::Output;
use std::time::Duration;

use chrono::Utc;
use common::{
    CliRoot, declare_image_volume, disable_image_network, import_image_with_config,
    make_disk_image, run_captured, stderr_text, stdout_text,
};
use dosker_core::ContainerId;
use dosker_image::DEFAULT_VOLUME_SIZE;
use dosker_runtime::VolumeStore;

fn container_id(output: &Output) -> ContainerId {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(output)
    );
    ContainerId::parse(stdout_text(output).trim()).expect("parse the printed id")
}

/// Runs a `create` invocation without a mock engine.
///
/// `create` only records the engine and never builds an adapter, so no engine
/// binary is required; `root.base_command()` avoids the mock engine lookup.
fn run_create(root: &CliRoot, args: &[&str]) -> Output {
    let mut command = root.base_command();
    command.args(args);
    run_captured(&mut command, Duration::from_secs(45))
}

#[test]
fn create_uses_the_image_engine_pin() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("dosbox-x"),
        BTreeMap::new(),
        Vec::new(),
    );
    disable_image_network(&root, &reference);

    let output = run_create(&root, &["create", &reference]);
    let id = container_id(&output);
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.engine, "dosbox-x");
}

#[test]
fn create_prefers_the_engine_flag_over_the_image_pin() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("dosbox-x"),
        BTreeMap::new(),
        Vec::new(),
    );

    let output = run_create(&root, &["create", &reference, "--engine", "qemu"]);
    let id = container_id(&output);
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.engine, "qemu");
}

#[cfg(feature = "qemu-engine")]
#[test]
fn create_falls_back_to_the_default_engine() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "unpinned:latest",
        None,
        BTreeMap::new(),
        Vec::new(),
    );

    let output = run_create(&root, &["create", &reference]);
    let id = container_id(&output);
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.engine, "qemu");
}

#[test]
fn create_copies_the_engine_config() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let engine_config: BTreeMap<String, Vec<String>> =
        [(String::from("qemu"), vec![String::from("-smp 2")])]
            .into_iter()
            .collect();
    let reference = import_image_with_config(
        &root,
        &source,
        "configured:latest",
        None,
        engine_config.clone(),
        Vec::new(),
    );

    let output = run_create(&root, &["create", &reference]);
    let id = container_id(&output);
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.engine_config, engine_config);
}

#[test]
fn create_records_the_bochs_engine() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("bochs"),
        BTreeMap::new(),
        Vec::new(),
    );
    disable_image_network(&root, &reference);

    let output = run_create(&root, &["create", &reference]);
    let id = container_id(&output);
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.engine, "bochs");
}

#[test]
fn create_rejects_an_unknown_engine_string() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "unpinned:latest",
        None,
        BTreeMap::new(),
        Vec::new(),
    );

    let output = run_create(&root, &["create", &reference, "--engine", "nope"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("unknown engine: nope"),
        "stderr: {}",
        stderr_text(&output)
    );

    let output = run_create(&root, &["create", &reference, "--engine", "NoPe"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("unknown engine: NoPe"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_ne2000_on_an_engine_without_nat() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("dosbox-x"),
        BTreeMap::new(),
        Vec::new(),
    );

    let output = run_create(&root, &["create", &reference]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(stderr.contains("nat"), "stderr: {stderr}");
    assert!(stderr.contains("qemu"), "stderr: {stderr}");
}

#[test]
fn create_rejects_a_ne2000_image_on_bochs() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "unpinned:latest",
        None,
        BTreeMap::new(),
        Vec::new(),
    );

    let output = run_create(&root, &["create", &reference, "--engine", "bochs"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains(
            "engine bochs does not provide the nat capability; the qemu engine provides it"
        ),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_vnc_on_an_engine_without_vnc() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("dosbox-x"),
        BTreeMap::new(),
        Vec::new(),
    );
    disable_image_network(&root, &reference);

    let output = run_create(&root, &["create", &reference, "--display", "vnc"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(stderr.contains("vnc"), "stderr: {stderr}");
    assert!(stderr.contains("qemu"), "stderr: {stderr}");
}

#[test]
fn create_rejects_vnc_on_bochs() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("bochs"),
        BTreeMap::new(),
        Vec::new(),
    );
    disable_image_network(&root, &reference);

    let output = run_create(&root, &["create", &reference, "--display", "vnc"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains(
            "engine bochs does not provide the vnc capability; the qemu engine provides it"
        ),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_named_volumes_on_dosbox_x() {
    let root = CliRoot::new();
    VolumeStore::new(root.path().to_path_buf())
        .create("data", DEFAULT_VOLUME_SIZE, Utc::now())
        .expect("create the volume");
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("dosbox-x"),
        BTreeMap::new(),
        Vec::new(),
    );
    disable_image_network(&root, &reference);

    let output = run_create(&root, &["create", "-v", "data:D:", &reference]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(stderr.contains("named volumes"), "stderr: {stderr}");
    assert!(stderr.contains("dosbox-x"), "stderr: {stderr}");
    assert!(stderr.contains("qemu"), "stderr: {stderr}");
}

#[test]
fn create_rejects_image_volume_declarations_on_dosbox_x() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("dosbox-x"),
        BTreeMap::new(),
        Vec::new(),
    );
    disable_image_network(&root, &reference);
    declare_image_volume(&root, &reference, "D:");

    let output = run_create(&root, &["create", &reference]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(stderr.contains("named volumes"), "stderr: {stderr}");
    assert!(stderr.contains("dosbox-x"), "stderr: {stderr}");
    assert!(stderr.contains("qemu"), "stderr: {stderr}");
}

#[test]
fn create_rejects_named_volumes_on_bochs() {
    let root = CliRoot::new();
    VolumeStore::new(root.path().to_path_buf())
        .create("data", DEFAULT_VOLUME_SIZE, Utc::now())
        .expect("create the volume");
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("bochs"),
        BTreeMap::new(),
        Vec::new(),
    );
    disable_image_network(&root, &reference);

    let output = run_create(&root, &["create", "-v", "data:D:", &reference]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output)
            .contains("named volumes are not supported by the bochs engine; use qemu"),
        "stderr: {}",
        stderr_text(&output)
    );

    declare_image_volume(&root, &reference, "D:");
    let output = run_create(&root, &["create", &reference]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output)
            .contains("named volumes are not supported by the bochs engine; use qemu"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_a_bind_on_bochs() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "pinned:latest",
        Some("bochs"),
        BTreeMap::new(),
        Vec::new(),
    );
    disable_image_network(&root, &reference);
    let host = tempfile::tempdir().expect("create a host directory");
    let value = format!("D:{}", host.path().display());

    let output = run_create(&root, &["create", "--bind", &value, &reference]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains(
            "engine bochs does not provide the host_dir_live capability; the qemu engine provides it"
        ),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_accepts_an_uppercase_engine_flag() {
    let root = CliRoot::new();
    let source = root.path().join("disk.img");
    make_disk_image(&source);
    let reference = import_image_with_config(
        &root,
        &source,
        "unpinned:latest",
        None,
        BTreeMap::new(),
        Vec::new(),
    );

    let output = run_create(&root, &["create", &reference, "--engine", "QEMU"]);
    let id = container_id(&output);
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.engine, "qemu");
}
