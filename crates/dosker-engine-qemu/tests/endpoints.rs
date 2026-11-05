use std::collections::{BTreeMap, HashSet};
#[cfg(windows)]
use std::net::Ipv4Addr;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

use dosker_core::ContainerId;
use dosker_engine::{EngineEndpointKind, EngineSpec};
use dosker_engine_qemu::QemuEndpoints;
#[cfg(unix)]
use dosker_platform::{engine_socket_path, short_socket_dir};

fn spec_for(root: &Path, id: &ContainerId) -> EngineSpec {
    EngineSpec {
        root: root.to_path_buf(),
        container_id: id.clone(),
        workdir: "C:\\DOSKER".to_owned(),
        env: BTreeMap::new(),
        cmd: None,
        expects_agent: true,
        log_dir: root.join("containers").join(id.as_str()),
        disk_path: root.join("containers").join(id.as_str()).join("disk.img"),
        booter: false,
        display: Default::default(),
        vnc_port: None,
    }
}

#[cfg(unix)]
fn short_root() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("dq")
        .tempdir_in("/tmp")
        .expect("create short temp directory")
}

#[cfg(unix)]
fn long_root(dir: &tempfile::TempDir) -> PathBuf {
    let root = dir.path().join("r".repeat(200));
    std::fs::create_dir_all(&root).expect("create long root");
    root
}

#[cfg(unix)]
fn unix_path(kind: &EngineEndpointKind) -> &Path {
    kind.unix_path().expect("a Unix endpoint")
}

#[test]
#[cfg(unix)]
fn unix_endpoints_use_the_container_socket_directory() {
    let dir = short_root();
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = spec_for(dir.path(), &id);

    let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve endpoints");
    let container = dir.path().join("containers").join(id.as_str());
    let agent = endpoints.agent_serial.as_ref().expect("an agent endpoint");

    assert_eq!(unix_path(agent), container.join("agent.sock").as_path());
    assert_eq!(
        unix_path(&endpoints.qmp),
        container.join("qmp.sock").as_path()
    );
    assert_eq!(
        unix_path(&endpoints.monitor),
        container.join("monitor.sock").as_path()
    );
    assert_eq!(
        unix_path(&endpoints.gdb),
        container.join("gdb.sock").as_path()
    );
}

#[test]
#[cfg(unix)]
fn unix_endpoints_fall_back_for_a_long_root() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = spec_for(&root, &id);

    let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve endpoints");
    let agent = endpoints.agent_serial.as_ref().expect("an agent endpoint");

    assert_eq!(
        unix_path(agent),
        short_socket_dir()
            .join(format!("{}-agent.sock", id.as_str()))
            .as_path()
    );
    assert_eq!(
        unix_path(&endpoints.qmp),
        short_socket_dir()
            .join(format!("{}-qmp.sock", id.as_str()))
            .as_path()
    );
    assert_eq!(
        unix_path(&endpoints.monitor),
        short_socket_dir()
            .join(format!("{}-monitor.sock", id.as_str()))
            .as_path()
    );
    assert_eq!(
        unix_path(&endpoints.gdb),
        short_socket_dir()
            .join(format!("{}-gdb.sock", id.as_str()))
            .as_path()
    );
}

#[test]
#[cfg(windows)]
fn windows_endpoints_are_distinct_loopback_tcp_ports() {
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = spec_for(Path::new(r"C:\dosker"), &id);

    let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve endpoints");
    let agent = endpoints.agent_serial.as_ref().expect("an agent endpoint");
    let kinds = [agent, &endpoints.qmp, &endpoints.monitor, &endpoints.gdb];
    let mut ports = Vec::with_capacity(kinds.len());

    for kind in kinds {
        match kind {
            EngineEndpointKind::Tcp { host, port } => {
                assert_eq!(*host, Ipv4Addr::LOCALHOST);
                assert_ne!(*port, 0);
                ports.push(*port);
            }
            EngineEndpointKind::UnixSocket(path) => {
                panic!("Windows endpoint is not TCP: {}", path.display())
            }
        }
    }

    let unique: HashSet<u16> = ports.iter().copied().collect();
    assert_eq!(unique.len(), 4);
}

#[test]
#[cfg(unix)]
fn unix_endpoints_are_unique_per_container() {
    let dir = short_root();
    let first = ContainerId::parse("0123456789ab").expect("parse container id");
    let second = ContainerId::parse("0123456789cd").expect("parse container id");

    let first = QemuEndpoints::for_spec(&spec_for(dir.path(), &first)).expect("first endpoints");
    let second = QemuEndpoints::for_spec(&spec_for(dir.path(), &second)).expect("second endpoints");

    let first_agent = first.agent_serial.as_ref().expect("first agent");
    let second_agent = second.agent_serial.as_ref().expect("second agent");
    let unique: HashSet<PathBuf> = [
        unix_path(first_agent),
        unix_path(&first.qmp),
        unix_path(&first.monitor),
        unix_path(&first.gdb),
        unix_path(second_agent),
        unix_path(&second.qmp),
        unix_path(&second.monitor),
        unix_path(&second.gdb),
    ]
    .into_iter()
    .map(Path::to_path_buf)
    .collect();

    assert_eq!(unique.len(), 8);
}

#[test]
#[cfg(unix)]
fn remove_stale_removes_existing_unix_sockets() {
    let dir = short_root();
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = spec_for(dir.path(), &id);
    let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve endpoints");
    let container = dir.path().join("containers").join(id.as_str());
    std::fs::create_dir_all(&container).expect("create container directory");

    let agent = endpoints.agent_serial.as_ref().expect("an agent endpoint");
    for path in [
        unix_path(agent),
        unix_path(&endpoints.qmp),
        unix_path(&endpoints.monitor),
        unix_path(&endpoints.gdb),
    ] {
        std::fs::write(path, b"stale").expect("create stale socket file");
        assert_eq!(path, engine_socket_path(dir.path(), &id, socket_name(path)));
    }

    endpoints.remove_stale().expect("remove stale sockets");

    assert!(!unix_path(agent).exists());
    assert!(!unix_path(&endpoints.qmp).exists());
    assert!(!unix_path(&endpoints.monitor).exists());
    assert!(!unix_path(&endpoints.gdb).exists());

    endpoints
        .remove_stale()
        .expect("remove missing sockets again");
}

#[cfg(unix)]
fn socket_name(path: &Path) -> &'static str {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("agent.sock") => "agent",
        Some("qmp.sock") => "qmp",
        Some("monitor.sock") => "monitor",
        Some("gdb.sock") => "gdb",
        other => panic!("unexpected socket path: {other:?}"),
    }
}

#[test]
fn endpoints_omit_the_agent_for_a_booter() {
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let mut spec = spec_for(Path::new("/tmp/dosker"), &id);
    spec.booter = true;

    let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve endpoints");

    assert!(endpoints.agent_serial.is_none());
    assert_eq!(
        endpoints.engine_endpoints().len(),
        3,
        "a booter exposes qmp, monitor, and gdb"
    );
}
