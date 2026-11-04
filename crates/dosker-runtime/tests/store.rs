use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use dosker_core::{ContainerId, ContainerName};
use dosker_platform::EndpointRecord;
use dosker_runtime::{ContainerRecord, StateStore, StoreError};

fn id(text: &str) -> ContainerId {
    ContainerId::parse(text).unwrap()
}

fn name(text: &str) -> ContainerName {
    ContainerName::parse(text).unwrap()
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn record(id_text: &str, name_text: &str, created_at: DateTime<Utc>) -> ContainerRecord {
    ContainerRecord::new(
        id(id_text),
        name(name_text),
        "freedos:latest".to_owned(),
        "mock".to_owned(),
        EndpointRecord::UnixSocket {
            path: PathBuf::from("/tmp/dosker/control.sock"),
        },
        created_at,
    )
}

fn store() -> (tempfile::TempDir, StateStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::new(dir.path().to_path_buf());
    (dir, store)
}

#[test]
fn create_writes_state_and_three_logs() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));

    store.create(&record).unwrap();

    assert_eq!(store.load(&record.id).unwrap(), record);

    let container_dir = store.paths().container_dir(&record.id);
    for log in ["engine.log", "serial.log", "container.log"] {
        let path = container_dir.join(log);
        assert!(path.is_file(), "{} should exist", path.display());
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
    }
}

#[test]
fn create_rejects_duplicate_name_across_ids() {
    let (_dir, store) = store();
    store
        .create(&record("000000000001", "alpha", at(0)))
        .unwrap();

    let error = store
        .create(&record("000000000002", "alpha", at(1)))
        .unwrap_err();

    assert!(matches!(error, StoreError::NameInUse(ref value) if value == &name("alpha")));
}

#[test]
fn create_rejects_duplicate_id() {
    let (_dir, store) = store();
    store
        .create(&record("000000000001", "alpha", at(0)))
        .unwrap();

    let error = store
        .create(&record("000000000001", "beta", at(1)))
        .unwrap_err();

    assert!(matches!(error, StoreError::AlreadyExists(ref value) if value == &id("000000000001")));
}

#[test]
fn create_reclaims_a_stale_name_lock() {
    let (_dir, store) = store();
    let name_dir = store.paths().name_dir(&name("alpha"));
    fs::create_dir_all(&name_dir).unwrap();
    fs::write(name_dir.join("id"), "0000000000ff\n").unwrap();

    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    let locked_id = fs::read_to_string(name_dir.join("id")).unwrap();
    assert_eq!(locked_id.trim(), record.id.as_str());
    assert_eq!(store.find_by_name(&record.name).unwrap(), Some(record));
}

#[test]
fn create_reclaims_a_stale_name_lock_without_id_file() {
    let (_dir, store) = store();
    let name_dir = store.paths().name_dir(&name("alpha"));
    fs::create_dir_all(&name_dir).unwrap();

    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    assert_eq!(store.find_by_name(&record.name).unwrap(), Some(record));
}

#[test]
fn create_reclaims_a_stale_name_lock_with_invalid_id() {
    let (_dir, store) = store();
    let name_dir = store.paths().name_dir(&name("alpha"));
    fs::create_dir_all(&name_dir).unwrap();
    fs::write(name_dir.join("id"), "not-an-id\n").unwrap();

    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    assert_eq!(store.find_by_name(&record.name).unwrap(), Some(record));
}

#[test]
fn load_returns_saved_record() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    assert_eq!(store.load(&record.id).unwrap(), record);
}

#[test]
fn load_missing_returns_not_found() {
    let (_dir, store) = store();
    let missing = id("00000000000f");

    let error = store.load(&missing).unwrap_err();

    assert!(matches!(error, StoreError::NotFound(ref value) if value == &missing));
}

#[test]
fn save_replaces_record() {
    let (_dir, store) = store();
    let mut record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    record
        .transition(dosker_core::Status::Starting, at(1))
        .unwrap();
    store.save(&record).unwrap();

    assert_eq!(
        store.load(&record.id).unwrap().status,
        dosker_core::Status::Starting
    );
}

#[test]
fn list_returns_all_records_sorted_by_created_at_then_id() {
    let (_dir, store) = store();
    let late = record("00000000000a", "late", at(2));
    let early = record("00000000000b", "early", at(0));
    let tie = record("00000000000c", "tie", at(0));
    for item in [&late, &early, &tie] {
        store.create(item).unwrap();
    }

    let listed = store.list().unwrap();

    let ids: Vec<String> = listed.iter().map(|item| item.id.to_string()).collect();
    assert_eq!(
        ids,
        vec![
            early.id.to_string(),
            tie.id.to_string(),
            late.id.to_string()
        ]
    );
}

#[test]
fn list_returns_empty_when_root_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::new(dir.path().join("absent"));

    assert!(store.list().unwrap().is_empty());
}

#[test]
fn find_by_name_returns_match() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    assert_eq!(store.find_by_name(&record.name).unwrap(), Some(record));
}

#[test]
fn find_by_name_on_missing_root_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::new(dir.path().join("absent"));

    assert_eq!(store.find_by_name(&name("alpha")).unwrap(), None);
}

#[test]
fn find_by_name_removes_a_name_dir_whose_container_is_gone() {
    let (_dir, store) = store();
    let name_dir = store.paths().name_dir(&name("alpha"));
    fs::create_dir_all(&name_dir).unwrap();
    fs::write(name_dir.join("id"), "000000000001\n").unwrap();

    assert_eq!(store.find_by_name(&name("alpha")).unwrap(), None);
    assert!(!name_dir.exists());
}

#[test]
fn list_skips_non_container_entries() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();
    fs::create_dir_all(store.paths().containers_dir().join("not-a-container")).unwrap();
    fs::write(store.paths().containers_dir().join("stray"), b"x").unwrap();

    assert_eq!(store.list().unwrap(), vec![record]);
}

#[test]
fn save_writes_pretty_json_with_trailing_newline() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    let path = store.paths().state_file(&record.id);
    let text = fs::read_to_string(&path).unwrap();

    assert!(text.ends_with('\n'));
    assert!(text.contains("\n  \"schema_version\""));
    let loaded: ContainerRecord = serde_json::from_str(&text).unwrap();
    assert_eq!(loaded, record);
}

#[test]
fn load_ignores_unknown_fields_through_the_store() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    let path = store.paths().state_file(&record.id);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["future_field"] = serde_json::json!(true);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

    assert_eq!(store.load(&record.id).unwrap(), record);
}

#[test]
fn corrupt_state_file_is_reported_with_path() {
    let (_dir, store) = store();
    let container_id = id("000000000001");
    let path = store.paths().state_file(&container_id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"this is not json").unwrap();

    let error = store.load(&container_id).unwrap_err();

    match error {
        StoreError::Corrupt { path: reported, .. } => assert_eq!(reported, path),
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn unsupported_schema_version_is_rejected() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    let path = store.paths().state_file(&record.id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut value = serde_json::to_value(&record).unwrap();
    value["schema_version"] = serde_json::json!(2);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

    let error = store.load(&record.id).unwrap_err();

    match error {
        StoreError::Corrupt { path: reported, .. } => assert_eq!(reported, path),
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn remove_deletes_the_container_disk_and_name_lock() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();
    let disk = store.paths().disk_file(&record.id);
    fs::write(&disk, b"disk").unwrap();

    store.remove(&record).unwrap();

    assert!(!disk.exists());
    assert!(!store.paths().container_dir(&record.id).exists());
    assert!(!store.paths().name_dir(&record.name).exists());
}

#[test]
fn remove_keeps_other_containers() {
    let (_dir, store) = store();
    let removed = record("000000000001", "alpha", at(0));
    let kept = record("000000000002", "beta", at(1));
    store.create(&removed).unwrap();
    store.create(&kept).unwrap();

    store.remove(&removed).unwrap();

    assert!(!store.paths().container_dir(&removed.id).exists());
    assert!(store.paths().container_dir(&kept.id).is_dir());
    assert!(store.paths().name_dir(&kept.name).is_dir());
    assert_eq!(store.load(&kept.id).unwrap(), kept);
}

#[test]
fn remove_is_idempotent() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    store.remove(&record).unwrap();
    store.remove(&record).unwrap();

    assert!(!store.paths().container_dir(&record.id).exists());
    assert!(!store.paths().name_dir(&record.name).exists());
}

#[test]
fn remove_leaves_a_name_lock_owned_by_another_id() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();
    let name_dir = store.paths().name_dir(&record.name);
    fs::write(name_dir.join("id"), "000000000002\n").unwrap();

    store.remove(&record).unwrap();

    assert!(!store.paths().container_dir(&record.id).exists());
    assert_eq!(
        fs::read_to_string(name_dir.join("id")).unwrap().trim(),
        "000000000002"
    );
}

#[cfg(unix)]
#[test]
fn remove_removes_recorded_control_and_engine_sockets() {
    let (_dir, store) = store();
    let mut record = record("0000000000f1", "alpha", at(0));
    store.create(&record).unwrap();

    let container_dir = store.paths().container_dir(&record.id);
    let control = container_dir.join("control.sock");
    let agent = container_dir.join("serial_agent.sock");
    let fallback = dosker_platform::short_socket_dir().join(format!("{}-gdb.sock", record.id));
    fs::create_dir_all(fallback.parent().unwrap()).unwrap();
    for path in [&control, &agent, &fallback] {
        fs::write(path, b"socket").unwrap();
    }
    record.sockets.insert(
        "control".to_owned(),
        EndpointRecord::UnixSocket {
            path: control.clone(),
        },
    );
    record.sockets.insert(
        "serial_agent".to_owned(),
        EndpointRecord::UnixSocket {
            path: agent.clone(),
        },
    );
    record.sockets.insert(
        "gdb".to_owned(),
        EndpointRecord::UnixSocket {
            path: fallback.clone(),
        },
    );

    store.remove(&record).unwrap();

    for path in [&control, &agent, &fallback] {
        assert!(!path.exists(), "{} should be gone", path.display());
    }
    assert!(!container_dir.exists());
}

#[cfg(unix)]
#[test]
fn root_containers_and_container_dir_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    let mode = |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;

    assert_eq!(mode(dir.path()), 0o700);
    assert_eq!(mode(&store.paths().containers_dir()), 0o700);
    assert_eq!(mode(&store.paths().container_dir(&record.id)), 0o700);
}

#[test]
fn log_file_names_are_engine_serial_and_container() {
    let (_dir, store) = store();
    let record = record("000000000001", "alpha", at(0));
    store.create(&record).unwrap();

    let mut names: Vec<String> = fs::read_dir(store.paths().container_dir(&record.id))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();

    assert_eq!(
        names,
        vec!["container.log", "engine.log", "serial.log", "state.json"]
    );
}
