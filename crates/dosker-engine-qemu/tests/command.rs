#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use dosker_core::ContainerId;
use dosker_engine::{Capability, CapabilitySet, DisplayMode, EngineAdapter, EngineSpec};
use dosker_engine_qemu::command;
use dosker_engine_qemu::paths::QemuPaths;
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

fn build(spec: &EngineSpec) -> Vec<String> {
    command::build(spec, &sample_binary())
        .expect("build command")
        .args
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
    let paths = QemuPaths::for_spec(&spec);
    let args = build(&spec);
    let drive = format!(
        "file={},format=raw,if=ide,index=0",
        spec.disk_path.display()
    );

    assert!(has_pair(&args, "-drive", &drive));
    assert_eq!(
        args.iter()
            .filter(|argument| argument.as_str() == "-serial")
            .count(),
        2
    );
    let agent = format!("unix:{},server=on,wait=off", paths.agent_serial.display());
    assert!(has_pair(&args, "-serial", &agent));
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
fn command_exposes_qmp_monitor_and_gdb_sockets() {
    let spec = sample_spec();
    let paths = QemuPaths::for_spec(&spec);
    let args = build(&spec);

    assert!(has_pair(
        &args,
        "-qmp",
        &format!("unix:{},server=on,wait=off", paths.qmp.display())
    ));
    assert!(has_pair(
        &args,
        "-monitor",
        &format!("unix:{},server=on,wait=off", paths.monitor.display())
    ));
    assert!(has_pair(
        &args,
        "-chardev",
        &format!(
            "socket,id=gdb0,path={},server=on,wait=off",
            paths.gdb.display()
        )
    ));
    assert!(has_pair(&args, "-gdb", "chardev:gdb0"));
}

#[test]
fn command_uses_the_host_agent_socket_for_a_normal_image() {
    let spec = sample_spec();
    let paths = QemuPaths::for_spec(&spec);
    let args = build(&spec);

    let expected = format!("unix:{},server=on,wait=off", paths.agent_serial.display());
    assert_eq!(value_after(&args, "-serial"), Some("none"));
    assert!(args.iter().any(|argument| argument == &expected));
}

#[test]
fn booter_command_has_no_agent_socket() {
    let mut spec = sample_spec();
    spec.booter = true;
    let paths = QemuPaths::for_spec(&spec);
    let args = build(&spec);

    assert!(!args.iter().any(|argument| {
        argument.contains("agent") || argument.contains(&paths.agent_serial.display().to_string())
    }));
    assert_eq!(
        args.windows(2)
            .filter(|pair| pair[0].as_str() == "-serial" && pair[1].as_str() == "none")
            .count(),
        2
    );
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
