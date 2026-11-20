use std::collections::BTreeMap;
use std::path::Path;

use dosker_core::ContainerId;
use dosker_engine::{
    Capability, DisplayMode, EngineEndpointKind, EngineError, EngineSpec, EngineVolume,
    EngineVolumeKind,
};
use dosker_engine_bochs::{BochsEndpoints, CONFIG_FILE_NAME, render_config, write_config};

const DISK_BYTES: u64 = 33_554_432;

fn sample_spec(root: &Path) -> EngineSpec {
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
    let log_dir = root.join("containers").join(id.as_str());
    let disk_path = log_dir.join("disk.img");
    std::fs::create_dir_all(&log_dir).expect("create container directory");
    std::fs::File::create(&disk_path)
        .and_then(|file| file.set_len(DISK_BYTES))
        .expect("create disk image");

    EngineSpec {
        root: root.to_path_buf(),
        container_id: id,
        workdir: "C:\\DOSKER".to_owned(),
        env: BTreeMap::new(),
        cmd: None,
        expects_agent: true,
        log_dir,
        disk_path,
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

fn agent_port(endpoints: &BochsEndpoints) -> u16 {
    match endpoints.agent_serial.as_ref().expect("an agent endpoint") {
        EngineEndpointKind::Tcp { port, .. } => *port,
        EngineEndpointKind::UnixSocket(path) => {
            panic!("the agent endpoint is not TCP: {}", path.display())
        }
    }
}

fn assert_line(config: &str, expected: &str) {
    assert!(
        config.lines().any(|line| line == expected),
        "expected the config to contain the line {expected:?}:\n{config}"
    );
}

fn line_index(config: &str, expected: &str) -> usize {
    config
        .lines()
        .position(|line| line == expected)
        .unwrap_or_else(|| {
            panic!("expected the config to contain the line {expected:?}:\n{config}")
        })
}

#[test]
fn renders_the_reference_boot_keys() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(&config, "romimage: file=$BXSHARE/BIOS-bochs-latest");
    assert_line(
        &config,
        "vgaromimage: file=$BXSHARE/VGABIOS-lgpl-latest.bin",
    );
    assert_line(&config, "cpu: model=i486dx4");
    assert_line(&config, "megs: 16");
    assert_line(&config, "boot: disk");
}

#[test]
fn renders_the_fixed_keys_in_order() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let port = agent_port(&endpoints);
    let config = render_config(&spec, &endpoints).expect("render config");

    let expected = [
        "romimage: file=$BXSHARE/BIOS-bochs-latest".to_owned(),
        "vgaromimage: file=$BXSHARE/VGABIOS-lgpl-latest.bin".to_owned(),
        "cpu: model=i486dx4".to_owned(),
        "megs: 16".to_owned(),
        format!(
            "ata0-master: type=disk, path=\"{}\", mode=flat, cylinders=128, heads=16, spt=32",
            spec.disk_path.display()
        ),
        "boot: disk".to_owned(),
        "display_library: nogui".to_owned(),
        "com1: enabled=1, mode=null".to_owned(),
        format!("com2: enabled=1, mode=socket-server, dev=localhost:{port}"),
        format!("log: \"{}\"", spec.log_dir.join("bochs.log").display()),
        "panic: action=fatal".to_owned(),
        "error: action=report".to_owned(),
        "info: action=report".to_owned(),
        "debug: action=ignore".to_owned(),
    ];

    let mut previous = None;
    for line in &expected {
        let index = line_index(&config, line);
        if let Some(previous) = previous {
            assert!(
                previous < index,
                "expected the line {line:?} after line index {previous}:\n{config}"
            );
        }
        previous = Some(index);
    }
}

#[test]
fn renders_the_ata_line_with_the_computed_geometry() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(
        &config,
        &format!(
            "ata0-master: type=disk, path=\"{}\", mode=flat, cylinders=128, heads=16, spt=32",
            spec.disk_path.display()
        ),
    );
}

#[test]
fn renders_the_nogui_display_library() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(&config, "display_library: nogui");
}

#[test]
fn rejects_a_window_display() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.display = DisplayMode::Window;
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject window display");

    assert!(matches!(error, EngineError::Unsupported("window display")));
}

#[test]
fn rejects_vnc() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.display = DisplayMode::Vnc;
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject vnc display");

    assert!(matches!(error, EngineError::Unsupported("vnc")));
}

#[test]
fn renders_both_serial_ports() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.control_port = 2;
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let port = agent_port(&endpoints);
    let config = render_config(&spec, &endpoints).expect("render config");

    let guest = line_index(&config, "com1: enabled=1, mode=null");
    let agent = line_index(
        &config,
        &format!("com2: enabled=1, mode=socket-server, dev=localhost:{port}"),
    );

    assert!(guest < agent, "com1 precedes com2:\n{config}");
}

#[test]
fn renders_the_agent_on_com1_when_the_control_port_is_one() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.control_port = 1;
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let port = agent_port(&endpoints);
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(
        &config,
        &format!("com1: enabled=1, mode=socket-server, dev=localhost:{port}"),
    );
    assert_line(&config, "com2: enabled=1, mode=null");
}

#[test]
fn rejects_a_control_port_outside_com1_and_com2() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.control_port = 3;
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject control port 3");

    match error {
        EngineError::Setup(message) => assert_eq!(
            message,
            "the control port 3 is outside the two Bochs serial ports"
        ),
        other => panic!("expected a setup error, got {other:?}"),
    }
}

#[test]
fn renders_the_log_path_quoted() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.log_dir = dir.path().join("container dir");
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(
        &config,
        &format!("log: \"{}\"", spec.log_dir.join("bochs.log").display()),
    );
}

#[test]
fn appends_engine_config_lines_in_order() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.engine_config.insert(
        "bochs".to_owned(),
        vec![
            "floppy_bootsig_check: disabled=0".to_owned(),
            "clock: sync=realtime".to_owned(),
        ],
    );
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    let actions = line_index(&config, "debug: action=ignore");
    let first = line_index(&config, "floppy_bootsig_check: disabled=0");
    let second = line_index(&config, "clock: sync=realtime");

    assert!(actions < first, "the raw lines follow the fixed keys");
    assert!(first < second, "the raw lines keep their order");
}

#[test]
fn omits_the_agent_serial_line_for_a_booter() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.booter = true;
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(&config, "com1: enabled=1, mode=null");
    assert_line(&config, "com2: enabled=1, mode=null");
    assert!(
        !config.lines().any(|line| line.contains("socket-server")),
        "a booter has no agent serial line:\n{config}"
    );
}

#[test]
fn rejects_host_directories() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.volumes.push(EngineVolume {
        drive: "D:".to_owned(),
        path: dir.path().join("host"),
        read_only: false,
        kind: EngineVolumeKind::HostDir,
    });
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject host directory");

    assert!(matches!(error, EngineError::Unsupported("host directory")));
}

#[test]
fn rejects_named_volumes() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.volumes.push(EngineVolume {
        drive: "D:".to_owned(),
        path: dir.path().join("data.img"),
        read_only: false,
        kind: EngineVolumeKind::Image,
    });
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject volume image");

    assert!(matches!(error, EngineError::Unsupported("volume image")));
}

#[test]
fn rejects_ne2000_without_the_nat_capability() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.net = "ne2000".to_owned();
    spec.ports = Vec::new();
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject ne2000");

    assert!(matches!(
        error,
        EngineError::Unsupported("network address translation")
    ));
}

#[test]
fn rejects_an_unknown_network() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.net = "foo".to_owned();
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject unknown network");

    match error {
        EngineError::Setup(message) => assert_eq!(message, "unknown network device foo"),
        other => panic!("expected a setup error, got {other:?}"),
    }
}

#[test]
fn the_nat_capability_message_names_qemu() {
    assert_eq!(
        dosker_engine::capability_message("bochs", Capability::Nat),
        "engine bochs does not provide the nat capability; the qemu engine provides it"
    );
}

#[test]
fn omits_the_nic_for_a_none_network() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.net = "none".to_owned();
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    let network_keys = ["e1000", "ne2k", "nic", "ethernet", "slirp"];
    assert!(
        !config.lines().any(|line| {
            let key = line.split(':').next().unwrap_or_default().trim();
            network_keys.iter().any(|prefix| key.starts_with(prefix))
        }),
        "a none network has no network key:\n{config}"
    );
}

#[test]
fn writes_a_deterministic_config() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let first = render_config(&spec, &endpoints).expect("render config");
    let second = render_config(&spec, &endpoints).expect("render config");

    assert_eq!(first, second);
}

#[test]
fn write_config_creates_the_container_directory() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    let log_dir = dir.path().join("missing").join("container");
    assert!(!log_dir.exists(), "the container directory must not exist");
    spec.log_dir = log_dir.clone();
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let path = write_config(&spec, &endpoints).expect("write config");

    assert!(log_dir.is_dir(), "write_config creates the directory");
    assert_eq!(path, log_dir.join(CONFIG_FILE_NAME));
    assert!(path.is_file(), "write_config writes the config file");
}

#[test]
fn writes_the_config_to_the_container_directory() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = BochsEndpoints::for_spec(&spec).expect("resolve endpoints");

    let path = write_config(&spec, &endpoints).expect("write config");

    assert_eq!(path, spec.log_dir.join(CONFIG_FILE_NAME));
    let written = std::fs::read_to_string(&path).expect("read the config");
    assert_eq!(
        written,
        render_config(&spec, &endpoints).expect("render config")
    );
}
