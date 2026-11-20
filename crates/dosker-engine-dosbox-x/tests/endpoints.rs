use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::Path;

use dosker_core::ContainerId;
use dosker_engine::{EngineEndpointKind, EngineSpec};
use dosker_engine_dosbox_x::DosboxXEndpoints;

fn sample_spec(root: &Path, id: &ContainerId) -> EngineSpec {
    EngineSpec {
        root: root.to_path_buf(),
        container_id: id.clone(),
        workdir: "C:\\DOSKER".to_owned(),
        env: BTreeMap::new(),
        cmd: None,
        expects_agent: true,
        log_dir: root.join("containers").join(id.as_str()),
        disk_path: root.join("containers").join(id.as_str()).join("disk.img"),
        disk_interface: "ide".to_owned(),
        serial_ports: 2,
        control_port: 2,
        booter: false,
        display: Default::default(),
        vnc_port: None,
        allow_reboot: false,
        net: "none".to_owned(),
        ports: Vec::new(),
        volumes: Vec::new(),
        engine_config: BTreeMap::new(),
    }
}

fn tcp_port(kind: &EngineEndpointKind) -> u16 {
    match kind {
        EngineEndpointKind::Tcp { port, .. } => *port,
        EngineEndpointKind::UnixSocket(path) => {
            panic!("the endpoint is not TCP: {}", path.display())
        }
    }
}

#[test]
fn uses_the_recorded_agent_endpoint_kind() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = sample_spec(dir.path(), &id);

    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let agent = endpoints.agent_serial.as_ref().expect("an agent endpoint");

    match agent {
        EngineEndpointKind::Tcp { host, port } => {
            assert_eq!(*host, Ipv4Addr::LOCALHOST);
            assert_ne!(*port, 0);
        }
        EngineEndpointKind::UnixSocket(path) => {
            panic!("the agent endpoint is not TCP: {}", path.display())
        }
    }
}

#[test]
fn allocates_distinct_tcp_ports_when_the_spike_records_tcp() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = sample_spec(dir.path(), &id);

    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let agent = tcp_port(endpoints.agent_serial.as_ref().expect("an agent endpoint"));
    let debug = tcp_port(endpoints.debug.as_ref().expect("a debug endpoint"));

    assert_ne!(agent, debug);
}

#[test]
#[cfg(unix)]
fn removes_stale_socket_files() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let path = dir.path().join("agent.sock");
    std::fs::write(&path, b"stale").expect("create stale socket file");
    let endpoints = DosboxXEndpoints {
        agent_serial: Some(EngineEndpointKind::UnixSocket(path.clone())),
        debug: None,
    };

    endpoints.remove_stale().expect("remove stale socket");

    assert!(!path.exists());
    endpoints
        .remove_stale()
        .expect("remove missing socket again");
}

#[test]
fn a_booter_has_no_agent_endpoint() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let mut spec = sample_spec(dir.path(), &id);
    spec.booter = true;

    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");

    assert!(endpoints.agent_serial.is_none());
}
