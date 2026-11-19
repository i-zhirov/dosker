//! End-to-end checks for bind mounts recorded by `create`.

mod common;

use std::fs;
use std::path::Path;
use std::process::Output;

#[cfg(unix)]
use std::os::unix::fs::symlink;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use common::run_captured;
use common::{CliRoot, stderr_text, stdout_text};
use dosker_runtime::VolumeStore;

#[cfg(unix)]
const VERSION_LINE: &str = "QEMU emulator version 11.1.3";

fn create_id(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(output)
    );
    stdout_text(output).trim().to_owned()
}

fn inspect(root: &CliRoot, id: &str) -> serde_json::Value {
    let output = root.run(&["inspect", id]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    serde_json::from_str(&stdout_text(&output)).expect("parse the inspect output")
}

fn host_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("create a host directory")
}

fn host_with_file(name: &str) -> tempfile::TempDir {
    let dir = host_dir();
    fs::write(dir.path().join(name), b"HOST\r\n").expect("write the host file");
    dir
}

fn bind(drive: &str, dir: &Path) -> String {
    format!("{drive}{}", dir.display())
}

#[cfg(unix)]
fn build_image(root: &CliRoot, tag: &str, doskerfile: &str) {
    use std::os::unix::fs::PermissionsExt;

    let context = tempfile::tempdir().expect("create a build context");
    // `FROM freedos:<version>` resolves the built-in base and rejects an
    // imported `freedos:latest` tag, so build bases use their own tag.
    fs::write(context.path().join("Doskerfile"), doskerfile).expect("write the Doskerfile");
    let qemu_dir = tempfile::tempdir().expect("create a qemu directory");
    let qemu = qemu_dir.path().join("qemu-system-i386");
    fs::write(&qemu, format!("#!/bin/sh\necho '{VERSION_LINE}'\n")).expect("write the qemu script");
    let mut permissions = fs::metadata(&qemu)
        .expect("read the script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&qemu, permissions).expect("set the script permissions");

    let mut command = root.base_command();
    command.env("DOSKER_QEMU", &qemu);
    command.args(["build", "-t", tag]);
    command.arg(context.path());
    let output = run_captured(&mut command, Duration::from_secs(30));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_records_a_bind_mount() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_with_file("HOST.TXT");

    let value = bind("D:", dir.path());
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &value,
        "freedos:latest",
    ]);
    let id = create_id(&output);
    let value = inspect(&root, &id);

    let volumes = value["volumes"].as_array().expect("the volumes array");
    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0]["drive"], "D:");
    assert_eq!(volumes[0]["kind"], "bind");
    assert_eq!(volumes[0]["name"], serde_json::Value::Null);
    assert_eq!(
        volumes[0]["host_path"].as_str(),
        dir.path().to_str(),
        "volumes: {volumes:?}"
    );
    assert_eq!(volumes[0]["read_only"], false);
}

#[test]
fn create_records_a_read_only_bind() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_with_file("HOST.TXT");

    let value = format!("{}:ro", bind("D:", dir.path()));
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &value,
        "freedos:latest",
    ]);
    let id = create_id(&output);
    let value = inspect(&root, &id);

    let volumes = value["volumes"].as_array().expect("the volumes array");
    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0]["read_only"], true);
}

#[test]
fn create_records_the_host_path_as_absolute() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_with_file("HOST.TXT");

    let value = bind("D:", dir.path());
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &value,
        "freedos:latest",
    ]);
    let id = create_id(&output);
    let value = inspect(&root, &id);

    let volumes = value["volumes"].as_array().expect("the volumes array");
    let host_path = volumes[0]["host_path"].as_str().expect("a host path");
    assert!(
        Path::new(host_path).is_absolute(),
        "host_path is not absolute: {host_path}"
    );
}

#[test]
fn create_rejects_a_bind_on_the_mock_engine() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_with_file("HOST.TXT");

    let value = bind("D:", dir.path());
    let output = root.run(&[
        "create",
        "--engine",
        "mock",
        "--bind",
        &value,
        "freedos:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("host_dir_live"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_a_bind_on_the_boot_drive() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_with_file("HOST.TXT");

    let value = bind("C:", dir.path());
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &value,
        "freedos:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("D:, E:, and F:"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_a_bind_on_a_floppy_drive() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_with_file("HOST.TXT");

    for drive in ["A:", "B:"] {
        let value = bind(drive, dir.path());
        let output = root.run(&[
            "create",
            "--engine",
            "qemu",
            "--bind",
            &value,
            "freedos:latest",
        ]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "drive {drive}; stderr: {}",
            stderr_text(&output)
        );
        assert!(
            stderr_text(&output).contains("D:, E:, and F:"),
            "drive {drive}; stderr: {}",
            stderr_text(&output)
        );
    }
}

#[test]
fn create_rejects_a_bind_outside_the_supported_set() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_with_file("HOST.TXT");

    let value = bind("G:", dir.path());
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &value,
        "freedos:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("D:, E:, and F:"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_a_bind_that_collides_with_a_volume_drive() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    VolumeStore::new(root.path().to_path_buf())
        .create(
            "data",
            dosker_image::DEFAULT_VOLUME_SIZE,
            chrono::Utc::now(),
        )
        .expect("create the volume");
    let dir = host_with_file("HOST.TXT");

    let value = bind("D:", dir.path());
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "-v",
        "data:D:",
        "--bind",
        &value,
        "freedos:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("mounted more than once"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn create_rejects_a_bind_on_a_drive_declared_by_the_image() {
    let root = CliRoot::new();
    root.import_image_as("base:latest");
    build_image(&root, "demo:latest", "FROM base:latest\nVOLUME D:\n");
    let dir = host_with_file("HOST.TXT");

    let value = bind("D:", dir.path());
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &value,
        "demo:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("declared by the image"),
        "stderr: {}",
        stderr_text(&output)
    );
    let volumes = VolumeStore::new(root.path().to_path_buf())
        .list()
        .expect("list the volumes");
    assert!(
        !volumes
            .iter()
            .any(|volume| volume.meta.name.starts_with("anon-")),
        "volumes: {:?}",
        volumes
            .iter()
            .map(|volume| volume.meta.name.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn create_rejects_a_duplicate_bind_drive() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let first = host_with_file("HOST.TXT");
    let second = host_with_file("HOST.TXT");

    let first_value = bind("D:", first.path());
    let second_value = bind("D:", second.path());
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &first_value,
        "--bind",
        &second_value,
        "freedos:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("mounted more than once"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_an_invalid_8_3_name_and_names_the_path() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_with_file("TOOLONGNAME.TXT");

    let value = bind("D:", dir.path());
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &value,
        "freedos:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains(&dir.path().display().to_string()),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("TOOLONGNAME.TXT"), "stderr: {stderr}");
}

#[test]
fn create_rejects_a_missing_host_directory() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_dir();
    let missing = dir.path().join("missing");

    let value = bind("D:", &missing);
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &value,
        "freedos:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains(&missing.display().to_string()),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn create_rejects_a_symlinked_host_entry() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let dir = host_with_file("HOST.TXT");
    let link = dir.path().join("LINK.TXT");
    symlink(dir.path().join("HOST.TXT"), &link).expect("create the symlink");

    let value = bind("D:", dir.path());
    let output = root.run(&[
        "create",
        "--engine",
        "qemu",
        "--bind",
        &value,
        "freedos:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains(&link.display().to_string()),
        "stderr: {}",
        stderr_text(&output)
    );
}
