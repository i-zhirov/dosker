//! End-to-end checks for volume mounts, published ports, and volume management.

mod common;

#[cfg(unix)]
use std::fs;
use std::process::Output;
#[cfg(unix)]
use std::time::Duration;

use chrono::Utc;
#[cfg(unix)]
use common::run_captured;
use common::{CliRoot, stderr_text, stdout_text};
#[cfg(unix)]
use dosker_core::{ContainerId, ContainerName};
use dosker_image::{DEFAULT_VOLUME_SIZE, FLOPPY_1440_SIZE};
#[cfg(unix)]
use dosker_platform::endpoint::control_endpoint;
#[cfg(unix)]
use dosker_runtime::ContainerRecord;
use dosker_runtime::{VolumeHolder, VolumeStore};

#[cfg(unix)]
const VERSION_LINE: &str = "QEMU emulator version 11.1.3";

fn seed_volume(root: &CliRoot, name: &str, size: u64) {
    VolumeStore::new(root.path().to_path_buf())
        .create(name, size, Utc::now())
        .expect("create the volume");
}

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

#[cfg(unix)]
fn seed_container(root: &CliRoot, name: &str) {
    let id = ContainerId::generate();
    let name = ContainerName::parse(name).expect("parse the container name");
    let endpoint = control_endpoint(root.path(), &id);
    let record = ContainerRecord::new(
        id,
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        Utc::now(),
    );
    root.store()
        .create(&record)
        .expect("create the seed record");
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
fn create_mounts_a_named_volume_at_the_requested_drive() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    seed_volume(&root, "data", DEFAULT_VOLUME_SIZE);

    let output = root.run(&["create", "-v", "data:D:", "freedos:latest"]);
    let id = create_id(&output);
    let value = inspect(&root, &id);

    let volumes = value["volumes"].as_array().expect("the volumes array");
    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0]["drive"], "D:");
    assert_eq!(volumes[0]["kind"], "named");
    assert_eq!(volumes[0]["name"], "data");
    assert_eq!(volumes[0]["read_only"], false);
}

#[test]
fn create_accepts_the_canonical_volume_form() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    seed_volume(&root, "data", DEFAULT_VOLUME_SIZE);

    for form in ["data:D:", "data:D"] {
        let output = root.run(&["create", "-v", form, "freedos:latest"]);
        let id = create_id(&output);
        let value = inspect(&root, &id);
        let volumes = value["volumes"].as_array().expect("the volumes array");
        assert_eq!(volumes.len(), 1, "form {form}");
        assert_eq!(volumes[0]["drive"], "D:", "form {form}");
        assert_eq!(volumes[0]["name"], "data", "form {form}");
    }
}

#[test]
fn create_mounts_a_superfloppy_volume_at_a_floppy_drive() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    seed_volume(&root, "flop", FLOPPY_1440_SIZE);

    let output = root.run(&["create", "-v", "flop:A:", "freedos:latest"]);
    let id = create_id(&output);
    let value = inspect(&root, &id);

    let volumes = value["volumes"].as_array().expect("the volumes array");
    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0]["drive"], "A:");
    assert_eq!(volumes[0]["name"], "flop");
}

#[test]
fn create_mounts_partitioned_volumes_on_the_ide_drives() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    for name in ["dvol", "evol", "fvol"] {
        seed_volume(&root, name, DEFAULT_VOLUME_SIZE);
    }

    let output = root.run(&[
        "create",
        "-v",
        "dvol:D:",
        "-v",
        "evol:E:",
        "-v",
        "fvol:F:",
        "freedos:latest",
    ]);
    let id = create_id(&output);
    let value = inspect(&root, &id);

    let drives: Vec<&str> = value["volumes"]
        .as_array()
        .expect("the volumes array")
        .iter()
        .map(|volume| volume["drive"].as_str().expect("a drive string"))
        .collect();
    assert_eq!(drives, ["D:", "E:", "F:"]);
}

#[test]
fn create_rejects_an_unknown_volume() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");

    let output = root.run(&["create", "-v", "missing:D:", "freedos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("volume not found"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_a_path_like_volume_name() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");

    let output = root.run(&["create", "-v", "../x:D:", "freedos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(!root.path().join("volumes").join("../x").exists());
}

#[test]
fn create_rejects_a_partitioned_volume_at_a_floppy_drive() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    seed_volume(&root, "data", DEFAULT_VOLUME_SIZE);

    let output = root.run(&["create", "-v", "data:A:", "freedos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_a_superfloppy_volume_at_an_ide_drive() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    seed_volume(&root, "flop", FLOPPY_1440_SIZE);

    let output = root.run(&["create", "-v", "flop:D:", "freedos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_a_duplicate_drive() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    seed_volume(&root, "data", DEFAULT_VOLUME_SIZE);

    let output = root.run(&["create", "-v", "data:D:", "-v", "data:D:", "freedos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_a_duplicate_volume() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    seed_volume(&root, "data", DEFAULT_VOLUME_SIZE);

    let output = root.run(&["create", "-v", "data:D:", "-v", "data:E:", "freedos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn create_creates_an_anonymous_volume_for_an_image_declaration() {
    let root = CliRoot::new();
    root.import_image_as("base:latest");
    build_image(
        &root,
        "demo:latest",
        "FROM base:latest\nVOLUME D:\nVOLUME E:\n",
    );

    let output = root.run(&["create", "demo:latest"]);
    let id = create_id(&output);
    let value = inspect(&root, &id);

    let names: Vec<&str> = value["volumes"]
        .as_array()
        .expect("the volumes array")
        .iter()
        .map(|volume| volume["name"].as_str().expect("a volume name"))
        .collect();
    let d = format!("anon-{id}-d");
    let e = format!("anon-{id}-e");
    assert_eq!(names.len(), 2, "volumes: {names:?}");
    assert!(names.contains(&d.as_str()), "volumes: {names:?}");
    assert!(names.contains(&e.as_str()), "volumes: {names:?}");
    assert!(
        root.path()
            .join("volumes")
            .join(&d)
            .join("meta.json")
            .is_file()
    );
    assert!(
        root.path()
            .join("volumes")
            .join(&e)
            .join("meta.json")
            .is_file()
    );
}

#[cfg(unix)]
#[test]
fn create_removes_anonymous_volumes_when_the_container_is_rejected() {
    let root = CliRoot::new();
    root.import_image_as("base:latest");
    seed_container(&root, "taken");
    build_image(&root, "demo:latest", "FROM base:latest\nVOLUME D:\n");

    let output = root.run(&["create", "--name", "taken", "demo"]);

    assert_eq!(
        output.status.code(),
        Some(1),
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
fn create_records_the_published_ports() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let port = dosker_engine_qemu::command::free_tcp_port().expect("allocate a free port");

    let mapping = format!("{port}:80");
    let output = root.run(&["create", "-p", &mapping, "freedos:latest"]);
    let id = create_id(&output);
    let value = inspect(&root, &id);

    let ports = value["ports"].as_array().expect("the ports array");
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0]["host_address"], "127.0.0.1");
    assert_eq!(ports[0]["host_port"].as_u64(), Some(u64::from(port)));
    assert_eq!(ports[0]["guest_port"].as_u64(), Some(80));
}

#[test]
fn create_rejects_a_duplicate_host_port() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let port = dosker_engine_qemu::command::free_tcp_port().expect("allocate a free port");
    let mapping = format!("{port}:80");

    let output = root.run(&["create", "-p", &mapping, "-p", &mapping, "freedos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn volume_warns_when_a_mounted_volume_is_dirty() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    seed_volume(&root, "data", DEFAULT_VOLUME_SIZE);
    let output = root.run(&["create", "-v", "data:D:", "freedos:latest"]);
    let id = create_id(&output);

    VolumeStore::new(root.path().to_path_buf())
        .set_dirty("data", true)
        .expect("mark the volume dirty");

    let output = root.run(&["start", &id]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("warning: volume data was not shut down cleanly"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn volume_create_list_and_remove_round_trip() {
    let root = CliRoot::new();

    let output = root.run(&["volume", "create", "data"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output).trim(), "data");

    let output = root.run(&["volume", "ls"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "NAME SIZE LAYOUT ATTACHED");
    assert_eq!(lines[1], "data 67108864 mbr_partitioned -");

    let output = root.run(&["volume", "rm", "data"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output).trim(), "data");
    assert!(!root.path().join("volumes").join("data").exists());
}

#[test]
fn volume_create_accepts_a_floppy_size() {
    let root = CliRoot::new();

    let output = root.run(&["volume", "create", "--size", "1440K", "flop"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let output = root.run(&["volume", "ls"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[1], "flop 1474560 superfloppy -");
}

#[test]
fn volume_create_rejects_an_unknown_size_suffix() {
    let root = CliRoot::new();

    let output = root.run(&["volume", "create", "--size", "64X", "data"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn volume_create_rejects_a_size_outside_the_volume_range() {
    let root = CliRoot::new();

    let output = root.run(&["volume", "create", "--size", "1M", "data"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn volume_create_rejects_a_path_like_name() {
    let root = CliRoot::new();

    let output = root.run(&["volume", "create", "../x"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(!root.path().join("volumes").join("../x").exists());
}

#[test]
fn volume_create_rejects_a_duplicate_name() {
    let root = CliRoot::new();
    let output = root.run(&["volume", "create", "data"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let image = root.path().join("volumes").join("data").join("volume.img");
    let before = std::fs::read(&image).expect("read the volume image");

    let output = root.run(&["volume", "create", "data"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("already exists"),
        "stderr: {}",
        stderr_text(&output)
    );
    let after = std::fs::read(&image).expect("read the volume image");
    assert_eq!(before, after);
}

#[test]
fn volume_rm_rejects_a_name_outside_the_rule() {
    let root = CliRoot::new();

    let output = root.run(&["volume", "rm", "../x"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn volume_rm_rejects_an_attached_volume() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let output = root.run(&["volume", "create", "data"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let output = root.run(&[
        "run",
        "-d",
        "--name",
        "holder",
        "-v",
        "data:D:",
        "freedos:latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let output = root.run(&["volume", "rm", "data"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("holder"),
        "stderr: {}",
        stderr_text(&output)
    );

    let output = root.run(&["stop", "holder"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let output = root.run(&["volume", "rm", "data"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn volume_rm_waits_for_a_stopping_container() {
    let root = CliRoot::new();
    let store = VolumeStore::new(root.path().to_path_buf());
    store
        .create("data", DEFAULT_VOLUME_SIZE, Utc::now())
        .expect("create the volume");
    let holder = VolumeHolder {
        container_id: dosker_core::ContainerId::generate(),
        container_name: String::from("holder"),
        supervisor_pid: std::process::id(),
    };
    let lock = store.acquire("data", holder).expect("acquire the lock");

    let mut command = root.command();
    command.args(["volume", "rm", "data"]);
    let worker = std::thread::spawn(move || {
        common::run_captured(&mut command, std::time::Duration::from_secs(15))
    });

    std::thread::sleep(std::time::Duration::from_millis(200));
    drop(lock);

    let output = worker.join().expect("join the rm command");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(!root.path().join("volumes").join("data").exists());
}

#[test]
fn volume_ls_reports_the_holder_container() {
    let root = CliRoot::new();
    root.import_image_as("freedos:latest");
    let output = root.run(&["volume", "create", "data"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let output = root.run(&[
        "run",
        "-d",
        "--name",
        "holder",
        "-v",
        "data:D:",
        "freedos:latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let output = root.run(&["volume", "ls"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(
        text.lines()
            .any(|line| line == "data 67108864 mbr_partitioned holder"),
        "stdout: {text}"
    );
}
