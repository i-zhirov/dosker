use std::collections::HashSet;
use std::io::{self, BufReader, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use dosker_cli::cli::StatusArgs;
use dosker_cli::commands::status;
use dosker_cli::context::Context;
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_platform::console::Console;
use dosker_platform::endpoint::control_endpoint;
use dosker_platform::{Endpoint, EndpointRecord, PlatformError};
use dosker_runtime::protocol::{ContainerSummary, Reply, Response};
use dosker_runtime::stale::HostProbe;
use dosker_runtime::{ContainerRecord, StateStore};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }

    fn lines(&self) -> Vec<String> {
        self.text().lines().map(str::to_owned).collect()
    }
}

fn context_for(root: &Path) -> (Context, Capture) {
    let stdout = Capture::default();
    let stderr = Capture::default();
    let context = Context::with_console(
        root.to_path_buf(),
        Console::new(Box::new(stdout.clone()), Box::new(stderr)),
    );
    (context, stdout)
}

struct FakeProbe {
    live_pids: HashSet<u32>,
    endpoints_alive: bool,
}

impl FakeProbe {
    fn live(pid: u32) -> Self {
        let mut live_pids = HashSet::new();
        live_pids.insert(pid);
        Self {
            live_pids,
            endpoints_alive: true,
        }
    }

    fn live_with_dead_endpoints(pid: u32) -> Self {
        let mut live_pids = HashSet::new();
        live_pids.insert(pid);
        Self {
            live_pids,
            endpoints_alive: false,
        }
    }

    fn dead() -> Self {
        Self {
            live_pids: HashSet::new(),
            endpoints_alive: false,
        }
    }
}

impl HostProbe for FakeProbe {
    fn process_alive(&self, pid: u32) -> bool {
        self.live_pids.contains(&pid)
    }

    fn endpoint_alive(&self, _endpoint: &EndpointRecord) -> bool {
        self.endpoints_alive
    }

    fn kill_group(&self, _pgid: u32) -> Result<(), PlatformError> {
        Ok(())
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn add_record(
    root: &Path,
    id: &str,
    name: &str,
    status: Status,
    pid: Option<u32>,
    created_at: DateTime<Utc>,
) -> ContainerRecord {
    let id = ContainerId::parse(id).unwrap();
    let name = ContainerName::parse(name).unwrap();
    let endpoint = control_endpoint(root, &id);
    let mut record = ContainerRecord::new(
        id,
        name,
        "freedos:latest".to_owned(),
        "mock".to_owned(),
        endpoint.record(),
        created_at,
    );
    record.status = status;
    record.pid = pid;
    StateStore::new(root.to_path_buf()).create(&record).unwrap();
    record
}

fn spawn_status_stub(endpoint: Endpoint, reply: Reply) {
    let listener = dosker_platform::control::bind(&endpoint).unwrap();
    std::thread::spawn(move || {
        if let Ok(Some(connection)) = listener.accept(Duration::from_secs(5)) {
            let mut reader = BufReader::new(connection);
            let _ = dosker_runtime::read_frame(&mut reader);
            let response = Response::ok(reply);
            let _ = dosker_runtime::write_message(reader.get_mut(), &response);
        }
    });
}

#[test]
fn status_accepts_a_full_id() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let record = add_record(root, "000000000020", "by-id", Status::Stopped, None, at(0));

    let (mut context, output) = context_for(root);
    status::execute(
        StatusArgs {
            container: record.id.to_string(),
        },
        &mut context,
        &FakeProbe::dead(),
        at(100),
    )
    .unwrap();

    assert_eq!(
        output.text(),
        format!("{} by-id stopped pid=- engine_pid=-\n", record.id)
    );
}

#[test]
fn status_accepts_a_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    add_record(root, "000000000021", "by-name", Status::Exited, None, at(0));

    let (mut context, output) = context_for(root);
    status::execute(
        StatusArgs {
            container: String::from("by-name"),
        },
        &mut context,
        &FakeProbe::dead(),
        at(100),
    )
    .unwrap();

    let lines = output.lines();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains(" by-name exited "));
}

#[test]
fn status_uses_the_supervisor_reply_when_reachable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let record = add_record(
        root,
        "000000000022",
        "reachable",
        Status::Starting,
        Some(3000),
        at(0),
    );

    let endpoint = record.control_endpoint().unwrap().clone();
    let summary = ContainerSummary {
        id: record.id.clone(),
        name: record.name.clone(),
        image: record.image.clone(),
        status: Status::Running,
        pid: Some(3000),
        engine_pid: Some(4000),
        exit_code: Some(0),
    };
    spawn_status_stub(
        Endpoint::from(endpoint),
        Reply::Status { container: summary },
    );

    let (mut context, output) = context_for(root);
    status::execute(
        StatusArgs {
            container: record.id.to_string(),
        },
        &mut context,
        &FakeProbe::live(3000),
        at(100),
    )
    .unwrap();

    assert_eq!(
        output.text(),
        format!("{} reachable running pid=3000 engine_pid=4000\n", record.id)
    );
}

#[test]
fn status_falls_back_to_the_record_when_the_endpoint_is_dead() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let record = add_record(
        root,
        "000000000023",
        "dead-endpoint",
        Status::Running,
        Some(3001),
        at(0),
    );

    let (mut context, output) = context_for(root);
    status::execute(
        StatusArgs {
            container: record.id.to_string(),
        },
        &mut context,
        &FakeProbe::live_with_dead_endpoints(3001),
        at(100),
    )
    .unwrap();

    assert_eq!(
        output.text(),
        format!("{} dead-endpoint exited pid=- engine_pid=-\n", record.id)
    );
}

#[test]
fn status_unknown_container_exits_one() {
    let dir = tempfile::tempdir().unwrap();
    let (mut context, _output) = context_for(dir.path());

    let error = status::execute(
        StatusArgs {
            container: String::from("missing-container"),
        },
        &mut context,
        &FakeProbe::dead(),
        at(0),
    )
    .unwrap_err();

    assert_eq!(error.exit_code(), 1);
}

#[test]
fn status_rejects_an_invalid_reference_with_exit_two() {
    let dir = tempfile::tempdir().unwrap();
    let (mut context, _output) = context_for(dir.path());

    let error = status::execute(
        StatusArgs {
            container: String::from("Bad Name"),
        },
        &mut context,
        &FakeProbe::dead(),
        at(0),
    )
    .unwrap_err();

    assert_eq!(error.exit_code(), 2);
}
