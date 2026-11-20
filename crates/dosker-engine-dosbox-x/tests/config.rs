use std::collections::BTreeMap;
use std::path::Path;

use dosker_core::ContainerId;
use dosker_engine::{
    Capability, EngineEndpointKind, EngineError, EngineSpec, EngineVolume, EngineVolumeKind,
};
use dosker_engine_dosbox_x::{DosboxXEndpoints, render_config};

fn sample_spec(root: &Path) -> EngineSpec {
    let id = ContainerId::parse("0123456789ab").expect("parse container id");
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

fn agent_port(endpoints: &DosboxXEndpoints) -> u16 {
    match endpoints.agent_serial.as_ref().expect("an agent endpoint") {
        EngineEndpointKind::Tcp { port, .. } => *port,
        EngineEndpointKind::UnixSocket(path) => {
            panic!("the agent endpoint is not TCP: {}", path.display())
        }
    }
}

fn debug_port(endpoints: &DosboxXEndpoints) -> u16 {
    match endpoints.debug.as_ref().expect("a debug endpoint") {
        EngineEndpointKind::Tcp { port, .. } => *port,
        EngineEndpointKind::UnixSocket(path) => {
            panic!("the debug endpoint is not TCP: {}", path.display())
        }
    }
}

fn assert_line(config: &str, expected: &str) {
    assert!(
        config.lines().any(|line| line == expected),
        "expected the config to contain the line {expected:?}:\n{config}"
    );
}

fn assert_no_line(config: &str, expected: &str) {
    assert!(
        !config.lines().any(|line| line == expected),
        "expected the config to omit the line {expected:?}:\n{config}"
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
fn renders_the_default_machine_keys() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(&config, "[dosbox]");
    assert_line(&config, "machine = svga_s3");
    assert_line(&config, "memsize = 16");
    assert_line(&config, "[cpu]");
    assert_line(&config, "cputype = 486");
    assert_line(&config, "[sblaster]");
    assert_line(&config, "sbtype = sb16");
    assert_line(&config, "[ide, primary]");
    assert_line(&config, "enable = true");
}

#[test]
fn renders_both_serial_ports() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.control_port = 2;
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let port = agent_port(&endpoints);
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(&config, "serial1 = dummy");
    assert_line(
        &config,
        &format!("serial2 = nullmodem port:{port} sock:0 transparent:1"),
    );
}

#[test]
fn renders_the_agent_on_serial1_when_the_control_port_is_one() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.control_port = 1;
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let port = agent_port(&endpoints);
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(
        &config,
        &format!("serial1 = nullmodem port:{port} sock:0 transparent:1"),
    );
    assert_no_line(&config, "serial1 = dummy");
}

#[test]
fn renders_the_boot_command() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(
        &config,
        &format!("IMGMOUNT C \"{}\" -t hdd", spec.disk_path.display()),
    );
    assert_line(&config, "BOOT C:");
}

#[test]
fn appends_engine_config_lines_before_autoexec() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.engine_config.insert(
        "dosbox-x".to_owned(),
        vec!["[sdl]".to_owned(), "videodriver = dummy".to_owned()],
    );
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    let sdl = line_index(&config, "[sdl]");
    let video = line_index(&config, "videodriver = dummy");
    let autoexec = line_index(&config, "[autoexec]");

    assert!(sdl < video, "the raw lines keep their order");
    assert!(video < autoexec, "the raw lines precede [autoexec]");
}

#[test]
fn tracks_an_engine_config_autoexec_header() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.engine_config.insert(
        "dosbox-x".to_owned(),
        vec!["[autoexec]".to_owned(), "ECHO HI".to_owned()],
    );
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    let headers = config.lines().filter(|line| *line == "[autoexec]").count();

    assert_eq!(headers, 1, "a raw header replaces the generated one");
    assert_line(&config, "ECHO HI");
    assert_line(&config, "BOOT C:");
}

#[test]
fn rejects_host_directories_without_the_capability() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.volumes.push(EngineVolume {
        drive: "D:".to_owned(),
        path: dir.path().join("host"),
        read_only: false,
        kind: EngineVolumeKind::HostDir,
    });
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject host directory");

    assert!(matches!(error, EngineError::Unsupported("host directory")));
}

#[test]
fn rejects_image_volumes_without_support() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.volumes.push(EngineVolume {
        drive: "D:".to_owned(),
        path: dir.path().join("data.img"),
        read_only: false,
        kind: EngineVolumeKind::Image,
    });
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject volume image");

    assert!(matches!(error, EngineError::Unsupported("volume image")));
}

#[test]
fn rejects_ne2000_without_the_capability() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.net = "ne2000".to_owned();
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");

    let error = render_config(&spec, &endpoints).expect_err("reject ne2000");

    assert!(matches!(
        error,
        EngineError::Unsupported("network address translation")
    ));
    assert_eq!(
        dosker_engine::capability_message("dosbox-x", Capability::Nat),
        "engine dosbox-x does not provide the nat capability; the qemu engine provides it"
    );
}

#[test]
fn omits_the_nic_sections_when_the_network_is_none() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.net = "none".to_owned();
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_no_line(&config, "[ne2000]");
    assert_no_line(&config, "[ethernet, slirp]");
}

#[test]
fn writes_the_debug_key_when_declared() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let port = debug_port(&endpoints);
    let config = render_config(&spec, &endpoints).expect("render config");

    let dosbox = line_index(&config, "[dosbox]");
    let mcp = line_index(&config, &format!("mcp_server = {port}"));
    let cpu = line_index(&config, "[cpu]");

    assert!(dosbox < mcp, "the debug key is under [dosbox]");
    assert!(mcp < cpu, "the debug key is under [dosbox]");
}

#[test]
fn omits_the_agent_serial_line_for_a_booter() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.booter = true;
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");
    let config = render_config(&spec, &endpoints).expect("render config");

    assert_line(&config, "serial1 = dummy");
    assert!(
        !config.lines().any(|line| line.contains("nullmodem")),
        "a booter has no agent serial line:\n{config}"
    );
}

#[test]
fn writes_a_deterministic_config() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let endpoints = DosboxXEndpoints::for_spec(&spec).expect("resolve endpoints");

    let first = render_config(&spec, &endpoints).expect("render config");
    let second = render_config(&spec, &endpoints).expect("render config");

    assert_eq!(first, second);
}
