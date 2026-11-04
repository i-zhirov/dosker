use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use dosker_cli::cli::PsArgs;
use dosker_cli::commands::ps;
use dosker_cli::context::Context;
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_engine::DisplayMode;
use dosker_platform::console::Console;
use dosker_platform::endpoint::control_endpoint;
use dosker_platform::{Endpoint, EndpointRecord, PlatformError};
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
    let disk = root.join("containers").join(id.as_str()).join("disk.img");
    let mut record = ContainerRecord::new(
        id,
        name,
        "freedos:latest".to_owned(),
        "mock".to_owned(),
        endpoint.record(),
        created_at,
    )
    .with_image(
        String::from("sha256:0123456789abcdef"),
        false,
        Some(String::from("APP.EXE")),
    )
    .with_display(DisplayMode::None, None)
    .with_disk(disk);
    record.engine_command = Some(String::from("qemu-system-i386 -machine pc"));
    record.status = status;
    record.pid = pid;
    StateStore::new(root.to_path_buf()).create(&record).unwrap();
    record
}

fn row_fields(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

#[test]
fn ps_lists_only_active_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    add_record(
        root,
        "000000000001",
        "alpha",
        Status::Running,
        Some(1000),
        at(0),
    );
    add_record(root, "000000000002", "beta", Status::Stopped, None, at(1));

    let (mut context, output) = context_for(root);
    ps::execute(
        PsArgs { all: false },
        &mut context,
        &FakeProbe::live(1000),
        at(100),
    )
    .unwrap();

    let lines = output.lines();
    assert_eq!(lines[0], "ID NAME IMAGE STATUS CREATED");
    assert_eq!(lines.len(), 2);
    assert_eq!(
        row_fields(&lines[1]),
        vec![
            "000000000001",
            "alpha",
            "freedos:latest",
            "running",
            "2026-01-01T00:00:00Z"
        ]
    );
    assert!(!output.text().contains("beta"));
}

#[test]
fn ps_all_includes_stopped_and_exited() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    add_record(
        root,
        "000000000003",
        "running-one",
        Status::Running,
        Some(1001),
        at(0),
    );
    add_record(
        root,
        "000000000004",
        "stopped-one",
        Status::Stopped,
        None,
        at(1),
    );
    add_record(
        root,
        "000000000005",
        "exited-one",
        Status::Exited,
        None,
        at(2),
    );

    let (mut context, output) = context_for(root);
    ps::execute(
        PsArgs { all: true },
        &mut context,
        &FakeProbe::live(1001),
        at(100),
    )
    .unwrap();

    let lines = output.lines();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].starts_with("ID NAME IMAGE STATUS CREATED"));
    assert_eq!(row_fields(&lines[1])[1], "running-one");
    assert_eq!(row_fields(&lines[2])[1], "stopped-one");
    assert_eq!(row_fields(&lines[3])[1], "exited-one");
}

#[test]
fn ps_reports_exited_after_dead_supervisor() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let record = add_record(
        root,
        "000000000006",
        "dead-supervisor",
        Status::Running,
        Some(4242),
        at(0),
    );

    let (mut context, output) = context_for(root);
    ps::execute(
        PsArgs { all: true },
        &mut context,
        &FakeProbe::dead(),
        at(100),
    )
    .unwrap();

    let lines = output.lines();
    assert_eq!(lines.len(), 2);
    assert_eq!(row_fields(&lines[1])[1], "dead-supervisor");
    assert_eq!(row_fields(&lines[1])[3], "exited");
    assert!(!output.text().contains("running"));

    let stored = StateStore::new(root.to_path_buf())
        .load(&record.id)
        .unwrap();
    assert_eq!(stored.status, Status::Exited);
}

#[test]
fn ps_hides_created_unless_all() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    add_record(
        root,
        "000000000007",
        "new-container",
        Status::Created,
        None,
        at(0),
    );

    let (mut context, output) = context_for(root);
    ps::execute(
        PsArgs { all: false },
        &mut context,
        &FakeProbe::dead(),
        at(100),
    )
    .unwrap();
    assert_eq!(
        output.lines(),
        vec![String::from("ID NAME IMAGE STATUS CREATED")]
    );

    let (mut context, output) = context_for(root);
    ps::execute(
        PsArgs { all: true },
        &mut context,
        &FakeProbe::dead(),
        at(100),
    )
    .unwrap();
    assert!(output.text().contains("new-container"));
}

#[test]
fn ps_prints_only_the_header_on_a_fresh_root() {
    let dir = tempfile::tempdir().unwrap();
    let (mut context, output) = context_for(dir.path());

    ps::execute(
        PsArgs { all: false },
        &mut context,
        &FakeProbe::dead(),
        at(0),
    )
    .unwrap();

    assert_eq!(output.text(), "ID NAME IMAGE STATUS CREATED\n");
}

#[test]
fn ps_does_not_connect_to_live_containers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let record = add_record(
        root,
        "000000000008",
        "live-container",
        Status::Running,
        Some(2000),
        at(0),
    );

    let endpoint = record.control_endpoint().unwrap().clone();
    let connected = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&connected);
    let listener = dosker_platform::control::bind(&Endpoint::from(endpoint)).unwrap();
    std::thread::spawn(move || {
        if let Ok(Some(_connection)) = listener.accept(Duration::from_millis(500)) {
            flag.store(true, Ordering::SeqCst);
        }
    });

    let (mut context, output) = context_for(root);
    ps::execute(
        PsArgs { all: false },
        &mut context,
        &FakeProbe::live(2000),
        at(100),
    )
    .unwrap();

    std::thread::sleep(Duration::from_millis(100));
    assert!(!connected.load(Ordering::SeqCst));
    assert_eq!(row_fields(&output.lines()[1])[3], "running");
}

#[test]
fn ps_lists_a_created_container_with_all() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let record = add_record(
        root,
        "000000000009",
        "created-one",
        Status::Created,
        None,
        at(0),
    );

    let (mut context, output) = context_for(root);
    ps::execute(
        PsArgs { all: true },
        &mut context,
        &FakeProbe::dead(),
        at(100),
    )
    .unwrap();

    let lines = output.lines();
    assert_eq!(lines[0], "ID NAME IMAGE STATUS CREATED");
    assert_eq!(
        row_fields(&lines[1]),
        vec![
            record.id.as_str(),
            "created-one",
            "freedos:latest",
            "created",
            "2026-01-01T00:00:00Z"
        ]
    );
}

#[test]
fn ps_header_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    add_record(
        root,
        "000000000010",
        "header-check",
        Status::Created,
        None,
        at(0),
    );

    let (mut context, output) = context_for(root);
    ps::execute(
        PsArgs { all: true },
        &mut context,
        &FakeProbe::dead(),
        at(100),
    )
    .unwrap();

    assert_eq!(output.lines()[0], "ID NAME IMAGE STATUS CREATED");
    assert_eq!(row_fields(&output.lines()[1]).len(), 5);
}
