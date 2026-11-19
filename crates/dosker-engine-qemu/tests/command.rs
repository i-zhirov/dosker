use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use dosker_core::ContainerId;
use dosker_engine::{
    Capability, CapabilitySet, DisplayMode, EngineAdapter, EngineCommand, EngineEndpointKind,
    EngineError, EnginePort, EngineSpec, EngineVolume, EngineVolumeKind,
};
#[cfg(unix)]
use dosker_engine_qemu::QemuEndpoints;
use dosker_engine_qemu::command;
use dosker_engine_qemu::{QemuAdapter, QemuBinary, QemuVersion};
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
        disk_interface: "ide".to_owned(),
        serial_ports: 2,
        control_port: 2,
        booter: false,
        display: DisplayMode::None,
        vnc_port: None,
        allow_reboot: false,
        net: "ne2000".to_owned(),
        ports: Vec::new(),
        volumes: Vec::new(),
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
fn command_uses_ide_for_a_partitioned_disk() {
    let mut spec = sample_spec();
    spec.disk_interface = "ide".to_owned();
    let args = build(&spec);
    assert!(args.join(" ").contains("if=ide"));
    assert!(!args.join(" ").contains("if=floppy"));
}

#[test]
fn command_uses_floppy_for_a_superfloppy() {
    let mut spec = sample_spec();
    spec.disk_interface = "floppy".to_owned();
    let args = build(&spec);
    assert!(args.join(" ").contains("if=floppy"));
    assert!(!args.join(" ").contains("if=ide"));
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
    assert!(has_pair(&args, "-nic", "user,model=ne2k_isa"));
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
    let command = build_command(&sample_spec());
    let endpoint = |name: &str| {
        command
            .endpoints
            .iter()
            .find(|endpoint| endpoint.name == name)
            .expect("the command carries the endpoint")
    };

    let serial = match &endpoint("serial_agent").kind {
        EngineEndpointKind::Tcp { host, port } => format!("tcp:{host}:{port},server=on,wait=off"),
        EngineEndpointKind::UnixSocket(path) => {
            panic!("Windows agent endpoint is not TCP: {}", path.display())
        }
    };
    let qmp = match &endpoint("qmp").kind {
        EngineEndpointKind::Tcp { host, port } => format!("tcp:{host}:{port},server=on,wait=off"),
        EngineEndpointKind::UnixSocket(path) => {
            panic!("Windows qmp endpoint is not TCP: {}", path.display())
        }
    };
    let monitor = match &endpoint("monitor").kind {
        EngineEndpointKind::Tcp { host, port } => format!("tcp:{host}:{port},server=on,wait=off"),
        EngineEndpointKind::UnixSocket(path) => {
            panic!("Windows monitor endpoint is not TCP: {}", path.display())
        }
    };
    let gdb = match &endpoint("gdb").kind {
        EngineEndpointKind::Tcp { host, port } => {
            format!("socket,id=gdb0,host={host},port={port},server=on,wait=off")
        }
        EngineEndpointKind::UnixSocket(path) => {
            panic!("Windows gdb endpoint is not TCP: {}", path.display())
        }
    };

    assert!(has_pair(&command.args, "-serial", &serial));
    assert!(has_pair(&command.args, "-qmp", &qmp));
    assert!(has_pair(&command.args, "-monitor", &monitor));
    assert!(has_pair(&command.args, "-chardev", &gdb));
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

fn nic_value(args: &[String]) -> &str {
    value_after(args, "-nic").expect("a nic option")
}

#[test]
fn command_uses_no_nic_when_the_machine_disables_the_network() {
    let mut spec = sample_spec();
    spec.net = "none".to_owned();
    let args = build(&spec);

    assert!(has_pair(&args, "-nic", "none"));
}

#[test]
fn command_publishes_a_host_port_to_a_guest_port() {
    let mut spec = sample_spec();
    spec.ports = vec![EnginePort {
        host_address: Ipv4Addr::LOCALHOST,
        host_port: 8080,
        guest_port: 80,
    }];
    let args = build(&spec);

    assert!(nic_value(&args).contains("hostfwd=tcp:127.0.0.1:8080-:80"));
}

#[test]
fn command_publishes_ports_in_host_port_order() {
    let mut spec = sample_spec();
    spec.ports = vec![
        EnginePort {
            host_address: Ipv4Addr::LOCALHOST,
            host_port: 9000,
            guest_port: 90,
        },
        EnginePort {
            host_address: Ipv4Addr::LOCALHOST,
            host_port: 8000,
            guest_port: 80,
        },
        EnginePort {
            host_address: Ipv4Addr::LOCALHOST,
            host_port: 8080,
            guest_port: 8080,
        },
    ];
    let args = build(&spec);
    let forwards: Vec<&str> = nic_value(&args)
        .split(',')
        .filter(|part| part.starts_with("hostfwd="))
        .collect();

    assert_eq!(
        forwards,
        [
            "hostfwd=tcp:127.0.0.1:8000-:80",
            "hostfwd=tcp:127.0.0.1:8080-:8080",
            "hostfwd=tcp:127.0.0.1:9000-:90",
        ]
    );
}

#[test]
fn command_publishes_to_an_explicit_address() {
    let mut spec = sample_spec();
    spec.ports = vec![EnginePort {
        host_address: Ipv4Addr::new(0, 0, 0, 0),
        host_port: 8080,
        guest_port: 80,
    }];
    let args = build(&spec);

    assert!(nic_value(&args).contains("hostfwd=tcp:0.0.0.0:8080-:80"));
}

#[test]
fn command_rejects_an_unknown_network_device() {
    let mut spec = sample_spec();
    spec.net = "ne2k_pci".to_owned();

    match command::build(&spec, &sample_binary()) {
        Err(EngineError::Setup(_)) => {}
        other => panic!("expected a setup error for an unknown network device, got {other:?}"),
    }
}

fn volume(drive: &str, name: &str, read_only: bool) -> EngineVolume {
    EngineVolume {
        drive: drive.to_owned(),
        path: PathBuf::from(format!("/tmp/dosker-volumes/{name}.img")),
        read_only,
        kind: EngineVolumeKind::Image,
    }
}

fn host_dir(drive: &str, path: &str, read_only: bool) -> EngineVolume {
    EngineVolume {
        drive: drive.to_owned(),
        path: PathBuf::from(path),
        read_only,
        kind: EngineVolumeKind::HostDir,
    }
}

#[test]
fn command_attaches_a_partitioned_volume_as_an_ide_disk() {
    let mut spec = sample_spec();
    spec.volumes = vec![volume("D:", "data", false), volume("E:", "extra", false)];
    let args = build(&spec);

    let data = format!(
        "file={},format=raw,if=ide,index=1",
        spec.volumes[0].path.display()
    );
    let extra = format!(
        "file={},format=raw,if=ide,index=2",
        spec.volumes[1].path.display()
    );
    assert!(has_pair(&args, "-drive", &data));
    assert!(has_pair(&args, "-drive", &extra));
}

#[test]
fn command_attaches_a_superfloppy_volume_as_a_floppy() {
    let mut spec = sample_spec();
    spec.volumes = vec![volume("A:", "first", false), volume("B:", "second", false)];
    let args = build(&spec);

    let first = format!(
        "file={},format=raw,if=floppy,index=0",
        spec.volumes[0].path.display()
    );
    let second = format!(
        "file={},format=raw,if=floppy,index=1",
        spec.volumes[1].path.display()
    );
    assert!(has_pair(&args, "-drive", &first));
    assert!(has_pair(&args, "-drive", &second));
}

#[test]
fn command_marks_a_read_only_volume_read_only() {
    let mut spec = sample_spec();
    spec.volumes = vec![volume("D:", "readonly", true)];
    let args = build(&spec);

    let drive = format!(
        "file={},format=raw,if=ide,index=1,snapshot=on",
        spec.volumes[0].path.display()
    );
    assert!(has_pair(&args, "-drive", &drive));
}

#[test]
fn command_marks_a_read_only_floppy_volume_read_only() {
    let mut spec = sample_spec();
    spec.volumes = vec![volume("A:", "readonly", true)];
    let args = build(&spec);

    let drive = format!(
        "file={},format=raw,if=floppy,index=0,readonly=on",
        spec.volumes[0].path.display()
    );
    assert!(has_pair(&args, "-drive", &drive));
}

#[test]
fn command_attaches_a_host_directory_as_vvfat() {
    let mut spec = sample_spec();
    spec.volumes = vec![host_dir("D:", "/tmp/dosker-binds/host", false)];
    let args = build(&spec);

    let drive = String::from("file=fat:rw:/tmp/dosker-binds/host,format=raw,if=ide,index=1");
    assert!(has_pair(&args, "-drive", &drive));
    assert!(!drive.contains("readonly=on"));
    assert!(!drive.contains("snapshot=on"));
}

#[test]
fn command_rejects_a_read_only_host_directory() {
    let mut spec = sample_spec();
    spec.volumes = vec![host_dir("D:", "/tmp/dosker-binds/host", true)];

    match command::build(&spec, &sample_binary()) {
        Err(EngineError::Setup(_)) => {}
        other => panic!("expected a setup error for a read-only host directory, got {other:?}"),
    }
}

#[test]
fn command_rejects_a_host_directory_with_a_comma() {
    let mut spec = sample_spec();
    spec.volumes = vec![host_dir("D:", "/tmp/dosker-binds/a,b", false)];

    match command::build(&spec, &sample_binary()) {
        Err(EngineError::Setup(_)) => {}
        other => panic!("expected a setup error for a comma in the host path, got {other:?}"),
    }
}

#[test]
fn command_attaches_a_host_directory_on_e_and_f() {
    let mut spec = sample_spec();
    spec.volumes = vec![
        host_dir("E:", "/tmp/dosker-binds/extra", false),
        host_dir("F:", "/tmp/dosker-binds/more", false),
    ];
    let args = build(&spec);

    let extra = String::from("file=fat:rw:/tmp/dosker-binds/extra,format=raw,if=ide,index=2");
    let more = String::from("file=fat:rw:/tmp/dosker-binds/more,format=raw,if=ide,index=3");
    assert!(has_pair(&args, "-drive", &extra));
    assert!(has_pair(&args, "-drive", &more));
}

#[test]
fn command_rejects_a_host_directory_on_a_floppy_drive() {
    for drive in ["A:", "B:"] {
        let mut spec = sample_spec();
        spec.volumes = vec![host_dir(drive, "/tmp/dosker-binds/host", false)];

        match command::build(&spec, &sample_binary()) {
            Err(EngineError::Setup(_)) => {}
            other => {
                panic!("expected a setup error for a host directory on {drive}, got {other:?}")
            }
        }
    }
}

#[test]
fn command_rejects_a_volume_drive_outside_the_supported_set() {
    let mut spec = sample_spec();
    spec.volumes = vec![volume("G:", "unsupported", false)];

    match command::build(&spec, &sample_binary()) {
        Err(EngineError::Setup(_)) => {}
        other => panic!("expected a setup error for an unsupported drive, got {other:?}"),
    }
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
        Capability::HostDirLive,
        Capability::DebugNative,
        Capability::GdbStub,
        Capability::Screenshot,
        Capability::Vnc,
        Capability::Nat,
    ]
    .into_iter()
    .collect();

    assert_eq!(adapter.name(), "qemu");
    assert_eq!(adapter.version(), "8.2.0");
    assert_eq!(adapter.capabilities(), expected);
}

#[test]
fn capabilities_include_host_dir_live() {
    let processes: Arc<dyn ProcessSupervisor> = Arc::new(system());
    let adapter = QemuAdapter::new(sample_binary(), processes);

    assert!(adapter.capabilities().contains(Capability::HostDirLive));
}

#[test]
fn capabilities_include_nat() {
    let processes: Arc<dyn ProcessSupervisor> = Arc::new(system());
    let adapter = QemuAdapter::new(sample_binary(), processes);

    assert!(adapter.capabilities().contains(Capability::Nat));
}

fn serial_values(args: &[String]) -> Vec<&str> {
    args.windows(2)
        .filter(|pair| pair[0].as_str() == "-serial")
        .map(|pair| pair[1].as_str())
        .collect()
}

fn serial_option(kind: &EngineEndpointKind) -> String {
    match kind {
        EngineEndpointKind::UnixSocket(path) => {
            format!("unix:{},server=on,wait=off", path.display())
        }
        EngineEndpointKind::Tcp { host, port } => format!("tcp:{host}:{port},server=on,wait=off"),
    }
}

#[test]
fn command_allows_a_reboot_for_a_transient_build() {
    let mut spec = sample_spec();
    assert!(
        build(&spec).iter().any(|argument| argument == "-no-reboot"),
        "the runtime command must stop a reboot"
    );

    spec.allow_reboot = true;
    assert!(
        !build(&spec).iter().any(|argument| argument == "-no-reboot"),
        "a transient build must let the guest reboot"
    );
}

#[test]
fn command_maps_the_agent_to_the_effective_control_port() {
    let mut spec = sample_spec();
    spec.serial_ports = 3;
    spec.control_port = 3;
    let command = build_command(&spec);
    let agent = agent_serial_option(&command);
    let values = serial_values(&command.args);

    assert_eq!(values, ["none", "none", agent.as_str()]);

    spec.serial_ports = 2;
    spec.control_port = 2;
    let command = build_command(&spec);
    let agent = agent_serial_option(&command);
    let values = serial_values(&command.args);

    assert_eq!(values, ["none", agent.as_str()]);
}

fn agent_serial_option(command: &EngineCommand) -> String {
    let endpoint = command
        .endpoints
        .iter()
        .find(|endpoint| endpoint.name == "serial_agent")
        .expect("the command carries an agent endpoint");

    serial_option(&endpoint.kind)
}

#[test]
fn command_rejects_a_control_port_outside_the_serial_layout() {
    let mut spec = sample_spec();
    spec.control_port = 0;
    match command::build(&spec, &sample_binary()) {
        Err(EngineError::Setup(reason)) => assert!(
            reason.contains("control port"),
            "unexpected setup reason: {reason}"
        ),
        other => panic!("expected a setup error for control port 0, got {other:?}"),
    }

    spec.serial_ports = 2;
    spec.control_port = 3;
    match command::build(&spec, &sample_binary()) {
        Err(EngineError::Setup(reason)) => assert!(
            reason.contains("control port"),
            "unexpected setup reason: {reason}"
        ),
        other => panic!("expected a setup error for control port 3, got {other:?}"),
    }
}
