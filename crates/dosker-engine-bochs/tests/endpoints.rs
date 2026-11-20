use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::Path;

use dosker_core::ContainerId;
use dosker_engine::{DisplayMode, EngineCommand, EngineEndpoint, EngineEndpointKind, EngineSpec};
use dosker_engine_bochs::BochsEndpoints;

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
        display: DisplayMode::None,
        vnc_port: None,
        allow_reboot: false,
        net: "none".to_owned(),
        ports: Vec::new(),
        volumes: Vec::new(),
        engine_config: BTreeMap::new(),
    }
}

#[test]
fn uses_a_loopback_tcp_agent_endpoint() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = sample_spec(dir.path(), &id);

    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
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
fn a_booter_has_no_agent_endpoint() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let mut spec = sample_spec(dir.path(), &id);
    spec.booter = true;

    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    assert!(endpoints.agent_serial.is_none());
}

#[test]
fn from_command_reads_the_agent_endpoint() {
    let endpoint = EngineEndpoint {
        name: "serial_agent".to_owned(),
        kind: EngineEndpointKind::Tcp {
            host: Ipv4Addr::LOCALHOST,
            port: 45_000,
        },
    };
    let command = EngineCommand {
        program: "bochs".into(),
        args: Vec::new(),
        env: Vec::new(),
        endpoints: vec![endpoint.clone()],
    };

    let endpoints = BochsEndpoints::from_command(&command).expect("read endpoints");

    assert_eq!(endpoints.agent_serial, Some(endpoint.kind.clone()));
    assert_eq!(endpoints.engine_endpoints(), vec![endpoint]);
}

#[test]
fn remove_stale_removes_nothing() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = sample_spec(dir.path(), &id);
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    endpoints.remove_stale().expect("remove stale endpoints");
}
