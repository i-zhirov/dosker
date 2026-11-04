//! End-to-end checks for the container lifecycle commands.

mod common;

use std::fs;
use std::process::Output;
use std::time::{Duration, Instant};

use chrono::Utc;
use common::{CliRoot, stderr_text, stdout_text};
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_engine::DisplayMode;
use dosker_image::{ImageReference, ImageStore};
#[cfg(unix)]
use dosker_platform::control::bind;
use dosker_platform::endpoint::control_endpoint;
use dosker_runtime::ContainerRecord;

fn stdout_lines(output: &Output) -> Vec<String> {
    stdout_text(output).lines().map(str::to_owned).collect()
}

fn id_from(output: &Output) -> ContainerId {
    ContainerId::parse(stdout_text(output).trim()).expect("parse the printed id")
}

fn create(root: &CliRoot, name: &str) -> ContainerId {
    let output = root.run(&["create", "freedos:latest", "--name", name]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    id_from(&output)
}

fn seed_failed(root: &CliRoot, name: &str) -> ContainerId {
    let id = ContainerId::generate();
    let name = ContainerName::parse(name).expect("parse the seed name");
    let endpoint = control_endpoint(root.path(), &id);
    let mut record = ContainerRecord::new(
        id.clone(),
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        Utc::now(),
    );
    record.status = Status::Failed;
    root.store()
        .create(&record)
        .expect("create the failed record");
    id
}

fn set_dirty(root: &CliRoot, id: &ContainerId) {
    let mut record = root.store().load(id).expect("load the container record");
    record.dirty = true;
    root.store().save(&record).expect("save the dirty record");
}

#[test]
fn create_copies_the_image_to_a_private_disk() {
    let root = CliRoot::new();
    root.import_image_as("freedos-test:latest");

    let output = root.run(&["create", "freedos-test:latest", "--name", "alpha"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let id = id_from(&output);
    let record = root.store().load(&id).expect("load the container record");
    let disk = record.disk_path.expect("the record has a private disk");
    assert!(disk.is_file(), "missing {}", disk.display());

    let image = root.path().join("images/freedos-test/latest/disk.img");
    assert_eq!(
        fs::read(&disk).expect("read the private disk"),
        fs::read(&image).expect("read the image disk")
    );
}

#[test]
fn create_does_not_modify_the_image() {
    let root = CliRoot::new();
    root.import_image_as("freedos-test:latest");
    let image = root.path().join("images/freedos-test/latest/disk.img");
    let before = fs::read(&image).expect("read the image disk");

    let output = root.run(&["create", "freedos-test:latest", "--name", "alpha"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let after = fs::read(&image).expect("read the image disk");
    assert_eq!(before, after);
}

#[test]
fn create_records_the_digest_and_booter_flag() {
    let root = CliRoot::new();
    root.import_image_as("freedos-test:latest");

    let blob = root.path().join("boot.img");
    fs::write(&blob, b"an opaque booter blob").expect("write booter blob");
    let booter = root.run(&[
        "image",
        "import",
        "--booter",
        blob.to_str().expect("blob path is utf-8"),
        "booter-test:latest",
    ]);
    assert_eq!(
        booter.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&booter)
    );

    let images = ImageStore::new(root.path().to_path_buf());
    let normal_reference = ImageReference::parse("freedos-test:latest").expect("parse reference");
    let booter_reference = ImageReference::parse("booter-test:latest").expect("parse reference");
    let normal_record = images
        .load(&normal_reference)
        .expect("load the normal image");
    let booter_record = images
        .load(&booter_reference)
        .expect("load the booter image");

    let output = root.run(&["create", "freedos-test:latest", "--name", "alpha"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let normal = root
        .store()
        .load(&id_from(&output))
        .expect("load the normal container");
    assert_eq!(normal.image_digest, normal_record.digest);
    assert!(!normal.booter);

    let output = root.run(&["create", "booter-test:latest", "--name", "booted"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let booted = root
        .store()
        .load(&id_from(&output))
        .expect("load the booter container");
    assert_eq!(booted.image_digest, booter_record.digest);
    assert!(booted.booter);
}

#[test]
fn create_resolves_an_untagged_image_reference() {
    let root = CliRoot::new();
    root.import_image_as("freedos-test:latest");

    let output = root.run(&["create", "freedos-test", "--name", "untagged"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );

    let record = root
        .store()
        .load(&id_from(&output))
        .expect("load the container record");
    assert_eq!(record.image_digest.len(), "sha256:".len() + 64);
}

#[test]
fn create_rejects_a_missing_image() {
    let root = CliRoot::new();
    let output = root.run(&["create", "missing:latest"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_text(&output).contains("image not found"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn create_rejects_a_duplicate_name() {
    let root = CliRoot::new();
    root.import_image_as("freedos-test:latest");
    let first = root.run(&["create", "freedos-test:latest", "--name", "duplicate"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&first)
    );

    let second = root.run(&["create", "freedos-test:latest", "--name", "duplicate"]);
    assert_eq!(second.status.code(), Some(1));
}

#[test]
fn create_rejects_an_unknown_engine() {
    let root = CliRoot::new();
    let output = root.run(&["create", "freedos-test:latest", "--engine", "nope"]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn stop_on_a_created_container_exits_125() {
    let root = CliRoot::new();
    root.import_image();
    let id = create(&root, "never-started");

    let output = root.run(&["stop", id.as_str()]);

    assert_eq!(
        output.status.code(),
        Some(125),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert_eq!(
        stdout_lines(&output),
        vec![String::from("never-started created")]
    );
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Created);
}

#[test]
fn start_returns_a_stopped_container_to_running() {
    let root = CliRoot::new();
    let id = root.start();

    let stop = root.run(&["stop", id.as_str()]);
    assert_eq!(
        stop.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&stop)
    );

    let start = root.run(&["start", id.as_str()]);
    assert_eq!(
        start.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&start),
        stderr_text(&start)
    );
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Running);
    assert!(
        record.disk_path.expect("the disk is kept").is_file(),
        "the private disk is missing"
    );
}

#[test]
fn start_rejects_a_running_container() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["start", id.as_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_text(&output).contains("running"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn start_rejects_a_failed_container() {
    let root = CliRoot::new();
    let id = seed_failed(&root, "failed-one");

    let output = root.run(&["start", "failed-one"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_text(&output).contains("rm"),
        "stderr: {}",
        stderr_text(&output)
    );

    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Failed);
}

#[test]
fn start_warns_when_the_disk_is_dirty() {
    let root = CliRoot::new();
    root.import_image();
    let id = create(&root, "dirty-one");
    set_dirty(&root, &id);

    let output = root.run(&["start", "dirty-one"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("dirty"),
        "stderr: {}",
        stderr_text(&output)
    );
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Running);
}

#[test]
fn start_prints_name_and_status() {
    let root = CliRoot::new();
    root.import_image();
    let id = create(&root, "alpha");

    let output = root.run(&["start", "alpha"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(stdout_lines(&output), vec![String::from("alpha running")]);
    let _ = id;
}

#[test]
fn start_reports_a_fast_exit_without_waiting_for_the_deadline() {
    let root = CliRoot::new();
    root.import_image();
    let id = create(&root, "fast-exit");

    let mut record = root.store().load(&id).expect("load the container record");
    record.status = Status::Exited;
    record.pid = None;
    record.engine_pid = None;
    record.engine_pgid = None;
    record.engine = String::from("missing");
    root.store().save(&record).expect("save the exited record");

    let started = Instant::now();
    let output = root.run(&["start", "fast-exit"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_lines(&output),
        vec![String::from("fast-exit exited")],
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "start waited for the deadline: {:?}",
        started.elapsed()
    );
    assert!(
        !stderr_text(&output).contains("unknown engine"),
        "the timed-out path printed the container log: {}",
        stderr_text(&output)
    );
}

#[test]
fn restart_stops_and_starts() {
    let root = CliRoot::new();
    let id = root.start();
    let before = root
        .store()
        .load(&id)
        .expect("load the container record")
        .pid;

    let output = root.run(&["restart", id.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(
        stdout_lines(&output),
        vec![format!("{} running", root.store().load(&id).unwrap().name)]
    );

    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Running);
    assert!(record.pid.is_some());
    assert_ne!(record.pid, before);
}

#[test]
fn restart_starts_a_stopped_container() {
    let root = CliRoot::new();
    let id = root.start();
    let stop = root.run(&["stop", id.as_str()]);
    assert_eq!(
        stop.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&stop)
    );

    let output = root.run(&["restart", id.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Running);
}

#[test]
fn restart_rejects_a_failed_container() {
    let root = CliRoot::new();
    seed_failed(&root, "failed-one");

    let output = root.run(&["restart", "failed-one"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_text(&output).contains("rm"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn restart_warns_when_the_disk_is_dirty() {
    let root = CliRoot::new();
    root.import_image();
    let id = create(&root, "dirty-one");
    set_dirty(&root, &id);

    let output = root.run(&["restart", "dirty-one"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("dirty"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn rm_removes_the_container_and_disk() {
    let root = CliRoot::new();
    root.import_image();
    let id = create(&root, "alpha");
    let record = root.store().load(&id).expect("load the container record");
    let disk = record.disk_path.clone().expect("the record has a disk");
    let container_dir = disk.parent().expect("the disk has a parent").to_path_buf();

    let output = root.run(&["rm", "alpha"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(stdout_lines(&output), vec![String::from("alpha")]);
    assert!(root.store().load(&id).is_err());
    assert!(!disk.exists(), "the disk {} still exists", disk.display());
    assert!(
        !container_dir.exists(),
        "the container directory {} still exists",
        container_dir.display()
    );
}

#[test]
fn rm_rejects_a_running_container() {
    let root = CliRoot::new();
    let id = root.start();

    let output = root.run(&["rm", id.as_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_text(&output).contains("running"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn restart_does_not_start_when_stop_times_out() {
    use std::thread;

    let root = CliRoot::new();
    let id = ContainerId::generate();
    let name = ContainerName::parse("stuck").expect("parse the name");
    let endpoint = control_endpoint(root.path(), &id);

    let listener = bind(&endpoint).expect("bind the stub endpoint");
    thread::spawn(move || {
        let mut held = Vec::new();
        loop {
            match listener.accept(Duration::from_secs(30)) {
                Ok(Some(connection)) => held.push(connection),
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });

    let mut record = ContainerRecord::new(
        id.clone(),
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        Utc::now(),
    );
    record.status = Status::Running;
    let mut supervisor = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("start a stand-in supervisor");
    record.pid = Some(supervisor.id());
    let original_pid = record.pid;
    root.store()
        .create(&record)
        .expect("create the running record");

    let output = root.run(&["restart", "--time", "1", id.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );

    let stored = root.store().load(&id).expect("load the container record");
    assert_eq!(stored.pid, original_pid);
    let _ = supervisor.kill();
    let _ = supervisor.wait();
}

#[cfg(unix)]
#[test]
fn restart_warns_when_stop_leaves_the_disk_dirty() {
    use std::io::BufReader;
    use std::sync::{Arc, Mutex};

    use dosker_runtime::protocol::{FrameRead, Reply, Response, read_frame, write_message};

    let root = CliRoot::new();
    root.import_image();
    let id = create(&root, "unclean");
    let endpoint = control_endpoint(root.path(), &id);

    let mut record = root.store().load(&id).expect("load the container record");
    record.status = Status::Running;
    let supervisor = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("start a stand-in supervisor");
    record.pid = Some(supervisor.id());
    root.store().save(&record).expect("save the running record");

    let listener = bind(&endpoint).expect("bind the stub endpoint");
    let store = root.store();
    let record_id = id.clone();
    let supervisor = Arc::new(Mutex::new(Some(supervisor)));
    let worker = Arc::clone(&supervisor);
    std::thread::spawn(move || {
        loop {
            match listener.accept(Duration::from_secs(5)) {
                Ok(Some(connection)) => {
                    let store = store.clone();
                    let record_id = record_id.clone();
                    let supervisor = Arc::clone(&worker);
                    std::thread::spawn(move || {
                        let mut reader = BufReader::new(connection);
                        let Ok(FrameRead::Line(_)) = read_frame(&mut reader) else {
                            return;
                        };
                        if let Some(mut child) = supervisor.lock().unwrap().take() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        let mut record = store.load(&record_id).expect("load the container record");
                        record.status = Status::Exited;
                        record.pid = None;
                        record.dirty = true;
                        record.engine = String::from("missing");
                        store.save(&record).expect("save the exited record");
                        let response = Response::ok(Reply::Stop {
                            status: Status::Exited,
                        });
                        let _ = write_message(reader.get_mut(), &response);
                    });
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });

    let output = root.run(&["restart", id.as_str()]);

    if let Some(mut child) = supervisor.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let stderr = stderr_text(&output);
    assert_eq!(
        stderr.matches("dirty").count(),
        1,
        "the unclean stop warning was missing or repeated: {stderr}"
    );
}

#[test]
fn create_records_the_requested_display_and_vnc_port() {
    let root = CliRoot::new();
    root.import_image_as("freedos-test:latest");

    let output = root.run(&[
        "create",
        "freedos-test:latest",
        "--name",
        "displayed",
        "--display",
        "vnc",
        "--vnc-port",
        "5901",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let record = root
        .store()
        .load(&id_from(&output))
        .expect("load the container record");
    assert_eq!(record.display, DisplayMode::Vnc);
    assert_eq!(record.vnc_port, Some(5901));
}

#[test]
fn create_rejects_an_invalid_display() {
    let root = CliRoot::new();
    let output = root.run(&["create", "freedos-test:latest", "--display", "broken"]);

    assert_eq!(output.status.code(), Some(2));
}
