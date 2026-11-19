use std::net::Ipv4Addr;
use std::path::Path;

use chrono::Utc;
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_image::PortProtocol;
use dosker_platform::endpoint::control_endpoint;
use dosker_runtime::{ContainerRecord, PortConflict, PortMapping, StateStore, find_conflict};

fn store() -> (tempfile::TempDir, StateStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::new(dir.path().to_path_buf());
    (dir, store)
}

fn mapping(host_address: Ipv4Addr, host_port: u16) -> PortMapping {
    PortMapping {
        host_address,
        host_port,
        guest_port: 80,
        protocol: PortProtocol::Tcp,
    }
}

fn seed(
    store: &StateStore,
    root: &Path,
    id: &str,
    name: &str,
    status: Status,
    ports: Vec<PortMapping>,
) -> ContainerId {
    let id = ContainerId::parse(id).unwrap();
    let name = ContainerName::parse(name).unwrap();
    let endpoint = control_endpoint(root, &id);
    let mut record = ContainerRecord::new(
        id.clone(),
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        Utc::now(),
    )
    .with_network(String::from("ne2000"), ports);
    record.status = status;
    store.create(&record).unwrap();
    id
}

fn requester() -> ContainerId {
    ContainerId::parse("0000000000ff").unwrap()
}

#[test]
fn a_free_host_port_has_no_conflict() {
    let (dir, store) = store();
    seed(
        &store,
        dir.path(),
        "000000000001",
        "first",
        Status::Running,
        vec![mapping(Ipv4Addr::LOCALHOST, 9000)],
    );

    let conflict =
        find_conflict(&store, &requester(), &[mapping(Ipv4Addr::LOCALHOST, 9001)]).unwrap();

    assert_eq!(conflict, None);
}

#[test]
fn a_taken_host_port_names_the_holder() {
    let (dir, store) = store();
    seed(
        &store,
        dir.path(),
        "000000000001",
        "first",
        Status::Running,
        vec![mapping(Ipv4Addr::LOCALHOST, 9000)],
    );

    let conflict =
        find_conflict(&store, &requester(), &[mapping(Ipv4Addr::LOCALHOST, 9000)]).unwrap();

    assert_eq!(
        conflict,
        Some(PortConflict::Taken {
            host_address: Ipv4Addr::LOCALHOST,
            host_port: 9000,
            holder: String::from("first"),
        })
    );
    assert_eq!(
        conflict.unwrap().to_string(),
        "port 127.0.0.1:9000/tcp is already published by container first"
    );
}

#[test]
fn a_stopped_container_does_not_hold_a_port() {
    let (dir, store) = store();
    seed(
        &store,
        dir.path(),
        "000000000001",
        "first",
        Status::Stopped,
        vec![mapping(Ipv4Addr::LOCALHOST, 9000)],
    );

    let conflict =
        find_conflict(&store, &requester(), &[mapping(Ipv4Addr::LOCALHOST, 9000)]).unwrap();

    assert_eq!(conflict, None);
}

#[test]
fn the_same_port_on_another_address_does_not_conflict() {
    let (dir, store) = store();
    seed(
        &store,
        dir.path(),
        "000000000001",
        "first",
        Status::Running,
        vec![mapping(Ipv4Addr::LOCALHOST, 9000)],
    );

    let conflict = find_conflict(
        &store,
        &requester(),
        &[mapping(Ipv4Addr::UNSPECIFIED, 9000)],
    )
    .unwrap();

    assert_eq!(conflict, None);
}

#[test]
fn a_container_does_not_conflict_with_itself() {
    let (dir, store) = store();
    let id = seed(
        &store,
        dir.path(),
        "000000000001",
        "first",
        Status::Running,
        vec![mapping(Ipv4Addr::LOCALHOST, 9000)],
    );

    let conflict = find_conflict(&store, &id, &[mapping(Ipv4Addr::LOCALHOST, 9000)]).unwrap();

    assert_eq!(conflict, None);
}
