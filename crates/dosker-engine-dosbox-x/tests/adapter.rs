use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
#[cfg(unix)]
use std::time::{Duration, Instant};

use dosker_core::ContainerId;
use dosker_engine::{DisplayMode, EngineAdapter, EngineError, EngineSpec};
use dosker_engine_dosbox_x::{CONFIG_FILE_NAME, DosboxXAdapter, DosboxXBinary, HEADLESS_FLAGS};
use dosker_platform::process::system;

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

fn adapter(path: &Path) -> DosboxXAdapter {
    DosboxXAdapter::new(
        DosboxXBinary {
            path: path.to_path_buf(),
            version: "2026.10.01".to_owned(),
        },
        Arc::new(system()),
    )
}

#[test]
fn build_command_writes_the_config_and_returns_the_conf_argument() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let adapter = adapter(Path::new("dosbox-x"));

    let command = adapter.build_command(&spec).expect("build the command");
    let config_path = spec.log_dir.join(CONFIG_FILE_NAME);
    let expected = config_path.to_string_lossy().into_owned();

    assert_eq!(
        command.args.first().map(String::as_str),
        Some("-conf"),
        "args: {:?}",
        command.args
    );
    assert_eq!(
        command.args.get(1),
        Some(&expected),
        "args: {:?}",
        command.args
    );

    let config = std::fs::read_to_string(&config_path).expect("read the generated config");
    assert!(config.contains("machine = svga_s3"), "config:\n{config}");
    assert!(
        config.contains(&format!(
            "IMGMOUNT C \"{}\" -t hdd",
            spec.disk_path.display()
        )),
        "config:\n{config}"
    );
    assert!(config.contains("BOOT C:"), "config:\n{config}");
}

#[test]
fn build_command_adds_the_headless_flags_for_display_none() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.display = DisplayMode::None;
    let adapter = adapter(Path::new("dosbox-x"));

    let command = adapter.build_command(&spec).expect("build the command");
    let tail: Vec<&str> = command.args[2..].iter().map(String::as_str).collect();

    assert_eq!(HEADLESS_FLAGS, ["-set", "sdl videodriver=dummy"]);
    assert_eq!(tail.as_slice(), HEADLESS_FLAGS, "args: {:?}", command.args);
}

#[test]
fn build_command_omits_the_headless_flags_for_display_window() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.display = DisplayMode::Window;
    let adapter = adapter(Path::new("dosbox-x"));

    let command = adapter.build_command(&spec).expect("build the command");

    assert_eq!(command.args.len(), 2, "args: {:?}", command.args);
    assert!(
        !command.args.iter().any(|argument| argument == "-set"),
        "args: {:?}",
        command.args
    );
}

#[test]
fn build_command_rejects_vnc() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.display = DisplayMode::Vnc;
    let adapter = adapter(Path::new("dosbox-x"));

    let error = adapter.build_command(&spec).expect_err("reject vnc");

    assert!(matches!(error, EngineError::Unsupported("vnc")));
    assert!(
        !spec.log_dir.join(CONFIG_FILE_NAME).exists(),
        "the config was written before the rejection"
    );
}

#[test]
fn probe_rejects_a_missing_binary() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let path = dir.path().join("missing-dosbox-x");
    let mut adapter = adapter(&path);

    let error = adapter.probe().expect_err("a missing binary must fail");

    match error {
        EngineError::NotFound(missing) => assert_eq!(missing, path),
        other => panic!("expected not found, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn start_spawns_the_process_with_the_generated_command() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("create temp directory");
    let script = dir.path().join("dosbox-x");
    std::fs::write(&script, "#!/bin/sh\necho dosbox-x started\nexec sleep 30\n")
        .expect("write the fake engine");
    let mut permissions = std::fs::metadata(&script)
        .expect("read the script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("set the script permissions");

    let spec = sample_spec(dir.path());
    let mut adapter = adapter(&script);

    let session = adapter.start(&spec).expect("start the fake engine");
    assert!(session.pid().is_some(), "the session has a child pid");

    let log_path = spec.log_dir.join("engine.log");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let captured = std::fs::read_to_string(&log_path).unwrap_or_default();
        if captured.contains("dosbox-x started") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "engine.log did not capture the child output"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    drop(session);
}
