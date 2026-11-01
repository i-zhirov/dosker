use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use chrono::Utc;
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_platform::EndpointRecord;
use dosker_platform::endpoint::control_endpoint;
use dosker_platform::process::{ProcessSupervisor, system};
use dosker_runtime::{ContainerRecord, StateStore};
use dosker_test_support::fixtures::{kill_process, mock_engine_path};

struct RunRoot {
    dir: tempfile::TempDir,
}

impl RunRoot {
    fn new() -> Self {
        #[cfg(unix)]
        let dir = tempfile::Builder::new()
            .prefix("ds")
            .tempdir_in("/tmp")
            .expect("create a temporary directory");
        #[cfg(windows)]
        let dir = tempfile::tempdir().expect("create a temporary directory");

        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn store(&self) -> StateStore {
        StateStore::new(self.path().to_path_buf())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dosker"));
        command.env("DOSKER_ROOT", self.path());
        command.env(
            "DOSKER_MOCK_ENGINE",
            mock_engine_path().expect("locate the mock engine"),
        );
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .output()
            .expect("run the dosker binary")
    }
}

impl Drop for RunRoot {
    fn drop(&mut self) {
        let Ok(records) = self.store().list() else {
            return;
        };
        let processes = system();
        for record in records {
            if let Some(pgid) = record.engine_pgid {
                let _ = processes.kill_group(pgid);
            }
            if let Some(pid) = record.engine_pid {
                kill_process(pid);
            }
            if let Some(pid) = record.pid {
                kill_process(pid);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn seed_named_record(root: &RunRoot, id: &str, name: &str) {
    let id = ContainerId::parse(id).expect("parse the seed id");
    let name = ContainerName::parse(name).expect("parse the seed name");
    let endpoint = control_endpoint(root.path(), &id);
    let record = ContainerRecord::new(
        id,
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        Utc::now(),
    );
    root.store()
        .create(&record)
        .expect("create the seed record");
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is utf-8")
}

fn run_ok(root: &RunRoot, args: &[&str]) -> String {
    let output = root.run(args);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_text(&output).trim().to_owned()
}

#[test]
fn run_requires_detach() {
    let root = RunRoot::new();
    let output = root.run(&["run", "freedos:latest"]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn run_rejects_an_unknown_engine() {
    let root = RunRoot::new();
    let output = root.run(&["run", "-d", "--engine", "nope", "freedos:latest"]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn run_rejects_a_duplicate_name() {
    let root = RunRoot::new();
    seed_named_record(&root, "000000000013", "duplicate");

    let output = root.run(&["run", "-d", "--name", "duplicate", "freedos:latest"]);

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn run_prints_a_twelve_hex_id() {
    let root = RunRoot::new();
    let output = root.run(&["run", "-d", "freedos:latest"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = stdout_text(&output);
    let id = text.trim();
    assert_eq!(text.lines().count(), 1);
    assert_eq!(id.len(), 12);
    assert!(
        id.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    );
}

#[test]
fn run_generates_a_name_when_omitted() {
    let root = RunRoot::new();
    let id = run_ok(&root, &["run", "-d", "freedos:latest"]);
    let id = ContainerId::parse(&id).expect("parse the id printed by run");

    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(
        record.name.as_str(),
        format!("dosker-{}", &id.as_str()[..8])
    );
}

#[test]
fn run_records_the_control_endpoint_under_the_root() {
    let root = RunRoot::new();
    let id = run_ok(&root, &["run", "-d", "freedos:latest"]);
    let id = ContainerId::parse(&id).expect("parse the id printed by run");

    let record = root.store().load(&id).expect("load the container record");
    match record
        .control_endpoint()
        .expect("recorded control endpoint")
    {
        EndpointRecord::UnixSocket { path } => assert!(path.starts_with(root.path())),
        EndpointRecord::NamedPipe { name } => {
            assert_eq!(name, &format!(r"\\.\pipe\dosker-{id}-control"));
        }
    }
}

#[test]
fn run_returns_after_the_container_is_running() {
    let root = RunRoot::new();
    let id = run_ok(&root, &["run", "-d", "freedos:latest"]);
    let id = ContainerId::parse(&id).expect("parse the id printed by run");

    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Running);
    let pid = record.pid.expect("the supervisor pid is recorded");
    assert!(system().process_alive(pid));
}

#[test]
fn run_reports_a_failed_start() {
    let root = RunRoot::new();
    let output = root
        .command()
        .env("MOCK_FAIL_START", "1")
        .args(["run", "-d", "freedos:latest"])
        .output()
        .expect("run the dosker binary");

    assert_eq!(output.status.code(), Some(1));
    assert!(stdout_text(&output).contains("failed"));

    let records = root.store().list().expect("list the containers");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, Status::Failed);
}
