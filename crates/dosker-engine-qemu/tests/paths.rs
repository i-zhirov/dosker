#![cfg(unix)]

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use dosker_core::ContainerId;
use dosker_engine::EngineSpec;
use dosker_engine_qemu::paths::QemuPaths;
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

fn short_root() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("dq")
        .tempdir_in("/tmp")
        .expect("create short temp directory")
}

fn long_root(dir: &tempfile::TempDir) -> PathBuf {
    let root = dir.path().join("r".repeat(200));
    std::fs::create_dir_all(&root).expect("create long root");
    root
}

#[test]
fn paths_use_the_container_directory() {
    let dir = short_root();
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = spec_for(dir.path(), &id);

    let paths = QemuPaths::for_spec(&spec);
    let container = dir.path().join("containers").join(id.as_str());

    assert_eq!(paths.agent_serial, container.join("agent.sock"));
    assert_eq!(paths.qmp, container.join("qmp.sock"));
    assert_eq!(paths.monitor, container.join("monitor.sock"));
    assert_eq!(paths.gdb, container.join("gdb.sock"));
}

#[test]
fn paths_fall_back_for_a_long_root() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let root = long_root(&dir);
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let spec = spec_for(&root, &id);

    let paths = QemuPaths::for_spec(&spec);

    assert_eq!(
        paths.agent_serial,
        short_socket_dir().join(format!("{}-agent.sock", id.as_str()))
    );
    assert_eq!(
        paths.qmp,
        short_socket_dir().join(format!("{}-qmp.sock", id.as_str()))
    );
    assert_eq!(
        paths.monitor,
        short_socket_dir().join(format!("{}-monitor.sock", id.as_str()))
    );
    assert_eq!(
        paths.gdb,
        short_socket_dir().join(format!("{}-gdb.sock", id.as_str()))
    );
}

#[test]
fn paths_are_unique_per_container() {
    let dir = short_root();
    let first = ContainerId::parse("0123456789ab").expect("parse container id");
    let second = ContainerId::parse("0123456789cd").expect("parse container id");

    let first_paths = QemuPaths::for_spec(&spec_for(dir.path(), &first));
    let second_paths = QemuPaths::for_spec(&spec_for(dir.path(), &second));

    let unique: HashSet<PathBuf> = [
        first_paths.agent_serial,
        first_paths.qmp,
        first_paths.monitor,
        first_paths.gdb,
        second_paths.agent_serial,
        second_paths.qmp,
        second_paths.monitor,
        second_paths.gdb,
    ]
    .into_iter()
    .collect();

    assert_eq!(unique.len(), 8);
}

#[test]
fn remove_stale_removes_existing_sockets() {
    let dir = short_root();
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let paths = QemuPaths::for_spec(&spec_for(dir.path(), &id));
    let container = dir.path().join("containers").join(id.as_str());
    std::fs::create_dir_all(&container).expect("create container directory");

    for path in [&paths.agent_serial, &paths.qmp, &paths.monitor, &paths.gdb] {
        std::fs::write(path, b"stale").expect("create stale socket file");
        assert_eq!(
            path,
            &engine_socket_path(dir.path(), &id, socket_name(path))
        );
    }

    paths.remove_stale().expect("remove stale sockets");

    assert!(!paths.agent_serial.exists());
    assert!(!paths.qmp.exists());
    assert!(!paths.monitor.exists());
    assert!(!paths.gdb.exists());

    paths.remove_stale().expect("remove missing sockets");
}

fn socket_name(path: &Path) -> &'static str {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("agent.sock") => "agent",
        Some("qmp.sock") => "qmp",
        Some("monitor.sock") => "monitor",
        Some("gdb.sock") => "gdb",
        other => panic!("unexpected socket path: {other:?}"),
    }
}
