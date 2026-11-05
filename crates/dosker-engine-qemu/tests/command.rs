use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use dosker_core::ContainerId;
#[cfg(windows)]
use dosker_engine::EngineError;
use dosker_engine::{
    Capability, CapabilitySet, DisplayMode, EngineAdapter, EngineCommand, EngineEndpointKind,
    EngineSpec,
};
use dosker_engine_qemu::command;
use dosker_engine_qemu::{QemuAdapter, QemuBinary, QemuEndpoints, QemuVersion};
use dosker_platform::process::{ProcessSupervisor, system};

fn sample_spec() -> EngineSpec {
    let root = PathBuf::from("/tmp/dosker-qemu-command");
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    EngineSpec {
        root: root.clone(),
        container_id: id.clone(),
        workdir: "C:\\DOSKER".to_owned(),
        env: BTreeMap::new(),
        cmd: None,
        expects_agent: true,
        log_dir: root.join("containers").join(id.as_str()),
        disk_path: root.join("containers").join(id.as_str()).join("disk.img"),
        booter: false,
        display: DisplayMode::None,
        vnc_port: None,
    }
}

fn sample_binary() -> QemuBinary {
    QemuBinary {
        path: PathBuf::from("/usr/bin/qemu-system-i386"),
        version: QemuVersion::new(8, 2, 0),
    }
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|argument| argument == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
    args.windows(2)
        .any(|pair| pair[0].as_str() == flag && pair[1].as_str() == value)
}

fn build_command(spec: &EngineSpec) -> EngineCommand {
    command::build(spec, &sample_binary()).expect("build command")
}

fn build(spec: &EngineSpec) -> Vec<String> {
    build_command(spec).args
}

#[cfg(unix)]
fn unix_path(kind: &EngineEndpointKind) -> &std::path::Path {
    kind.unix_path().expect("a Unix endpoint")
}

#[cfg(windows)]
fn tcp_option_port(value: &str) -> u16 {
    value
        .strip_prefix("tcp:127.0.0.1:")
        .expect("a loopback TCP option")
        .split(',')
        .next()
        .and_then(|port| port.parse().ok())
        .expect("a TCP port")
}

#[cfg(windows)]
fn gdb_option_port(value: &str) -> u16 {
    value
        .split(",port=")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .and_then(|port| port.parse().ok())
        .expect("a gdb TCP port")
}

#[test]
fn headless_command_uses_display_none() {
    let args = build(&sample_spec());

    assert!(has_pair(&args, "-display", "none"));
    assert!(!args.iter().any(|argument| argument == "-vnc"));
}

#[test]
fn command_uses_the_private_disk_and_two_serial_ports() {
    let spec = sample_spec();
    let command = build_command(&spec);
    let drive = format!(
        "file={},format=raw,if=ide,index=0",
        spec.disk_path.display()
    );

    assert!(has_pair(&command.args, "-drive", &drive));
    assert_eq!(
        command
            .args
            .iter()
            .filter(|argument| argument.as_str() == "-serial")
            .count(),
        2
    );

    #[cfg(unix)]
    {
        let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve endpoints");
        let agent = endpoints.agent_serial.as_ref().expect("an agent endpoint");
        let expected = format!("unix:{},server=on,wait=off", unix_path(agent).display());
        assert!(has_pair(&command.args, "-serial", &expected));
    }
}

#[test]
fn command_uses_ide_for_the_disk_and_booter() {
    for booter in [false, true] {
        let mut spec = sample_spec();
        spec.booter = booter;
        let args = build(&spec);
        let joined = args.join(" ");

        assert!(
            joined.contains("if=ide"),
            "missing if=ide for booter={booter}"
        );
        assert!(
            !joined.contains("if=floppy"),
            "unexpected if=floppy for booter={booter}"
        );
    }
}

#[test]
fn command_uses_the_conformance_machine() {
    let args = build(&sample_spec());
    let joined = args.join(" ");

    assert!(has_pair(&args, "-machine", "pc"));
    assert!(has_pair(&args, "-cpu", "486"));
    assert!(has_pair(&args, "-m", "16"));
    assert!(!joined.contains("-soundhw"));
    assert!(!joined.contains("sb16"));
    assert!(has_pair(&args, "-nic", "none"));
}

#[test]
fn command_carries_one_endpoint_for_each_engine_channel() {
    let command = build_command(&sample_spec());
    let names: Vec<&str> = command
        .endpoints
        .iter()
        .map(|endpoint| endpoint.name.as_str())
        .collect();

    assert_eq!(names, ["serial_agent", "qmp", "monitor", "gdb"]);
}

#[test]
#[cfg(unix)]
fn command_uses_unix_sockets_on_unix() {
    let spec = sample_spec();
    let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve endpoints");
    let agent = endpoints.agent_serial.as_ref().expect("an agent endpoint");
    let args = build(&spec);

    assert!(has_pair(
        &args,
        "-serial",
        &format!("unix:{},server=on,wait=off", unix_path(agent).display())
    ));
    assert!(has_pair(
        &args,
        "-qmp",
        &format!(
            "unix:{},server=on,wait=off",
            unix_path(&endpoints.qmp).display()
        )
    ));
    assert!(has_pair(
        &args,
        "-monitor",
        &format!(
            "unix:{},server=on,wait=off",
            unix_path(&endpoints.monitor).display()
        )
    ));
    assert!(has_pair(
        &args,
        "-chardev",
        &format!(
            "socket,id=gdb0,path={},server=on,wait=off",
            unix_path(&endpoints.gdb).display()
        )
    ));
}

#[test]
#[cfg(windows)]
fn command_uses_loopback_tcp_on_windows() {
    let spec = sample_spec();
    let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve endpoints");
    let agent = endpoints.agent_serial.as_ref().expect("an agent endpoint");
    let args = build(&spec);

    let serial = match agent {
        EngineEndpointKind::Tcp { host, port } => format!("tcp:{host}:{port},server=on,wait=off"),
        EngineEndpointKind::UnixSocket(path) => {
            panic!("Windows agent endpoint is not TCP: {}", path.display())
        }
    };
    let qmp = match &endpoints.qmp {
        EngineEndpointKind::Tcp { host, port } => format!("tcp:{host}:{port},server=on,wait=off"),
        EngineEndpointKind::UnixSocket(path) => {
            panic!("Windows qmp endpoint is not TCP: {}", path.display())
        }
    };
    let monitor = match &endpoints.monitor {
        EngineEndpointKind::Tcp { host, port } => format!("tcp:{host}:{port},server=on,wait=off"),
        EngineEndpointKind::UnixSocket(path) => {
            panic!("Windows monitor endpoint is not TCP: {}", path.display())
        }
    };
    let gdb = match &endpoints.gdb {
        EngineEndpointKind::Tcp { host, port } => {
            format!("socket,id=gdb0,host={host},port={port},server=on,wait=off")
        }
        EngineEndpointKind::UnixSocket(path) => {
            panic!("Windows gdb endpoint is not TCP: {}", path.display())
        }
    };

    assert!(has_pair(&args, "-serial", &serial));
    assert!(has_pair(&args, "-qmp", &qmp));
    assert!(has_pair(&args, "-monitor", &monitor));
    assert!(has_pair(&args, "-chardev", &gdb));
}

#[test]
#[cfg(windows)]
fn command_uses_distinct_loopback_ports_on_windows() {
    use std::collections::HashSet;

    let args = build(&sample_spec());
    let serial_ports: Vec<u16> = args
        .windows(2)
        .filter(|pair| pair[0].as_str() == "-serial" && pair[1].starts_with("tcp:127.0.0.1:"))
        .map(|pair| tcp_option_port(&pair[1]))
        .collect();
    assert_eq!(serial_ports.len(), 1);

    let qmp = tcp_option_port(value_after(&args, "-qmp").expect("the qmp option"));
    let monitor = tcp_option_port(value_after(&args, "-monitor").expect("the monitor option"));
    let gdb = gdb_option_port(value_after(&args, "-chardev").expect("the gdb chardev"));

    let unique: HashSet<u16> = [serial_ports[0], qmp, monitor, gdb].into_iter().collect();
    assert_eq!(unique.len(), 4);
}

#[test]
fn booter_command_has_no_agent_socket() {
    let mut spec = sample_spec();
    spec.booter = true;
    let command = build_command(&spec);

    assert!(
        !command
            .args
            .iter()
            .any(|argument| { argument.contains("agent") || argument.contains("serial_agent") })
    );
    assert_eq!(
        command
            .args
            .windows(2)
            .filter(|pair| pair[0].as_str() == "-serial" && pair[1].as_str() == "none")
            .count(),
        2
    );
    let names: Vec<&str> = command
        .endpoints
        .iter()
        .map(|endpoint| endpoint.name.as_str())
        .collect();
    assert_eq!(names, ["qmp", "monitor", "gdb"]);
}

#[test]
fn command_translates_the_vnc_port_to_a_display_number() {
    let mut spec = sample_spec();
    spec.display = DisplayMode::Vnc;
    spec.vnc_port = Some(5905);
    let args = build(&spec);

    assert!(has_pair(&args, "-display", "none"));
    assert!(has_pair(&args, "-vnc", "127.0.0.1:5"));
}

#[test]
fn command_rejects_a_vnc_port_below_5900() {
    let mut spec = sample_spec();
    spec.display = DisplayMode::Vnc;
    spec.vnc_port = Some(5899);

    assert!(command::build(&spec, &sample_binary()).is_err());
}

#[test]
fn command_accepts_the_vnc_port_upper_bound() {
    let mut spec = sample_spec();
    spec.display = DisplayMode::Vnc;
    spec.vnc_port = Some(65535);
    let args = build(&spec);

    assert!(has_pair(&args, "-vnc", "127.0.0.1:59635"));
}

#[test]
fn command_uses_a_free_vnc_port_when_vnc_port_is_absent() {
    let mut spec = sample_spec();
    spec.display = DisplayMode::Vnc;
    spec.vnc_port = None;
    let args = build(&spec);

    let value = value_after(&args, "-vnc").expect("vnc option");
    let display: u16 = value
        .strip_prefix("127.0.0.1:")
        .expect("vnc address")
        .parse()
        .expect("vnc display number");
    let port = 5900 + u32::from(display);

    assert!(
        (5900..=65535).contains(&port),
        "vnc port out of range: {port}"
    );
}

#[test]
#[cfg(windows)]
fn vnc_port_does_not_reuse_an_engine_port() {
    use std::collections::HashSet;

    let mut spec = sample_spec();
    spec.display = DisplayMode::Vnc;
    spec.vnc_port = None;
    let command = build_command(&spec);

    let engine_ports: HashSet<u16> = command
        .endpoints
        .iter()
        .filter_map(|endpoint| endpoint.kind.tcp_addr().map(|address| address.port()))
        .collect();
    assert_eq!(engine_ports.len(), 4);

    let value = value_after(&command.args, "-vnc").expect("vnc option");
    let display: u32 = value
        .strip_prefix("127.0.0.1:")
        .expect("vnc address")
        .parse()
        .expect("vnc display number");
    let port = u16::try_from(5900 + display).expect("vnc port in range");

    assert!(!engine_ports.contains(&port));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn command_uses_the_window_display_on_each_host() {
    let mut spec = sample_spec();
    spec.display = DisplayMode::Window;
    let args = build(&spec);
    let expected = if cfg!(target_os = "macos") {
        "cocoa"
    } else {
        "gtk"
    };

    assert!(has_pair(&args, "-display", expected));
}

#[test]
#[cfg(windows)]
fn windows_rejects_the_window_display() {
    let mut spec = sample_spec();
    spec.display = DisplayMode::Window;

    match command::build(&spec, &sample_binary()) {
        Err(EngineError::Unsupported("window display on Windows")) => {}
        other => panic!("expected the Windows window display error, got {other:?}"),
    }
}

#[test]
fn command_disables_networking() {
    let args = build(&sample_spec());

    assert!(has_pair(&args, "-nic", "none"));
}

#[test]
fn capabilities_are_the_proven_set() {
    let processes: Arc<dyn ProcessSupervisor> = Arc::new(system());
    let adapter = QemuAdapter::new(sample_binary(), processes);
    let expected: CapabilitySet = [
        Capability::Boot,
        Capability::Serial,
        Capability::Stop,
        Capability::Headless,
        Capability::DebugNative,
        Capability::GdbStub,
        Capability::Screenshot,
        Capability::Vnc,
    ]
    .into_iter()
    .collect();

    assert_eq!(adapter.name(), "qemu");
    assert_eq!(adapter.version(), "8.2.0");
    assert_eq!(adapter.capabilities(), expected);
}

#[test]
fn capabilities_never_include_nat() {
    let processes: Arc<dyn ProcessSupervisor> = Arc::new(system());
    let adapter = QemuAdapter::new(sample_binary(), processes);

    assert!(!adapter.capabilities().contains(Capability::Nat));
}
