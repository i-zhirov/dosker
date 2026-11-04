//! End-to-end checks that drive the real `dosker` binary through a detached
//! container lifecycle.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use common::{run_captured, stderr_text, stdout_text};
use dosker_core::{ContainerId, Status};
#[cfg(unix)]
use dosker_platform::EndpointRecord;
use dosker_platform::process::{ProcessSupervisor, system};
use dosker_runtime::StateStore;
#[cfg(unix)]
use dosker_test_support::fixtures::wait_until;
use dosker_test_support::fixtures::{kill_process, mock_engine_path};

#[cfg(unix)]
const STOP_WAIT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const POLL_STEP: Duration = Duration::from_millis(20);

fn temp_root() -> tempfile::TempDir {
    #[cfg(unix)]
    {
        tempfile::Builder::new()
            .prefix("ds")
            .tempdir_in("/tmp")
            .expect("create a temporary directory")
    }
    #[cfg(windows)]
    {
        tempfile::tempdir().expect("create a temporary directory")
    }
}

/// A data root plus the environment each `dosker` child receives.
struct Root {
    _dirs: Vec<tempfile::TempDir>,
    path: PathBuf,
    env: Vec<(&'static str, PathBuf)>,
}

impl Root {
    fn short() -> Self {
        let dir = temp_root();
        Self {
            path: dir.path().to_path_buf(),
            _dirs: vec![dir],
            env: Vec::new(),
        }
    }

    #[cfg(unix)]
    fn long() -> Self {
        let dir = temp_root();
        let path = dir.path().join("x".repeat(200));
        fs::create_dir(&path).expect("create the long root directory");
        Self {
            path,
            _dirs: vec![dir],
            env: Vec::new(),
        }
    }

    fn with_env(mut self, key: &'static str, value: &Path) -> Self {
        self.env.push((key, value.to_path_buf()));
        self
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn store(&self) -> StateStore {
        StateStore::new(self.path.clone())
    }

    fn command(&self) -> Command {
        self.command_with_root(&self.path)
    }

    fn command_with_root(&self, root: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dosker"));
        command.env("DOSKER_ROOT", root);
        command.env(
            "DOSKER_MOCK_ENGINE",
            mock_engine_path().expect("locate the mock engine"),
        );
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = self.command();
        command.args(args);
        run_captured(&mut command, Duration::from_secs(45))
    }

    fn run_with_root(&self, root: &Path, args: &[&str]) -> Output {
        let mut command = self.command_with_root(root);
        command.args(args);
        run_captured(&mut command, Duration::from_secs(45))
    }

    fn start(&mut self) -> ContainerId {
        self.import_image();
        let output = self.run(&["run", "-d", "freedos:latest"]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "stderr: {}",
            stderr_text(&output)
        );
        let text = stdout_text(&output);
        ContainerId::parse(text.trim()).expect("parse the id printed by run")
    }

    fn import_image(&self) {
        let dir = tempfile::tempdir().expect("create an image source directory");
        let source = dir.path().join("disk.img");
        common::make_disk_image(&source);
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
}

impl Drop for Root {
    fn drop(&mut self) {
        let processes = system();
        for record in self.store().list().unwrap_or_default() {
            let stopped = self.run(&["stop", record.id.as_str()]).status.success();
            if stopped {
                continue;
            }

            let Ok(current) = self.store().load(&record.id) else {
                continue;
            };
            if let Some(pgid) = current.engine_pgid {
                let _ = processes.kill_group(pgid);
            }
            if let Some(pid) = current.engine_pid {
                kill_process(pid);
            }
            if let Some(pid) = current.pid {
                kill_process(pid);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn rows(output: &Output) -> Vec<Vec<String>> {
    stdout_text(output)
        .lines()
        .skip(1)
        .map(|line| line.split_whitespace().map(str::to_owned).collect())
        .collect()
}

fn row_for<'a>(rows: &'a [Vec<String>], id: &ContainerId) -> &'a [String] {
    rows.iter()
        .find(|row| row.first().map(String::as_str) == Some(id.as_str()))
        .unwrap_or_else(|| panic!("no row for {id}"))
}

fn last_field(output: &Output) -> String {
    stdout_text(output)
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn detached_container_answers_exec_status_and_stop() {
    let mut root = Root::short();
    let id = root.start();

    let record = root.store().load(&id).expect("load the container record");
    assert_eq!(record.status, Status::Running);

    let exec = root.run(&["exec", id.as_str(), "ECHO", "HELLO"]);
    assert_eq!(
        exec.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&exec)
    );
    assert_eq!(stdout_text(&exec).trim(), "HELLO");

    let status = root.run(&["status", id.as_str()]);
    assert_eq!(
        status.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&status)
    );
    let status_line = stdout_text(&status);
    let fields: Vec<&str> = status_line.split_whitespace().collect();
    assert_eq!(fields.get(2), Some(&"running"), "status: {status_line}");

    let stop = root.run(&["stop", id.as_str()]);
    assert_eq!(
        stop.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&stop)
    );
    assert_eq!(last_field(&stop), "stopped");

    let ps = root.run(&["ps", "-a"]);
    assert_eq!(ps.status.code(), Some(0), "stderr: {}", stderr_text(&ps));
    let rows = rows(&ps);
    let row = row_for(&rows, &id);
    assert_eq!(row.get(3).map(String::as_str), Some("stopped"));
}

#[cfg(unix)]
#[test]
fn killing_the_supervisor_leaves_no_engine_and_ps_reports_exited() {
    let mut root = Root::short();
    let id = root.start();

    let record = root.store().load(&id).expect("load the container record");
    let supervisor_pid = record.pid.expect("the supervisor pid is recorded");
    let engine_pid = record.engine_pid.expect("the engine pid is recorded");

    assert!(kill_process(supervisor_pid), "kill the supervisor");
    assert!(
        wait_until(STOP_WAIT, POLL_STEP, || !system()
            .process_alive(supervisor_pid)),
        "the supervisor did not exit"
    );

    let ps = root.run(&["ps", "-a"]);
    assert_eq!(ps.status.code(), Some(0), "stderr: {}", stderr_text(&ps));
    let rows = rows(&ps);
    let row = row_for(&rows, &id);
    assert_eq!(row.get(3).map(String::as_str), Some("exited"));

    assert!(
        wait_until(STOP_WAIT, POLL_STEP, || !system().process_alive(engine_pid)),
        "the engine pid {engine_pid} is still alive"
    );

    let recovered = root.store().load(&id).expect("load the recovered record");
    assert_eq!(recovered.status, Status::Exited);
    assert_eq!(recovered.pid, None);
    assert_eq!(recovered.engine_pid, None);
    assert_eq!(recovered.engine_pgid, None);
    assert!(recovered.dirty);
}

#[test]
fn dosker_root_redirects_all_state() {
    let other = temp_root();
    let root = Root::short();
    #[cfg(unix)]
    let mut root = root
        .with_env("XDG_DATA_HOME", other.path())
        .with_env("HOME", other.path());
    #[cfg(windows)]
    let mut root = root.with_env("LOCALAPPDATA", other.path());

    let id = root.start();

    let container_dir = root.path().join("containers").join(id.as_str());
    for name in ["state.json", "engine.log", "serial.log", "container.log"] {
        let path = container_dir.join(name);
        assert!(path.is_file(), "missing {}", path.display());
    }
    #[cfg(unix)]
    assert!(
        container_dir.join("control.sock").exists(),
        "missing control socket"
    );

    let entries = fs::read_dir(other.path()).expect("read the other root");
    assert_eq!(
        entries.count(),
        0,
        "the host data root received state despite DOSKER_ROOT"
    );
}

#[cfg(unix)]
#[test]
fn long_dosker_root_starts_and_stops_through_the_fallback() {
    let mut root = Root::long();
    let id = root.start();

    let exec = root.run(&["exec", id.as_str(), "ECHO", "HELLO"]);
    assert_eq!(
        exec.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&exec)
    );
    assert_eq!(stdout_text(&exec).trim(), "HELLO");

    let record = root.store().load(&id).expect("load the container record");
    let EndpointRecord::UnixSocket { path } = record
        .control_endpoint()
        .expect("the record has a control endpoint")
    else {
        panic!("expected a Unix socket endpoint");
    };
    assert!(
        path.starts_with(dosker_platform::short_socket_dir()),
        "endpoint {} is not under the fallback directory",
        path.display()
    );
    use std::os::unix::ffi::OsStrExt;
    assert!(path.as_os_str().as_bytes().len() <= dosker_platform::MAX_UNIX_SOCKET_PATH_BYTES);

    let stop = root.run(&["stop", id.as_str()]);
    assert_eq!(
        stop.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&stop)
    );
    assert_eq!(last_field(&stop), "stopped");
    assert!(
        wait_until(STOP_WAIT, POLL_STEP, || !path.exists()),
        "the fallback socket {} is still present",
        path.display()
    );
}

#[test]
fn status_reads_state_from_the_dosker_root() {
    let mut root = Root::short();
    let id = root.start();

    let other = temp_root();
    let status = root.run_with_root(other.path(), &["status", id.as_str()]);

    assert_eq!(
        status.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&status)
    );
    assert!(
        stderr_text(&status).contains("container not found"),
        "stderr: {}",
        stderr_text(&status)
    );
}

#[test]
fn exec_and_stop_on_an_absent_container_exit_one() {
    let root = Root::short();
    let absent = "absent-container";

    let exec = root.run(&["exec", absent, "ECHO", "HELLO"]);
    assert_eq!(
        exec.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&exec)
    );
    assert!(
        stderr_text(&exec).contains(absent),
        "stderr: {}",
        stderr_text(&exec)
    );

    let stop = root.run(&["stop", absent]);
    assert_eq!(
        stop.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&stop)
    );
    assert!(
        stderr_text(&stop).contains(absent),
        "stderr: {}",
        stderr_text(&stop)
    );
}
