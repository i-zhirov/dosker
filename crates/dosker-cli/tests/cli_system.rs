mod common;

use std::fs;
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use common::{CliRoot, stderr_text, stdout_text};
use dosker_core::{ContainerId, Status};
use dosker_image::{
    CacheRecord, CacheStore, ImageDigest, ImageRef, ImageReference, OciStore, sha256_digest,
};
use dosker_platform::process::{ProcessSupervisor, system};

fn prune(root: &CliRoot) -> std::process::Output {
    root.run(&["system", "prune"])
}

fn id_from(output: &std::process::Output) -> ContainerId {
    ContainerId::parse(stdout_text(output).trim()).expect("parse the container id")
}

fn create_container(root: &CliRoot) -> ContainerId {
    root.import_image();
    let output = root.run(&["create", "freedos:latest"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    id_from(&output)
}

fn cache(root: &CliRoot) -> CacheStore {
    CacheStore::new(root.path().to_path_buf())
}

fn record(key: &str, layer_digest: &str) -> CacheRecord {
    CacheRecord {
        schema_version: dosker_image::cache::CACHE_SCHEMA_VERSION,
        key: key.to_owned(),
        layer_digest: layer_digest.to_owned(),
        instruction: "COPY A.TXT A.TXT".to_owned(),
        created_at: Utc.timestamp_opt(1, 0).unwrap(),
    }
}

#[test]
fn prune_removes_a_stopped_container() {
    let root = CliRoot::new();
    let id = root.start();
    assert_eq!(root.run(&["stop", id.as_str()]).status.code(), Some(0));

    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(root.store().load(&id).is_err());
    let ps = root.run(&["ps", "-a"]);
    assert!(!stdout_text(&ps).contains(id.as_str()));
}

#[test]
fn prune_removes_a_created_container() {
    let root = CliRoot::new();
    let id = create_container(&root);
    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(root.store().load(&id).is_err());
    assert!(!stdout_text(&root.run(&["ps", "-a"])).contains(id.as_str()));
}

#[test]
fn prune_removes_a_failed_container() {
    let root = CliRoot::new();
    let id = create_container(&root);
    let mut record = root.store().load(&id).unwrap();
    record.status = Status::Failed;
    root.store().save(&record).unwrap();

    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(root.store().load(&id).is_err());
}

#[test]
fn prune_keeps_a_running_container() {
    let root = CliRoot::new();
    let id = root.start();
    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(root.store().load(&id).unwrap().status, Status::Running);
    assert!(stdout_text(&root.run(&["ps"])).contains(id.as_str()));
    root.kill(root.engine_pid(&id));
}

#[test]
fn prune_removes_an_untagged_image() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let store = OciStore::new(root.path().to_path_buf());
    let image = store
        .load(&ImageRef::Tag(
            ImageReference::parse("demo:latest").unwrap(),
        ))
        .unwrap();
    assert_eq!(root.run(&["rmi", "demo:latest"]).status.code(), Some(0));

    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        store
            .load(&ImageRef::Digest(
                ImageDigest::parse(&image.manifest_digest).unwrap()
            ))
            .is_err()
    );
    assert!(!store.blob_path(&image.manifest_digest).unwrap().exists());
    assert!(!store.blob_path(&image.config_digest).unwrap().exists());
    for digest in image.layer_digests {
        assert!(!store.blob_path(&digest).unwrap().exists());
    }
    if let Some(raw) = image.raw_disk_digest {
        assert!(!store.blob_path(&raw).unwrap().exists());
    }
}

#[test]
fn prune_removes_an_untagged_manifest_with_a_missing_config() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let store = OciStore::new(root.path().to_path_buf());
    let image = store
        .load(&ImageRef::Tag(
            ImageReference::parse("demo:latest").unwrap(),
        ))
        .unwrap();
    assert_eq!(root.run(&["rmi", "demo:latest"]).status.code(), Some(0));
    fs::remove_file(store.blob_path(&image.config_digest).unwrap()).unwrap();

    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(!store.blob_path(&image.manifest_digest).unwrap().exists());
}

#[test]
fn prune_keeps_an_untagged_image_used_by_a_running_container() {
    let root = CliRoot::new();
    let id = root.start();
    let store = OciStore::new(root.path().to_path_buf());
    let image = store
        .load(&ImageRef::Tag(
            ImageReference::parse("freedos:latest").unwrap(),
        ))
        .unwrap();
    assert_eq!(root.run(&["rmi", "freedos:latest"]).status.code(), Some(0));

    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        store
            .load(&ImageRef::Digest(
                ImageDigest::parse(&image.manifest_digest).unwrap()
            ))
            .is_ok()
    );
    assert_eq!(root.store().load(&id).unwrap().status, Status::Running);
    root.kill(root.engine_pid(&id));
}

#[test]
fn prune_removes_a_stale_cache_entry() {
    let root = CliRoot::new();
    cache(&root)
        .store(&record("stale", &sha256_digest(b"missing layer")))
        .unwrap();
    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(cache(&root).load("stale").unwrap().is_none());
}

#[test]
fn prune_keeps_a_valid_cache_entry() {
    let root = CliRoot::new();
    let blob = OciStore::new(root.path().to_path_buf())
        .put_blob(b"layer")
        .unwrap();
    cache(&root).store(&record("valid", &blob.digest)).unwrap();
    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(cache(&root).load("valid").unwrap().is_some());
}

#[test]
fn prune_reports_the_removed_counts() {
    let root = CliRoot::new();
    let id = root.start();
    assert_eq!(root.run(&["stop", id.as_str()]).status.code(), Some(0));
    root.import_image_as("demo:latest");
    assert_eq!(root.run(&["rmi", "demo:latest"]).status.code(), Some(0));
    cache(&root)
        .store(&record("stale", &sha256_digest(b"missing layer")))
        .unwrap();

    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(
        stdout_text(&output).trim_end(),
        "removed 1 containers, 1 images, 1 cache entries"
    );
}

#[test]
fn prune_reports_zeros_on_a_fresh_root() {
    let root = CliRoot::new();
    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(
        stdout_text(&output).trim_end(),
        "removed 0 containers, 0 images, 0 cache entries"
    );
}

#[cfg(unix)]
#[test]
fn prune_recovers_a_stale_supervisor_before_removing() {
    let root = CliRoot::new();
    let id = root.start();
    let engine_pid = root.engine_pid(&id).expect("the engine pid is recorded");
    root.kill_supervisor(&id);
    std::thread::sleep(Duration::from_millis(100));

    let output = prune(&root);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(root.store().load(&id).is_err());

    let deadline = Instant::now() + Duration::from_secs(10);
    while system().process_alive(engine_pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !system().process_alive(engine_pid),
        "the engine is still alive"
    );
}
