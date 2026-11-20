use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
#[cfg(unix)]
use std::time::{Duration, Instant};

use dosker_core::ContainerId;
use dosker_engine::{DisplayMode, EngineAdapter, EngineError, EngineSpec};
use dosker_engine_bochs::{BochsAdapter, BochsBinary, CONFIG_FILE_NAME, Debugger};
use dosker_platform::process::system;

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

fn adapter(path: &Path, debugger: Debugger) -> BochsAdapter {
    BochsAdapter::new(
        BochsBinary {
            path: path.to_path_buf(),
            version: "3.1".to_owned(),
            debugger,
        },
        Arc::new(system()),
    )
}

fn fixed_args(spec: &EngineSpec) -> Vec<String> {
    vec![
        "-q".to_owned(),
        "-f".to_owned(),
        spec.log_dir
            .join(CONFIG_FILE_NAME)
            .to_string_lossy()
            .into_owned(),
    ]
}

#[test]
fn build_command_writes_the_config_and_returns_the_fixed_arguments() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let adapter = adapter(Path::new("bochs"), Debugger::Internal);

    let command = adapter.build_command(&spec).expect("build the command");
    let config_path = spec.log_dir.join(CONFIG_FILE_NAME);
    let mut expected = fixed_args(&spec);
    if cfg!(unix) {
        expected.push("-dbg".to_owned());
    }

    assert_eq!(command.args, expected);
    assert_eq!(command.program, Path::new("bochs"));
    assert!(command.env.is_empty(), "env: {:?}", command.env);

    let config = std::fs::read_to_string(&config_path).expect("read the generated config");
    assert!(config.contains("boot: disk"), "config:\n{config}");
    assert!(
        config.contains(&format!(
            "log: \"{}\"",
            spec.log_dir.join("bochs.log").display()
        )),
        "config:\n{config}"
    );
}

#[test]
fn build_command_omits_the_debug_flag_without_an_internal_debugger() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let spec = sample_spec(dir.path());
    let adapter = adapter(Path::new("bochs"), Debugger::None);

    let command = adapter.build_command(&spec).expect("build the command");

    assert_eq!(command.args, fixed_args(&spec));
    assert!(
        !command.args.iter().any(|argument| argument == "-dbg"),
        "args: {:?}",
        command.args
    );
}

#[test]
fn build_command_rejects_vnc() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.display = DisplayMode::Vnc;
    let adapter = adapter(Path::new("bochs"), Debugger::Internal);

    let error = adapter.build_command(&spec).expect_err("reject vnc");

    assert!(matches!(error, EngineError::Unsupported("vnc")));
    assert!(
        !spec.log_dir.join(CONFIG_FILE_NAME).exists(),
        "the config was written before the rejection"
    );
}

#[test]
fn build_command_rejects_a_window_display() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut spec = sample_spec(dir.path());
    spec.display = DisplayMode::Window;
    let adapter = adapter(Path::new("bochs"), Debugger::Internal);

    let error = adapter
        .build_command(&spec)
        .expect_err("reject window display");

    assert!(matches!(error, EngineError::Unsupported("window display")));
    assert!(
        !spec.log_dir.join(CONFIG_FILE_NAME).exists(),
        "the config was written before the rejection"
    );
}

#[test]
fn probe_rejects_a_missing_binary() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let path = dir.path().join("missing-bochs");
    let mut adapter = adapter(&path, Debugger::Internal);

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
    let script = dir.path().join("bochs");
    let args_file = dir.path().join("bochs.args");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\necho bochs started\nexec sleep 30\n",
            args_file.display()
        ),
    )
    .expect("write the fake engine");
    let mut permissions = std::fs::metadata(&script)
        .expect("read the script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("set the script permissions");

    let spec = sample_spec(dir.path());
    let mut adapter = adapter(&script, Debugger::Internal);

    let session = adapter.start(&spec).expect("start the fake engine");
    assert!(session.pid().is_some(), "the session has a child pid");

    let config_path = spec.log_dir.join(CONFIG_FILE_NAME);
    let config = std::fs::read_to_string(&config_path).expect("read the generated config");
    assert!(
        config.contains(&format!(
            "log: \"{}\"",
            spec.log_dir.join("bochs.log").display()
        )),
        "config:\n{config}"
    );

    let log_path = spec.log_dir.join("engine.log");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let captured = std::fs::read_to_string(&log_path).unwrap_or_default();
        if captured.contains("bochs started") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "engine.log did not capture the child output"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let written_args = std::fs::read_to_string(&args_file).expect("read the recorded arguments");
    assert!(
        written_args.lines().any(|line| line == "-dbg"),
        "arguments: {written_args:?}"
    );

    drop(session);
}

#[test]
fn adapter_reports_the_discovered_version() {
    let adapter = adapter(Path::new("bochs"), Debugger::Internal);

    assert_eq!(adapter.version(), "3.1");
}
