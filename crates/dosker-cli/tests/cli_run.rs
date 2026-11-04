mod common;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use chrono::Utc;
use common::{make_disk_image, run_captured, stderr_text};
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_engine::DisplayMode;
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

        let root = Self { dir };
        root.import_image();
        root
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
        let mut command = self.command();
        command.args(args);
        run_captured(&mut command, Duration::from_secs(15))
    }

    fn import_image(&self) {
        let dir = tempfile::tempdir().expect("create an image source directory");
        let source = dir.path().join("disk.img");
        make_disk_image(&source);
        let output = self.run(&[
            "image",
            "import",
            source.to_str().expect("source path is utf-8"),
            "freedos:latest",
        ]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "stderr: {}",
            stderr_text(&output)
        );
    }

    fn import_booter(&self) -> String {
        let dir = tempfile::tempdir().expect("create a booter source directory");
        let source = dir.path().join("booter.img");
        std::fs::write(&source, b"not a filesystem").expect("write a booter blob");
        let output = self.run(&[
            "image",
            "import",
            "--booter",
            source.to_str().expect("source path is utf-8"),
            "booter-test:latest",
        ]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "stderr: {}",
            stderr_text(&output)
        );
        String::from("booter-test:latest")
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

fn spawn_with_stdin(root: &RunRoot, args: &[&str], stdin: &str) -> Output {
    let mut child = root
        .command()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the dosker binary");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("write scripted stdin");
    child
        .wait_with_output()
        .expect("wait for the dosker binary")
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
fn run_records_the_display_and_vnc_port() {
    let root = RunRoot::new();
    let id = run_ok(
        &root,
        &[
            "run",
            "-d",
            "--display",
            "vnc",
            "--vnc-port",
            "5901",
            "freedos:latest",
        ],
    );
    let id = ContainerId::parse(&id).expect("parse the id printed by run");

    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.display, DisplayMode::Vnc);
    assert_eq!(record.vnc_port, Some(5901));
}

#[test]
fn run_passes_the_boot_timeout_to_the_supervisor() {
    let root = RunRoot::new();
    let started = Instant::now();
    let output = root
        .command()
        .env("MOCK_READY_DELAY_MS", "2000")
        .args(["run", "-d", "--boot-timeout", "1", "freedos:latest"])
        .output()
        .expect("run the dosker binary");

    assert_eq!(output.status.code(), Some(1));
    assert!(stdout_text(&output).contains("failed"));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the boot timeout was not applied: {:?}",
        started.elapsed()
    );
}

#[test]
fn run_detached_prints_only_one_id_line() {
    let root = RunRoot::new();
    let output = root.run(&["run", "-d", "freedos:latest"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = stdout_text(&output);
    assert_eq!(text.lines().count(), 1, "output: {text}");
    ContainerId::parse(text.trim()).expect("parse the id printed by run");
}

#[test]
fn run_reports_a_boot_timeout_with_the_last_serial_line() {
    let root = RunRoot::new();
    let output = root
        .command()
        .env("MOCK_READY_DELAY_MS", "2000")
        .args(["run", "-d", "--boot-timeout", "1", "freedos:latest"])
        .output()
        .expect("run the dosker binary");

    assert_eq!(output.status.code(), Some(1));
    assert!(stdout_text(&output).contains("failed"));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("last serial line"), "stderr: {stderr}");
    assert!(stderr.contains("--boot-timeout"), "stderr: {stderr}");
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

    let records = root.store().list().expect("list the containers");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, Status::Failed);
    let id = records[0].id.as_str();
    let name = records[0].name.as_str();

    let stdout = stdout_text(&output);
    assert_eq!(stdout.trim(), format!("{name} failed"));
    assert!(!stdout.contains(id), "the failed start printed the id");

    let stderr = stderr_text(&output);
    assert!(stderr.contains("start failed"), "stderr: {stderr}");
}

#[test]
fn run_without_detach_attaches_and_leaves_the_container_running() {
    let root = RunRoot::new();
    let output = spawn_with_stdin(&root, &["run", "freedos:latest"], "ECHO HELLO\nexit\n");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("HELLO"));

    let records = root.store().list().expect("list the containers");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, Status::Running);
}

#[test]
fn run_without_detach_on_a_booter() {
    let root = RunRoot::new();
    let image = root.import_booter();
    let output = spawn_with_stdin(&root, &["run", image.as_str()], "exit\n");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let stdout = stdout_text(&output);
    assert!(
        stdout.contains("no agent"),
        "the informational line is missing: {stdout}"
    );

    let records = root.store().list().expect("list the containers");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, Status::Running);
    assert!(
        !stdout.contains(records[0].id.as_str()),
        "the non-detached run printed the id: {stdout}"
    );
}

#[test]
fn run_without_detach_detaches_on_exit() {
    let root = RunRoot::new();
    let output = spawn_with_stdin(&root, &["run", "freedos:latest"], "exit\n");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let stdout = stdout_text(&output);
    assert!(!stdout.contains("HELLO"));
    assert!(
        stdout.contains(r"C:\>"),
        "the attached prompt is missing: {stdout}"
    );

    let records = root.store().list().expect("list the containers");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, Status::Running);
    assert!(
        !stdout.contains(records[0].id.as_str()),
        "the non-detached run printed the id: {stdout}"
    );
}
