//! A fake engine adapter backed by the mock engine child process.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use dosker_engine::{
    Capability, CapabilitySet, EngineAdapter, EngineError, EngineSession, EngineSpec, ExecRequest,
    ExecResult,
};
use dosker_platform::process::ProcessSupervisor;
use serde_json::{Value, json};

const ENGINE_NAME: &str = "mock";
const ENGINE_VERSION: &str = "0.1.0";
const POP_EVERY: Duration = Duration::from_millis(10);

/// An [`EngineAdapter`] that launches the mock engine binary.
pub struct MockAdapter {
    engine_path: PathBuf,
    processes: Arc<dyn ProcessSupervisor>,
    child_env: Vec<(String, String)>,
}

impl MockAdapter {
    pub fn new(engine_path: PathBuf, processes: Arc<dyn ProcessSupervisor>) -> Self {
        Self {
            engine_path,
            processes,
            child_env: Vec::new(),
        }
    }

    pub fn with_child_env(mut self, env: Vec<(String, String)>) -> Self {
        self.child_env = env;
        self
    }
}

impl EngineAdapter for MockAdapter {
    fn name(&self) -> &str {
        ENGINE_NAME
    }

    fn version(&self) -> &str {
        ENGINE_VERSION
    }

    fn capabilities(&self) -> CapabilitySet {
        [Capability::Boot, Capability::Serial, Capability::Stop]
            .into_iter()
            .collect()
    }

    fn probe(&mut self) -> Result<(), EngineError> {
        if self.engine_path.is_file() {
            Ok(())
        } else {
            Err(EngineError::NotFound(self.engine_path.clone()))
        }
    }

    fn start(&mut self, spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError> {
        std::fs::create_dir_all(&spec.log_dir)?;
        let log_path = spec.log_dir.join("engine.log");
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;

        let mut command = Command::new(&self.engine_path);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        for (key, value) in &self.child_env {
            command.env(key, value);
        }
        self.processes
            .prepare_engine(&mut command)
            .map_err(|source| EngineError::Setup(source.to_string()))?;

        let mut child = command.spawn().map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                EngineError::NotFound(self.engine_path.clone())
            } else {
                EngineError::Io(source)
            }
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| EngineError::Setup(String::from("engine stdin is not piped")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| EngineError::Setup(String::from("engine stdout is not piped")))?;
        let pid = child.id();

        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if sender.send(value).is_err() {
                    break;
                }
            }
        });

        Ok(Box::new(MockSession {
            child,
            stdin,
            receiver,
            pid,
            next_id: 0,
            exit_code: None,
        }))
    }
}

struct MockSession {
    child: Child,
    stdin: ChildStdin,
    receiver: Receiver<Value>,
    pid: u32,
    next_id: u64,
    exit_code: Option<i32>,
}

impl MockSession {
    fn write_request(&mut self, value: &Value) -> Result<(), EngineError> {
        writeln!(self.stdin, "{value}").map_err(EngineError::Io)?;
        self.stdin.flush().map_err(EngineError::Io)
    }

    fn exited_error(&mut self) -> EngineError {
        let code = self
            .exit_code
            .or_else(|| self.child.try_wait().ok().flatten().map(exit_code));
        EngineError::Exited(code.unwrap_or(1))
    }

    fn record_exit(&mut self, status: ExitStatus) -> i32 {
        let code = exit_code(status);
        self.exit_code = Some(code);
        code
    }
}

impl EngineSession for MockSession {
    fn pid(&self) -> Option<u32> {
        Some(self.pid)
    }

    #[cfg(unix)]
    fn process_group(&self) -> Option<u32> {
        Some(self.pid)
    }

    #[cfg(windows)]
    fn process_group(&self) -> Option<u32> {
        None
    }

    fn ready(&mut self, timeout: Duration) -> Result<(), EngineError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(EngineError::ReadyTimeout(timeout));
            }
            match self.receiver.recv_timeout(remaining) {
                Ok(value) => {
                    if value.get("event").and_then(Value::as_str) == Some("ready") {
                        return Ok(());
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Err(EngineError::ReadyTimeout(timeout)),
                Err(RecvTimeoutError::Disconnected) => return Err(self.exited_error()),
            }
        }
    }

    fn exec(&mut self, request: &ExecRequest) -> Result<ExecResult, EngineError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write_request(&json!({
            "op": "exec",
            "id": id,
            "command": request.command,
            "workdir": request.workdir,
            "timeout_ms": request.timeout_ms,
        }))?;

        let timeout = Duration::from_millis(request.timeout_ms);
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(EngineError::ExecTimeout(timeout));
            }
            match self.receiver.recv_timeout(remaining) {
                Ok(value) => {
                    if value.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    let exit_code =
                        value.get("exit_code").and_then(Value::as_i64).unwrap_or(0) as i32;
                    let output = value
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    return Ok(ExecResult {
                        exit_code,
                        output,
                        timed_out: false,
                        truncated: false,
                    });
                }
                Err(RecvTimeoutError::Timeout) => return Err(EngineError::ExecTimeout(timeout)),
                Err(RecvTimeoutError::Disconnected) => return Err(self.exited_error()),
            }
        }
    }

    fn stop(&mut self, grace: Duration) -> Result<i32, EngineError> {
        self.write_request(&json!({"op": "stop"}))?;

        let deadline = Instant::now() + grace;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(self.record_exit(status));
            }
            if Instant::now() >= deadline {
                return Err(EngineError::StopTimeout(grace));
            }
            std::thread::sleep(POP_EVERY);
        }
    }

    fn kill(&mut self) -> Result<i32, EngineError> {
        if let Some(code) = self.exit_code {
            return Ok(code);
        }
        let _ = self.child.kill();
        let status = self.child.wait()?;
        Ok(self.record_exit(status))
    }

    fn try_exit(&mut self) -> Result<Option<i32>, EngineError> {
        if let Some(code) = self.exit_code {
            return Ok(Some(code));
        }
        match self.child.try_wait()? {
            Some(status) => Ok(Some(self.record_exit(status))),
            None => Ok(None),
        }
    }
}

impl Drop for MockSession {
    fn drop(&mut self) {
        if self.exit_code.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(unix)]
fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(137)
}

#[cfg(windows)]
fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}
