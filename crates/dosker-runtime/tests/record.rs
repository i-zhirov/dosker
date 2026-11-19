use std::net::Ipv4Addr;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use dosker_core::{ContainerId, ContainerName, CoreError, Status};
use dosker_engine::DisplayMode;
use dosker_image::PortProtocol;
use dosker_platform::EndpointRecord;
use dosker_runtime::record::RECORD_SCHEMA_VERSION;
use dosker_runtime::{CONTROL_SOCKET, ContainerRecord, PortMapping, VolumeKind, VolumeMount};

fn sample_id() -> ContainerId {
    ContainerId::parse("0123456789ab").unwrap()
}

fn sample_name() -> ContainerName {
    ContainerName::parse("dos-app").unwrap()
}

fn sample_control() -> EndpointRecord {
    EndpointRecord::UnixSocket {
        path: PathBuf::from("/tmp/dosker/control.sock"),
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn sample_record() -> ContainerRecord {
    ContainerRecord::new(
        sample_id(),
        sample_name(),
        "freedos:latest".to_owned(),
        "mock".to_owned(),
        sample_control(),
        at(0),
    )
}

fn advance(record: &mut ContainerRecord, path: &[Status]) {
    for (index, next) in path.iter().enumerate() {
        record.transition(*next, at(index as i64 + 1)).unwrap();
    }
}

#[test]
fn new_record_starts_created() {
    let record = sample_record();

    assert_eq!(record.schema_version, RECORD_SCHEMA_VERSION);
    assert_eq!(record.schema_version, 1);
    assert_eq!(record.status, Status::Created);
    assert_eq!(record.engine_version, "");
    assert_eq!(record.disk_path, None);
    assert!(!record.dirty);
    assert_eq!(record.pid, None);
    assert_eq!(record.engine_pid, None);
    assert_eq!(record.engine_pgid, None);
    assert_eq!(record.exit_code, None);
    assert_eq!(record.started_at, None);
    assert_eq!(record.finished_at, None);
}

#[test]
fn new_record_starts_with_runtime_defaults() {
    let record = sample_record();

    assert_eq!(record.image_digest, "");
    assert!(!record.booter);
    assert_eq!(record.display, DisplayMode::None);
    assert_eq!(record.vnc_port, None);
    assert_eq!(record.cmd, None);
    assert_eq!(record.engine_command, None);
}

#[test]
fn a_new_record_has_no_ports_or_volumes_and_uses_the_ne2000_network() {
    let record = sample_record();

    assert!(record.ports.is_empty());
    assert!(record.volumes.is_empty());
    assert_eq!(record.net, "ne2000");
}

#[test]
fn with_network_records_the_ports_and_the_network_device() {
    let mapping = PortMapping {
        host_address: Ipv4Addr::LOCALHOST,
        host_port: 8080,
        guest_port: 80,
        protocol: PortProtocol::Tcp,
    };
    let record = sample_record().with_network("none".to_owned(), vec![mapping]);

    assert_eq!(record.net, "none");
    assert_eq!(record.ports, vec![mapping]);
}

#[test]
fn with_volumes_records_the_mounts() {
    let named = VolumeMount {
        drive: "D:".to_owned(),
        kind: VolumeKind::Named,
        name: Some("data".to_owned()),
        host_path: None,
        read_only: false,
    };
    let bind = VolumeMount {
        drive: "E:".to_owned(),
        kind: VolumeKind::Bind,
        name: None,
        host_path: Some(PathBuf::from("/tmp/dosker/host")),
        read_only: true,
    };
    let record = sample_record().with_volumes(vec![named.clone(), bind.clone()]);

    assert_eq!(record.volumes, vec![named, bind]);
}

#[test]
fn a_port_mapping_renders_its_endpoint() {
    let mapping = PortMapping {
        host_address: Ipv4Addr::LOCALHOST,
        host_port: 8080,
        guest_port: 80,
        protocol: PortProtocol::Tcp,
    };

    assert_eq!(mapping.endpoint(), "127.0.0.1:8080->80/tcp");
}

#[test]
fn with_image_sets_digest_booter_and_cmd() {
    let record = sample_record().with_image(
        "sha256:0123456789abcdef".to_owned(),
        true,
        Some("GAME.EXE".to_owned()),
    );

    assert_eq!(record.image_digest, "sha256:0123456789abcdef");
    assert!(record.booter);
    assert_eq!(record.cmd.as_deref(), Some("GAME.EXE"));
}

#[test]
fn with_display_sets_mode_and_port() {
    let record = sample_record().with_display(DisplayMode::Vnc, Some(5901));

    assert_eq!(record.display, DisplayMode::Vnc);
    assert_eq!(record.vnc_port, Some(5901));
}

#[test]
fn with_disk_sets_the_disk_path() {
    let path = PathBuf::from("/tmp/dosker/containers/0123456789ab/disk.img");
    let record = sample_record().with_disk(path.clone());

    assert_eq!(record.disk_path, Some(path));
}

#[test]
fn new_record_inserts_the_control_socket() {
    let control = sample_control();
    let record = ContainerRecord::new(
        sample_id(),
        sample_name(),
        "freedos:latest".to_owned(),
        "mock".to_owned(),
        control.clone(),
        at(0),
    );

    assert_eq!(record.sockets.get(CONTROL_SOCKET), Some(&control));
    assert_eq!(record.control_endpoint(), Some(&control));
}

#[test]
fn transition_sets_started_at_once() {
    let mut record = sample_record();
    advance(&mut record, &[Status::Starting]);
    record.transition(Status::Running, at(10)).unwrap();

    assert_eq!(record.started_at, Some(at(10)));

    record.transition(Status::Running, at(20)).unwrap();

    assert_eq!(record.started_at, Some(at(10)));
}

#[test]
fn transition_sets_finished_at_for_outcomes() {
    let outcomes = [
        (
            Status::Stopped,
            vec![
                Status::Starting,
                Status::Running,
                Status::Stopping,
                Status::Stopped,
            ],
        ),
        (
            Status::Exited,
            vec![Status::Starting, Status::Running, Status::Exited],
        ),
        (Status::Failed, vec![Status::Starting, Status::Failed]),
    ];

    for (outcome, path) in outcomes {
        let mut record = sample_record();
        advance(&mut record, &path);

        assert_eq!(record.status, outcome);
        assert_eq!(
            record.finished_at,
            Some(at(path.len() as i64)),
            "{outcome} should set finished_at"
        );
    }
}

#[test]
fn transition_repeated_outcome_keeps_finished_at() {
    let mut record = sample_record();
    advance(
        &mut record,
        &[Status::Starting, Status::Running, Status::Exited],
    );
    let finished_at = record.finished_at;

    record.transition(Status::Exited, at(10)).unwrap();

    assert_eq!(record.finished_at, finished_at);
}

#[test]
fn transition_allows_starting_to_exited() {
    let mut record = sample_record();
    advance(&mut record, &[Status::Starting]);

    record.transition(Status::Exited, at(2)).unwrap();

    assert_eq!(record.status, Status::Exited);
    assert_eq!(record.finished_at, Some(at(2)));
}

#[test]
fn transition_rejects_a_forbidden_edge() {
    let mut record = sample_record();

    let error = record.transition(Status::Running, at(1)).unwrap_err();

    assert!(matches!(
        error,
        CoreError::InvalidTransition {
            from: Status::Created,
            to: Status::Running
        }
    ));
    assert_eq!(record.status, Status::Created);
}

#[test]
fn serde_round_trip_preserves_every_field() {
    let mut record = sample_record();
    record.engine_version = "1.2.3".to_owned();
    record.status = Status::Running;
    record.pid = Some(11);
    record.engine_pid = Some(12);
    record.engine_pgid = Some(13);
    record.exit_code = Some(0);
    record.started_at = Some(at(5));
    record.finished_at = Some(at(6));
    record.disk_path = Some(PathBuf::from("/tmp/dosker/disk.img"));
    record.dirty = true;
    record.image_digest = "sha256:0123456789abcdef".to_owned();
    record.booter = true;
    record.display = DisplayMode::Window;
    record.vnc_port = Some(5900);
    record.cmd = Some("GAME.EXE".to_owned());
    record.engine_command = Some("qemu -machine pc".to_owned());
    record.net = "none".to_owned();
    record.ports = vec![PortMapping {
        host_address: Ipv4Addr::LOCALHOST,
        host_port: 8080,
        guest_port: 80,
        protocol: PortProtocol::Tcp,
    }];
    record.volumes = vec![VolumeMount {
        drive: "D:".to_owned(),
        kind: VolumeKind::Named,
        name: Some("data".to_owned()),
        host_path: None,
        read_only: true,
    }];

    let json = serde_json::to_string(&record).unwrap();
    let restored: ContainerRecord = serde_json::from_str(&json).unwrap();

    assert_eq!(restored, record);
}

#[test]
fn an_existing_state_file_loads_with_empty_ports_and_volumes() {
    let record = sample_record();
    let mut value = serde_json::to_value(&record).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("ports");
    object.remove("volumes");
    object.remove("net");

    let restored: ContainerRecord = serde_json::from_value(value).unwrap();

    assert!(restored.ports.is_empty());
    assert!(restored.volumes.is_empty());
    assert_eq!(restored.net, "ne2000");
}

#[test]
fn unknown_fields_are_ignored() {
    let record = sample_record();
    let mut value = serde_json::to_value(&record).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("a_future_field".to_owned(), serde_json::json!(true));

    let restored: ContainerRecord = serde_json::from_value(value).unwrap();

    assert_eq!(restored, record);
}
