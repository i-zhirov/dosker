#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use dosker_core::ContainerId;
use dosker_engine::{DisplayMode, EngineAdapter, EngineSpec};
use dosker_engine_bochs::{BochsAdapter, BochsBinary, Debugger};
use dosker_platform::process::system;
use dosker_test_support::fixtures::{bochs_gdbstub, bochs_image, integration_enabled};

const CONNECT_RETRY: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const BOCHS_VERSION: &str = "3.1";

/// Returns the gdbstub binary and prepared image when every gate is open.
///
/// A missing gate skips the test.
fn gates() -> Option<(PathBuf, PathBuf)> {
    if !integration_enabled() {
        return None;
    }

    Some((bochs_gdbstub().ok()?, bochs_image().ok()?))
}

fn booter_spec(root: &Path, image: &Path, port: u16) -> EngineSpec {
    let id = ContainerId::generate();
    let container = root.join("containers").join(id.as_str());
    std::fs::create_dir_all(&container).expect("create the container directory");
    let disk_path = container.join("disk.img");
    std::fs::copy(image, &disk_path).expect("copy the prepared image");

    let mut engine_config = BTreeMap::new();
    engine_config.insert(
        "bochs".to_owned(),
        vec![format!(
            "gdbstub: enabled=1, port={port}, text_base=0, data_base=0, bss_base=0"
        )],
    );

    EngineSpec {
        root: root.to_path_buf(),
        container_id: id,
        workdir: "C:\\".to_owned(),
        env: BTreeMap::new(),
        cmd: None,
        expects_agent: false,
        log_dir: container,
        disk_path,
        disk_interface: "ide".to_owned(),
        serial_ports: 2,
        control_port: 2,
        booter: true,
        display: DisplayMode::None,
        vnc_port: None,
        allow_reboot: false,
        net: "none".to_owned(),
        ports: Vec::new(),
        volumes: Vec::new(),
        engine_config,
    }
}

fn connect(port: u16) -> TcpStream {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match TcpStream::connect((Ipv4Addr::LOCALHOST, port)) {
            Ok(stream) => return stream,
            Err(error) if Instant::now() >= deadline => {
                panic!("cannot connect to the gdbstub on port {port}: {error}");
            }
            Err(_) => thread::sleep(CONNECT_RETRY),
        }
    }
}

#[test]
fn gdbstub_build_has_no_internal_debugger() {
    let Some((gdbstub, _image)) = gates() else {
        return;
    };

    let binary = BochsBinary::from_path(&gdbstub).expect("probe the gdbstub build");

    assert_eq!(binary.debugger, Debugger::None);
    assert_eq!(binary.version, BOCHS_VERSION);
}

#[test]
fn gdbstub_build_accepts_a_gdb_connection() {
    let Some((gdbstub, image)) = gates() else {
        return;
    };

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind a loopback port");
    let port = listener
        .local_addr()
        .expect("read the loopback port")
        .port();
    drop(listener);

    let dir = tempfile::tempdir().expect("create a temporary directory");
    let spec = booter_spec(dir.path(), &image, port);

    let binary = BochsBinary::from_path(&gdbstub).expect("probe the gdbstub build");
    let mut adapter = BochsAdapter::new(binary, Arc::new(system()));
    let command = adapter.build_command(&spec).expect("build the command");
    let mut session = adapter
        .start_with_command(&spec, &command)
        .expect("start the gdbstub session");

    let mut stream = connect(port);
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .expect("set the read timeout");
    stream
        .write_all(b"$qSupported#37")
        .expect("send the gdb query");

    let mut buffer = [0u8; 256];
    let read = stream.read(&mut buffer).expect("read the gdb reply");
    let reply = String::from_utf8_lossy(&buffer[..read]);

    assert!(
        reply.contains('$') && reply.contains('#'),
        "unexpected gdb reply: {reply:?}"
    );

    session.kill().expect("kill the session");
}
