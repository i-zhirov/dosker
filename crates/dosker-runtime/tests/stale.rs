use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, TimeZone, Utc};
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_platform::{EndpointRecord, PlatformError};
use dosker_runtime::stale::{HostProbe, recover_stale};
use dosker_runtime::{ContainerRecord, StateStore, StoreError};

#[derive(Default)]
struct FakeProbe {
    live_pids: HashSet<u32>,
    dead_endpoints: HashSet<String>,
    killed: Mutex<Vec<u32>>,
    fail_kill: bool,
}

impl FakeProbe {
    fn with_live_pids(pids: impl IntoIterator<Item = u32>) -> Self {
        Self {
            live_pids: pids.into_iter().collect(),
            ..Self::default()
        }
    }

    fn dead_endpoint(mut self, endpoint: &EndpointRecord) -> Self {
        self.dead_endpoints.insert(key(endpoint));
        self
    }

    fn with_kill_error(mut self) -> Self {
        self.fail_kill = true;
        self
    }

    fn killed(&self) -> Vec<u32> {
        self.killed.lock().unwrap().clone()
    }
}

impl HostProbe for FakeProbe {
    fn process_alive(&self, pid: u32) -> bool {
        self.live_pids.contains(&pid)
    }

    fn endpoint_alive(&self, endpoint: &EndpointRecord) -> bool {
        !self.dead_endpoints.contains(&key(endpoint))
    }

    fn kill_group(&self, pgid: u32) -> Result<(), PlatformError> {
        self.killed.lock().unwrap().push(pgid);
        if self.fail_kill {
            return Err(PlatformError::Unsupported("kill failed"));
        }
        Ok(())
    }
}

fn key(endpoint: &EndpointRecord) -> String {
    serde_json::to_string(endpoint).unwrap()
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn make_record(root: &Path, id: &str, name: &str) -> ContainerRecord {
    let id = ContainerId::parse(id).unwrap();
    let name = ContainerName::parse(name).unwrap();
    let endpoint = dosker_platform::endpoint::control_endpoint(root, &id);
    ContainerRecord::new(
        id,
        name,
        "freedos:latest".to_owned(),
        "mock".to_owned(),
        endpoint.record(),
        at(0),
    )
}

fn store_with(record: &ContainerRecord) -> (tempfile::TempDir, StateStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::new(dir.path().to_path_buf());
    store.create(record).unwrap();
    (dir, store)
}

fn running(record: &mut ContainerRecord, pid: u32) {
    record.status = Status::Running;
    record.pid = Some(pid);
    record.engine_pid = Some(pid + 1);
    record.engine_pgid = Some(pid + 1);
    record.started_at = Some(at(1));
}

#[test]
fn dead_supervisor_marks_running_container_exited() {
    let mut record = make_record(
        Path::new("/tmp/dosker-stale"),
        "000000000001",
        "dead-supervisor",
    );
    running(&mut record, 1000);
    let (dir, store) = store_with(&record);

    let probe = FakeProbe::default();
    let changed = recover_stale(&store, &probe, at(100)).unwrap();

    assert_eq!(changed, vec![record.id.clone()]);
    let loaded = store.load(&record.id).unwrap();
    assert_eq!(loaded.status, Status::Exited);
    assert_eq!(loaded.pid, None);
    drop(dir);
}

#[test]
fn dead_endpoint_marks_running_container_exited_even_when_pid_is_alive() {
    let mut record = make_record(
        Path::new("/tmp/dosker-stale"),
        "000000000002",
        "dead-endpoint",
    );
    running(&mut record, 1001);
    let endpoint = record.control_endpoint().unwrap().clone();
    let (dir, store) = store_with(&record);

    let probe = FakeProbe::with_live_pids([1001]).dead_endpoint(&endpoint);
    let changed = recover_stale(&store, &probe, at(100)).unwrap();

    assert_eq!(changed, vec![record.id.clone()]);
    assert_eq!(store.load(&record.id).unwrap().status, Status::Exited);
    drop(dir);
}

#[test]
fn missing_supervisor_pid_marks_starting_container_exited() {
    let mut record = make_record(
        Path::new("/tmp/dosker-stale"),
        "000000000003",
        "missing-pid",
    );
    record.status = Status::Starting;
    let (dir, store) = store_with(&record);

    let probe = FakeProbe::default();
    let changed = recover_stale(&store, &probe, at(100)).unwrap();

    assert_eq!(changed, vec![record.id.clone()]);
    assert_eq!(store.load(&record.id).unwrap().status, Status::Exited);
    drop(dir);
}

#[test]
fn dead_endpoint_leaves_a_starting_container_untouched_when_the_pid_is_alive() {
    let mut record = make_record(
        Path::new("/tmp/dosker-stale"),
        "000000000004",
        "starting-live",
    );
    record.status = Status::Starting;
    record.pid = Some(1002);
    let endpoint = record.control_endpoint().unwrap().clone();
    let (dir, store) = store_with(&record);

    let probe = FakeProbe::with_live_pids([1002]).dead_endpoint(&endpoint);
    let changed = recover_stale(&store, &probe, at(100)).unwrap();

    assert!(changed.is_empty());
    let loaded = store.load(&record.id).unwrap();
    assert_eq!(loaded.status, Status::Starting);
    assert_eq!(loaded.pid, Some(1002));
    drop(dir);
}

#[test]
fn live_supervisor_and_endpoint_are_left_untouched() {
    let mut record = make_record(Path::new("/tmp/dosker-stale"), "000000000005", "live");
    running(&mut record, 1003);
    let (dir, store) = store_with(&record);

    let probe = FakeProbe::with_live_pids([1003]);
    let changed = recover_stale(&store, &probe, at(100)).unwrap();

    assert!(changed.is_empty());
    let loaded = store.load(&record.id).unwrap();
    assert_eq!(loaded.status, Status::Running);
    assert_eq!(loaded.pid, Some(1003));
    assert!(!loaded.dirty);
    drop(dir);
}

#[test]
fn stale_engine_group_is_killed() {
    let mut record = make_record(Path::new("/tmp/dosker-stale"), "000000000006", "kill-group");
    running(&mut record, 1004);
    let (dir, store) = store_with(&record);

    let probe = FakeProbe::with_live_pids([1005]);
    let changed = recover_stale(&store, &probe, at(100)).unwrap();

    assert_eq!(changed, vec![record.id.clone()]);
    assert_eq!(probe.killed(), vec![1005]);
    drop(dir);
}

#[test]
fn kill_group_error_leaves_the_record_untouched() {
    let mut record = make_record(Path::new("/tmp/dosker-stale"), "000000000008", "kill-error");
    running(&mut record, 1008);
    let (dir, store) = store_with(&record);

    let probe = FakeProbe::with_live_pids([1009]).with_kill_error();
    let error = recover_stale(&store, &probe, at(100)).unwrap_err();

    assert!(matches!(error, StoreError::Platform(_)));
    assert_eq!(probe.killed(), vec![1009]);
    let loaded = store.load(&record.id).unwrap();
    assert_eq!(loaded.status, Status::Running);
    assert_eq!(loaded.pid, Some(1008));
    assert_eq!(loaded.engine_pgid, Some(1009));
    assert!(!loaded.dirty);
    drop(dir);
}

#[test]
fn stale_recovery_sets_dirty_and_clears_pids() {
    let mut record = make_record(Path::new("/tmp/dosker-stale"), "000000000007", "dirty");
    running(&mut record, 1006);
    record.exit_code = Some(7);
    let (dir, store) = store_with(&record);

    let probe = FakeProbe::default();
    recover_stale(&store, &probe, at(100)).unwrap();

    let loaded = store.load(&record.id).unwrap();
    assert_eq!(loaded.status, Status::Exited);
    assert_eq!(loaded.pid, None);
    assert_eq!(loaded.engine_pid, None);
    assert_eq!(loaded.engine_pgid, None);
    assert_eq!(loaded.exit_code, None);
    assert_eq!(loaded.finished_at, Some(at(100)));
    assert!(loaded.dirty);
    drop(dir);
}

#[test]
fn stale_recovery_returns_only_changed_ids() {
    let mut stale = make_record(Path::new("/tmp/dosker-stale"), "000000000008", "stale-one");
    running(&mut stale, 1007);
    let mut live = make_record(Path::new("/tmp/dosker-stale"), "000000000009", "live-one");
    running(&mut live, 1008);

    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::new(dir.path().to_path_buf());
    store.create(&stale).unwrap();
    store.create(&live).unwrap();

    let probe = FakeProbe::with_live_pids([1008]);
    let changed = recover_stale(&store, &probe, at(100)).unwrap();

    assert_eq!(changed, vec![stale.id.clone()]);
    assert_eq!(store.load(&live.id).unwrap().status, Status::Running);
    drop(dir);
}

#[test]
fn non_active_statuses_are_untouched() {
    let statuses = [
        (Status::Created, "00000000000a", "created-one"),
        (Status::Stopped, "00000000000b", "stopped-one"),
        (Status::Exited, "00000000000c", "exited-one"),
        (Status::Failed, "00000000000d", "failed-one"),
        (Status::Removed, "00000000000e", "removed-one"),
    ];

    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::new(dir.path().to_path_buf());
    let mut records = Vec::new();
    for (status, id, name) in statuses {
        let mut record = make_record(dir.path(), id, name);
        record.status = status;
        record.pid = Some(2000);
        store.create(&record).unwrap();
        records.push(record);
    }

    let probe = FakeProbe::default();
    let changed = recover_stale(&store, &probe, at(100)).unwrap();

    assert!(changed.is_empty());
    for record in &records {
        let loaded = store.load(&record.id).unwrap();
        assert_eq!(loaded.status, record.status);
        assert_eq!(loaded.pid, record.pid);
        assert!(!loaded.dirty);
    }
    assert!(probe.killed().is_empty());
    drop(dir);
}

#[test]
fn stale_recovery_is_idempotent() {
    let mut record = make_record(Path::new("/tmp/dosker-stale"), "00000000000f", "idempotent");
    running(&mut record, 1009);
    let (dir, store) = store_with(&record);

    let probe = FakeProbe::default();
    let first = recover_stale(&store, &probe, at(100)).unwrap();
    let second = recover_stale(&store, &probe, at(101)).unwrap();

    assert_eq!(first, vec![record.id.clone()]);
    assert!(second.is_empty());
    assert_eq!(store.load(&record.id).unwrap().finished_at, Some(at(100)));
    drop(dir);
}
