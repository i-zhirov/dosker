//! Shared helpers for tests that drive the real `dosker` binary.

#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use dosker_core::ContainerId;
use dosker_platform::process::{ProcessSupervisor, system};
use dosker_runtime::StateStore;
use dosker_test_support::fixtures::{kill_process, mock_engine_path};

/// A temporary DOSKER_ROOT with a helper to run the CLI against it.
pub struct CliRoot {
    dir: tempfile::TempDir,
}

impl CliRoot {
    pub fn new() -> Self {
        #[cfg(unix)]
        let dir = tempfile::Builder::new()
            .prefix("ds")
            .tempdir_in("/tmp")
            .expect("create a temporary directory");
        #[cfg(windows)]
        let dir = tempfile::tempdir().expect("create a temporary directory");

        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn store(&self) -> StateStore {
        StateStore::new(self.path().to_path_buf())
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dosker"));
        command.env("DOSKER_ROOT", self.path());
        command.env(
            "DOSKER_MOCK_ENGINE",
            mock_engine_path().expect("locate the mock engine"),
        );
        command
    }

    pub fn run(&self, args: &[&str]) -> Output {
        let mut command = self.command();
        command.args(args);
        run_captured(&mut command, Duration::from_secs(45))
    }

    /// Starts a detached container and returns its id.
    pub fn start(&self) -> ContainerId {
        let mut command = self.command();
        command.args(["run", "-d", "freedos:latest"]);
        let output = run_captured(&mut command, Duration::from_secs(15));
        assert_eq!(
            output.status.code(),
            Some(0),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let id = stdout_text(&output);
        ContainerId::parse(id.trim()).expect("parse the id printed by run")
    }

    /// Returns the engine PID recorded for a container.
    pub fn engine_pid(&self, id: &ContainerId) -> Option<u32> {
        self.store()
            .load(id)
            .expect("load the container record")
            .engine_pid
    }

    /// Kills the supervisor of a container without stopping its engine.
    pub fn kill_supervisor(&self, id: &ContainerId) {
        let record = self.store().load(id).expect("load the container record");
        if let Some(pid) = record.pid {
            kill_process(pid);
        }
    }

    /// Kills a process if it is still running.
    pub fn kill(&self, pid: Option<u32>) {
        if let Some(pid) = pid {
            kill_process(pid);
        }
    }
}

impl Drop for CliRoot {
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

/// Runs a command with file-backed stdout/stderr and a hard timeout.
///
/// A detached child must not inherit the capture pipes; files avoid a hang
/// when the supervisor outlives the `run` process.
pub fn run_captured(command: &mut Command, timeout: Duration) -> Output {
    let dir = tempfile::tempdir().expect("create a capture directory");
    let out_path = dir.path().join("stdout");
    let err_path = dir.path().join("stderr");
    let stdout = fs::File::create(&out_path).expect("create stdout capture");
    let stderr = fs::File::create(&err_path).expect("create stderr capture");

    let mut child = command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn the dosker binary");

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the dosker binary") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let stdout = fs::read(&out_path).unwrap_or_default();
            let stderr = fs::read(&err_path).unwrap_or_default();
            panic!(
                "command timed out after {timeout:?}; stdout: {}; stderr: {}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let stdout = fs::read(&out_path).expect("read stdout capture");
    let stderr = fs::read(&err_path).expect("read stderr capture");
    Output {
        status,
        stdout,
        stderr,
    }
}

pub fn stdout_text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is utf-8")
}

pub fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
