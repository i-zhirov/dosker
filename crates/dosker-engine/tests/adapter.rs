use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dosker_core::ContainerId;
use dosker_engine::{
    Capability, CapabilitySet, DebugKind, DisplayMode, EngineAdapter, EngineCommand,
    EngineEndpoint, EngineEndpointKind, EngineError, EngineSession, EngineSpec, ExecRequest,
    ExecResult,
};

struct StubSession;

impl EngineSession for StubSession {
    fn pid(&self) -> Option<u32> {
        Some(4321)
    }

    fn process_group(&self) -> Option<u32> {
        Some(4321)
    }

    fn ready(&mut self, _timeout: Duration) -> Result<(), EngineError> {
        Ok(())
    }

    fn exec(&mut self, request: &ExecRequest) -> Result<ExecResult, EngineError> {
        Ok(ExecResult {
            exit_code: 0,
            output: request.command.clone(),
            timed_out: false,
            truncated: false,
        })
    }

    fn stop(&mut self, _grace: Duration) -> Result<i32, EngineError> {
        Ok(7)
    }

    fn kill(&mut self) -> Result<i32, EngineError> {
        Ok(9)
    }

    fn try_exit(&mut self) -> Result<Option<i32>, EngineError> {
        Ok(None)
    }
}

struct StubAdapter {
    capabilities: CapabilitySet,
}

impl EngineAdapter for StubAdapter {
    fn name(&self) -> &str {
        "stub"
    }

    fn version(&self) -> &str {
        "9.9.9"
    }

    fn capabilities(&self) -> CapabilitySet {
        self.capabilities
    }

    fn probe(&mut self) -> Result<(), EngineError> {
        Ok(())
    }

    fn start(&mut self, _spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError> {
        Ok(Box::new(StubSession))
    }
}

fn sample_spec() -> EngineSpec {
    EngineSpec {
        root: PathBuf::from("/tmp/dosker"),
        container_id: ContainerId::parse("0123456789ab").unwrap(),
        workdir: "C:\\DOSKER".to_owned(),
        env: BTreeMap::from([("NAME".to_owned(), "VALUE".to_owned())]),
        cmd: Some("ECHO HELLO".to_owned()),
        expects_agent: true,
        log_dir: PathBuf::from("/tmp/dosker-logs"),
        disk_path: PathBuf::from("/tmp/dosker/containers/0123456789ab/disk.img"),
        booter: false,
        display: DisplayMode::None,
        vnc_port: None,
    }
}

#[test]
fn adapter_is_object_safe() {
    let capabilities: CapabilitySet = [Capability::Boot, Capability::Serial].into_iter().collect();
    let mut adapter: Box<dyn EngineAdapter> = Box::new(StubAdapter { capabilities });

    assert_eq!(adapter.name(), "stub");
    assert_eq!(adapter.version(), "9.9.9");
    assert_eq!(adapter.capabilities(), capabilities);
    adapter.probe().unwrap();

    let mut session: Box<dyn EngineSession> = adapter.start(&sample_spec()).unwrap();

    assert_eq!(session.pid(), Some(4321));
    assert_eq!(session.process_group(), Some(4321));
    session.ready(Duration::from_secs(1)).unwrap();

    let request = ExecRequest {
        command: "ECHO HELLO".to_owned(),
        env: BTreeMap::new(),
        workdir: "C:\\".to_owned(),
        timeout_ms: 30_000,
    };
    let result = session.exec(&request).unwrap();
    assert_eq!(result.output, "ECHO HELLO");

    assert_eq!(session.stop(Duration::from_secs(1)).unwrap(), 7);
    assert_eq!(session.kill().unwrap(), 9);
    assert_eq!(session.try_exit().unwrap(), None);
}

#[test]
fn spec_holds_the_pinned_fields() {
    let spec = sample_spec();

    assert_eq!(spec.container_id.as_str(), "0123456789ab");
    assert_eq!(spec.workdir, "C:\\DOSKER");
    assert_eq!(spec.env.get("NAME").map(String::as_str), Some("VALUE"));
    assert_eq!(spec.cmd.as_deref(), Some("ECHO HELLO"));
    assert!(spec.expects_agent);
    assert_eq!(spec.log_dir, PathBuf::from("/tmp/dosker-logs"));
    assert_eq!(spec.root, PathBuf::from("/tmp/dosker"));
    assert_eq!(
        spec.disk_path,
        PathBuf::from("/tmp/dosker/containers/0123456789ab/disk.img")
    );
    assert!(!spec.booter);
    assert_eq!(spec.display, DisplayMode::None);
    assert_eq!(spec.vnc_port, None);
}

#[test]
fn display_mode_names_round_trip() {
    for (mode, name) in [
        (DisplayMode::None, "none"),
        (DisplayMode::Window, "window"),
        (DisplayMode::Vnc, "vnc"),
    ] {
        assert_eq!(mode.as_str(), name);
        assert_eq!(DisplayMode::parse(name), Some(mode));
        assert_eq!(serde_json::to_string(&mode).unwrap(), format!("\"{name}\""));
    }

    assert_eq!(DisplayMode::parse("headless"), None);
    assert_eq!(DisplayMode::default(), DisplayMode::None);
}

#[test]
fn debug_kind_is_gdb() {
    assert_eq!(serde_json::to_string(&DebugKind::Gdb).unwrap(), "\"gdb\"");
    assert_eq!(
        serde_json::from_str::<DebugKind>("\"gdb\"").unwrap(),
        DebugKind::Gdb
    );
}

#[test]
fn engine_command_render_quotes_arguments() {
    let command = EngineCommand {
        program: PathBuf::from("/usr/bin/qemu-system-i386"),
        args: vec![
            "-machine".to_owned(),
            "pc".to_owned(),
            "-display".to_owned(),
            "gtk,title=My Machine".to_owned(),
            "-fw_cfg".to_owned(),
            "name=opt/rom,string=it's".to_owned(),
        ],
        env: vec![("QEMU_LOG".to_owned(), "1".to_owned())],
    };

    let rendered = command.render();

    assert!(rendered.starts_with("/usr/bin/qemu-system-i386 -machine pc -display "));
    assert!(rendered.contains("'gtk,title=My Machine'"));
    assert!(rendered.contains("'name=opt/rom,string=it'\\''s'"));
    assert!(!rendered.contains("QEMU_LOG"));
}

#[test]
fn engine_endpoint_tcp_displays_loopback_address() {
    let endpoint = EngineEndpoint {
        name: "qmp".to_owned(),
        kind: EngineEndpointKind::Tcp {
            host: Ipv4Addr::LOCALHOST,
            port: 45_000,
        },
    };

    assert_eq!(endpoint.kind.display(), "127.0.0.1:45000");
}

#[test]
fn engine_endpoint_unix_displays_the_path() {
    let endpoint = EngineEndpoint {
        name: "qmp".to_owned(),
        kind: EngineEndpointKind::UnixSocket(PathBuf::from("/tmp/dosker/qmp.sock")),
    };

    assert_eq!(endpoint.kind.display(), "/tmp/dosker/qmp.sock");
}

#[test]
fn default_session_features_are_unsupported() {
    let mut session: Box<dyn EngineSession> = Box::new(StubSession);

    assert!(session.command().is_none());
    assert!(session.endpoints().is_empty());

    match session.screenshot(Path::new("/tmp/screen.png")) {
        Err(EngineError::Unsupported("screenshot")) => {}
        other => panic!("expected Unsupported screenshot, got {other:?}"),
    }
    match session.debug_endpoint(DebugKind::Gdb) {
        Err(EngineError::Unsupported("debugger")) => {}
        other => panic!("expected Unsupported debugger, got {other:?}"),
    }
    match session.put_file(r"C:\FILE.TXT", b"data") {
        Err(EngineError::Unsupported("file upload")) => {}
        other => panic!("expected Unsupported file upload, got {other:?}"),
    }
    match session.get_file(r"C:\FILE.TXT") {
        Err(EngineError::Unsupported("file download")) => {}
        other => panic!("expected Unsupported file download, got {other:?}"),
    }
}

#[test]
fn no_agent_error_message() {
    assert_eq!(EngineError::NoAgent.to_string(), "the image has no agent");
}

#[test]
fn guest_rebooted_error_message() {
    assert_eq!(
        EngineError::GuestRebooted.to_string(),
        "the guest rebooted during the command"
    );
}

#[test]
fn exec_result_equality_covers_all_fields() {
    let base = ExecResult {
        exit_code: 0,
        output: "HELLO\r\n".to_owned(),
        timed_out: false,
        truncated: false,
    };
    assert_eq!(base, base.clone());

    assert_ne!(
        base,
        ExecResult {
            exit_code: 1,
            ..base.clone()
        }
    );
    assert_ne!(
        base,
        ExecResult {
            output: "BYE\r\n".to_owned(),
            ..base.clone()
        }
    );
    assert_ne!(
        base,
        ExecResult {
            timed_out: true,
            ..base.clone()
        }
    );
    assert_ne!(
        base,
        ExecResult {
            truncated: true,
            ..base.clone()
        }
    );
}
