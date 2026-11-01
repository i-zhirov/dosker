use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use dosker_core::ContainerId;
use dosker_engine::{
    Capability, CapabilitySet, EngineAdapter, EngineError, EngineSession, EngineSpec, ExecRequest,
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
        container_id: ContainerId::parse("0123456789ab").unwrap(),
        workdir: "C:\\DOSKER".to_owned(),
        env: BTreeMap::from([("NAME".to_owned(), "VALUE".to_owned())]),
        cmd: Some("ECHO HELLO".to_owned()),
        expects_agent: true,
        log_dir: PathBuf::from("/tmp/dosker-logs"),
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
