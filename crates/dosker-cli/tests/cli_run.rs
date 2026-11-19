mod common;

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use common::{make_disk_image, run_captured, stderr_text};
use dosker_core::{ContainerId, ContainerName, Status};
use dosker_engine::DisplayMode;
use dosker_image::PortProtocol;
use dosker_platform::EndpointRecord;
use dosker_platform::endpoint::control_endpoint;
use dosker_platform::process::{ProcessSupervisor, system};
use dosker_runtime::{ContainerRecord, PortMapping, StateStore};
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
        let agent_dir = root.path().join("agent");
        std::fs::create_dir_all(&agent_dir).expect("create the agent directory");
        std::fs::write(agent_dir.join("DOSKER.EXE"), b"MZ cli fake agent")
            .expect("write the fake agent");
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
        let mut command = self.command();
        command.env("DOSKER_AGENT_EXE", self.path().join("agent/DOSKER.EXE"));
        command.args([
            "image",
            "import",
            source.to_str().expect("source path is utf-8"),
            "freedos:latest",
        ]);
        let output = run_captured(&mut command, Duration::from_secs(15));
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

const CHILD_TIMEOUT: Duration = Duration::from_secs(30);

fn wait_with_timeout(mut child: Child, timeout: Duration) -> ExitStatus {
    let pid = child.id();
    let finished = Arc::new(AtomicBool::new(false));
    let watched = Arc::clone(&finished);
    let watchdog = thread::spawn(move || {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if watched.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        if !watched.load(Ordering::SeqCst) {
            eprintln!("dosker child {pid} did not exit within {timeout:?}; killing it");
            kill_process(pid);
        }
    });

    let status = child.wait().expect("wait for the dosker binary");
    finished.store(true, Ordering::SeqCst);
    watchdog.join().expect("join the child watchdog");
    status
}

fn spawn_with_stdin(root: &RunRoot, args: &[&str], stdin: &str) -> Output {
    let capture = tempfile::tempdir().expect("create an output directory");
    let stdout_path = capture.path().join("stdout.log");
    let stderr_path = capture.path().join("stderr.log");
    let stdout = File::create(&stdout_path).expect("create stdout capture");
    let stderr = File::create(&stderr_path).expect("create stderr capture");

    let mut child = root
        .command()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn the dosker binary");
    let mut input = child.stdin.take().expect("stdin is piped");
    input
        .write_all(stdin.as_bytes())
        .expect("write scripted stdin");
    drop(input);

    let status = wait_with_timeout(child, CHILD_TIMEOUT);
    let stdout = std::fs::read(stdout_path).expect("read captured stdout");
    let stderr = std::fs::read(stderr_path).expect("read captured stderr");
    Output {
        status,
        stdout,
        stderr,
    }
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
        EndpointRecord::Tcp { host, port } => {
            panic!("a control endpoint is never tcp: {host}:{port}");
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
fn a_second_container_cannot_publish_a_taken_host_port() {
    let root = RunRoot::new();
    let port = dosker_engine_qemu::command::free_tcp_port().expect("allocate a free port");
    let mapping = format!("{port}:80");
    let first = run_ok(&root, &["run", "-d", "-p", &mapping, "freedos:latest"]);
    let first_id = ContainerId::parse(&first).expect("parse the first id");

    let output = root.run(&["run", "-d", "-p", &mapping, "freedos:latest"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_text(&output).contains("already published"),
        "stderr: {}",
        stderr_text(&output)
    );

    let inspected = root.run(&["inspect", &first]);
    assert_eq!(
        inspected.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&inspected)
    );
    let value: serde_json::Value =
        serde_json::from_str(&stdout_text(&inspected)).expect("parse the inspect output");
    assert_eq!(value["status"], "running");

    let records = root.store().list().expect("list the containers");
    let failed = records
        .iter()
        .find(|record| record.id != first_id)
        .expect("the failed container is recorded");
    assert_eq!(
        failed.status,
        Status::Created,
        "the conflict does not transition the record"
    );
    let log = std::fs::read_to_string(
        root.path()
            .join("containers")
            .join(failed.id.as_str())
            .join("container.log"),
    )
    .expect("read the failed container log");
    assert!(
        log.contains("already published"),
        "the conflict is logged: {log:?}"
    );
    let rm = root.run(&["rm", failed.id.as_str()]);
    assert_eq!(rm.status.code(), Some(0), "stderr: {}", stderr_text(&rm));
}

#[test]
fn a_stale_container_does_not_hold_a_port() {
    let root = RunRoot::new();
    let port = dosker_engine_qemu::command::free_tcp_port().expect("allocate a free port");
    let id = ContainerId::parse("000000000021").expect("parse the stale id");
    let name = ContainerName::parse("stale").expect("parse the stale name");
    let endpoint = control_endpoint(root.path(), &id);
    let mut record = ContainerRecord::new(
        id.clone(),
        name,
        String::from("freedos:latest"),
        String::from("mock"),
        endpoint.record(),
        Utc::now(),
    )
    .with_network(
        String::from("ne2000"),
        vec![PortMapping {
            host_address: std::net::Ipv4Addr::LOCALHOST,
            host_port: port,
            guest_port: 80,
            protocol: PortProtocol::Tcp,
        }],
    );
    record.status = Status::Running;
    record.pid = Some(u32::MAX);
    root.store()
        .create(&record)
        .expect("create the stale record");

    let mapping = format!("{port}:80");
    let output = root.run(&["run", "-d", "-p", &mapping, "freedos:latest"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let stale = root.store().load(&id).expect("load the stale record");
    assert_eq!(stale.status, Status::Exited);
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
