//! A running QEMU process and its host endpoints.

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use dosker_agent_proto::{
    AgentClient, ExecRequest as ProtoExecRequest, MAX_OUTPUT_BYTES, ProtoError,
};
use dosker_engine::{
    DebugKind, EngineCommand, EngineEndpoint, EngineError, EngineSession, EngineSpec, ExecRequest,
    ExecResult,
};
use dosker_platform::process::ProcessSupervisor;

use crate::discovery::{QemuBinary, QemuVersion};
use crate::endpoints::QemuEndpoints;
use crate::qmp::{QMP_CONNECT_TIMEOUT, QmpClient};
use crate::serial_log::SerialLog;
use crate::stream::EngineStream;

const BOOTER_ALIVE_PROBE: Duration = Duration::from_millis(250);
const AGENT_SOCKET_RETRY: Duration = Duration::from_millis(50);
const AGENT_CONNECT_ATTEMPT: Duration = Duration::from_millis(100);
const DEBUG_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const STOP_POLL: Duration = Duration::from_millis(20);
const SCREENSHOT_WAIT: Duration = Duration::from_secs(5);
const SCREENSHOT_POLL: Duration = Duration::from_millis(20);
const SCREENDUMP_FORMAT_VERSION: QemuVersion = QemuVersion {
    major: 7,
    minor: 1,
    patch: 0,
};

/// A QEMU process attached to one container disk.
pub struct QemuSession {
    child: Child,
    binary: QemuBinary,
    spec: EngineSpec,
    command: EngineCommand,
    endpoints: QemuEndpoints,
    agent: Option<AgentClient<SerialLog<EngineStream>>>,
    qmp: Option<QmpClient>,
    exit_code: Option<i32>,
}

impl QemuSession {
    /// Starts QEMU for a spec and a prepared command line.
    pub fn spawn(
        binary: &QemuBinary,
        spec: &EngineSpec,
        processes: &dyn ProcessSupervisor,
        command: EngineCommand,
    ) -> Result<Self, EngineError> {
        fs::create_dir_all(&spec.log_dir)?;
        let endpoints = QemuEndpoints::from_command(&command)?;
        endpoints.remove_stale()?;

        let log_path = spec.log_dir.join("engine.log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        let log_err = log.try_clone()?;

        let mut process = Command::new(&command.program);
        process
            .args(&command.args)
            .envs(command.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));
        processes.prepare_engine(&mut process).map_err(|error| {
            EngineError::Setup(format!("could not prepare the QEMU process: {error}"))
        })?;

        let child = process.spawn()?;

        Ok(Self {
            child,
            binary: binary.clone(),
            spec: spec.clone(),
            command,
            endpoints,
            agent: None,
            qmp: None,
            exit_code: None,
        })
    }

    fn qmp(&mut self) -> Result<&mut QmpClient, EngineError> {
        self.qmp_with_deadline(Instant::now() + QMP_CONNECT_TIMEOUT)
    }

    /// Connects lazily, keeping the QMP handshake before `deadline`.
    ///
    /// A caller with an existing overall deadline (such as `stop`) uses this
    /// instead of the fixed `QMP_CONNECT_TIMEOUT`, so a QMP server that
    /// accepts but never greets cannot extend the caller's budget.
    fn qmp_with_deadline(&mut self, deadline: Instant) -> Result<&mut QmpClient, EngineError> {
        if self.qmp.is_none() {
            if Instant::now() >= deadline {
                return Err(EngineError::Protocol(
                    "the QMP deadline passed before a connection".to_owned(),
                ));
            }

            let qmp_endpoint = self.endpoints.qmp.clone();
            self.qmp = Some(QmpClient::connect_until(&qmp_endpoint, deadline)?);
        }

        self.qmp
            .as_mut()
            .ok_or_else(|| EngineError::Protocol("the QMP client is missing".to_owned()))
    }

    /// Sends `quit` with no more than the time left before `deadline`.
    fn quit_with_deadline(&mut self, deadline: Instant) -> bool {
        if Instant::now() >= deadline {
            return false;
        }

        let Ok(client) = self.qmp_with_deadline(deadline) else {
            return false;
        };

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || client.set_io_timeout(remaining).is_err() {
            return false;
        }

        client.quit().is_ok()
    }

    fn poll_exit(&mut self) -> Result<Option<i32>, EngineError> {
        if let Some(code) = self.exit_code {
            return Ok(Some(code));
        }

        let Some(status) = self.child.try_wait()? else {
            return Ok(None);
        };

        let code = status_code(status);
        self.exit_code = Some(code);
        Ok(Some(code))
    }

    fn record_exit(&mut self, code: i32) -> std::result::Result<i32, EngineError> {
        self.exit_code = Some(code);
        Ok(code)
    }

    fn agent_mut(&mut self) -> Result<&mut AgentClient<SerialLog<EngineStream>>, EngineError> {
        if self.spec.booter {
            return Err(EngineError::NoAgent);
        }

        self.agent.as_mut().ok_or(EngineError::NoAgent)
    }
}

impl Drop for QemuSession {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl EngineSession for QemuSession {
    fn pid(&self) -> Option<u32> {
        Some(self.child.id())
    }

    #[cfg(unix)]
    fn process_group(&self) -> Option<u32> {
        Some(self.child.id())
    }

    #[cfg(windows)]
    fn process_group(&self) -> Option<u32> {
        None
    }

    fn ready(&mut self, timeout: Duration) -> Result<(), EngineError> {
        if self.spec.booter {
            thread::sleep(BOOTER_ALIVE_PROBE);
            return match self.poll_exit()? {
                Some(code) => Err(EngineError::Exited(code)),
                None => Ok(()),
            };
        }

        let agent_serial = self
            .endpoints
            .agent_serial
            .as_ref()
            .cloned()
            .ok_or(EngineError::NoAgent)?;
        let deadline = Instant::now() + timeout;
        let stream = loop {
            if let Some(code) = self.poll_exit()? {
                return Err(EngineError::Exited(code));
            }

            if Instant::now() >= deadline {
                return Err(EngineError::ReadyTimeout(timeout));
            }

            let attempt_deadline = (Instant::now() + AGENT_CONNECT_ATTEMPT).min(deadline);
            match EngineStream::connect(&agent_serial, attempt_deadline) {
                Ok(stream) => break stream,
                Err(_) => {
                    if let Some(code) = self.poll_exit()? {
                        return Err(EngineError::Exited(code));
                    }

                    if Instant::now() >= deadline {
                        return Err(EngineError::ReadyTimeout(timeout));
                    }

                    thread::sleep(AGENT_SOCKET_RETRY);
                }
            }
        };

        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.spec.log_dir.join("serial.log"))?;
        let mut client = AgentClient::new(SerialLog::new(stream, log));
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(EngineError::ReadyTimeout(timeout));
        }

        match client.handshake(remaining) {
            Ok(_) => {
                self.agent = Some(client);
                Ok(())
            }
            Err(ProtoError::HandshakeTimeout) => Err(EngineError::ReadyTimeout(timeout)),
            Err(error) => Err(protocol_error(error)),
        }
    }

    fn exec(&mut self, request: &ExecRequest) -> Result<ExecResult, EngineError> {
        let timeout = Duration::from_millis(request.timeout_ms);
        let proto_request = ProtoExecRequest {
            command: request.command.clone(),
            env: request.env.clone(),
            workdir: request.workdir.clone(),
            max_output_bytes: u32::try_from(MAX_OUTPUT_BYTES).unwrap_or(u32::MAX),
        };
        let agent = self.agent_mut()?;
        let outcome = agent
            .exec(&proto_request, timeout)
            .map_err(|error| exec_error(error, timeout))?;

        Ok(ExecResult {
            exit_code: outcome.exit_code,
            output: String::from_utf8_lossy(&outcome.output).into_owned(),
            timed_out: false,
            truncated: outcome.truncated,
        })
    }

    fn stop(&mut self, grace: Duration) -> Result<i32, EngineError> {
        // The grace window covers the agent shutdown, the QMP quit, and the
        // wait for the process to exit. Capture it before any of those steps
        // so no step can extend the stop by another full grace period.
        let deadline = Instant::now() + grace;
        let mut acknowledged = false;

        if !self.spec.booter {
            let grace_ms = u64::try_from(grace.as_millis()).unwrap_or(u64::MAX);
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Some(agent) = self.agent.as_mut()
                && let Ok(ack) = agent.shutdown_with_timeout(grace_ms, remaining)
            {
                acknowledged = ack.ready;
            }
        }

        let quit_ok = self.quit_with_deadline(deadline);
        if self.spec.booter {
            acknowledged = quit_ok;
        }

        loop {
            if let Some(code) = self.poll_exit()? {
                if acknowledged {
                    return Ok(code);
                }

                // A booter whose QMP quit failed is treated conservatively:
                // an exit without that acknowledgement is reported as dirty
                // rather than as a clean stop.
                return Err(EngineError::Protocol(
                    "shutdown was not acknowledged".to_owned(),
                ));
            }

            if Instant::now() >= deadline {
                break;
            }

            thread::sleep(STOP_POLL);
        }

        let _ = self.child.kill();
        let _ = self.child.wait();
        Err(EngineError::StopTimeout(grace))
    }

    fn kill(&mut self) -> Result<i32, EngineError> {
        if let Some(code) = self.poll_exit()? {
            return self.record_exit(code);
        }

        let _ = self.child.kill();
        let status = self.child.wait()?;
        self.record_exit(status_code(status))
    }

    fn try_exit(&mut self) -> Result<Option<i32>, EngineError> {
        self.poll_exit()
    }

    fn command(&self) -> Option<&EngineCommand> {
        Some(&self.command)
    }

    fn endpoints(&self) -> Vec<EngineEndpoint> {
        self.endpoints.engine_endpoints()
    }

    fn screenshot(&mut self, destination: &Path) -> Result<(), EngineError> {
        let format = if self.binary.version >= SCREENDUMP_FORMAT_VERSION {
            Some("ppm")
        } else {
            None
        };

        let client = self.qmp()?;

        // Remove a stale file first so a previous screenshot cannot make the
        // wait below succeed before QEMU has written anything.
        match fs::remove_file(destination) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(EngineError::Io(error)),
        }

        client.screendump(destination, format)?;

        let deadline = Instant::now() + SCREENSHOT_WAIT;
        loop {
            if let Ok(metadata) = fs::metadata(destination)
                && metadata.len() > 0
            {
                return Ok(());
            }

            if Instant::now() >= deadline {
                return Err(EngineError::Protocol(
                    "the screenshot was not written".to_owned(),
                ));
            }

            thread::sleep(SCREENSHOT_POLL);
        }
    }

    fn debug_endpoint(&mut self, kind: DebugKind) -> Result<String, EngineError> {
        match kind {
            DebugKind::Gdb => {
                if EngineStream::probe(&self.endpoints.gdb, DEBUG_PROBE_TIMEOUT) {
                    Ok(self.endpoints.gdb.display())
                } else {
                    Err(EngineError::Unsupported("debugger"))
                }
            }
        }
    }

    fn put_file(&mut self, path: &str, data: &[u8]) -> Result<u64, EngineError> {
        let agent = self.agent_mut()?;
        let end = agent.put(path, data, true).map_err(transfer_error)?;

        Ok(end.bytes)
    }

    fn get_file(&mut self, path: &str) -> Result<Vec<u8>, EngineError> {
        let agent = self.agent_mut()?;
        agent.get(path).map_err(transfer_error)
    }
}

fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(if cfg!(unix) { 137 } else { -1 })
}

fn protocol_error(error: ProtoError) -> EngineError {
    match error {
        ProtoError::GuestRebooted => EngineError::GuestRebooted,
        ProtoError::Agent(payload) => EngineError::Agent {
            code: payload.code.as_str().to_owned(),
            message: payload.message,
        },
        ProtoError::Io(error) => EngineError::Io(error),
        other => EngineError::Protocol(other.to_string()),
    }
}

fn exec_error(error: ProtoError, timeout: Duration) -> EngineError {
    match error {
        ProtoError::Timeout => EngineError::ExecTimeout(timeout),
        other => protocol_error(other),
    }
}

fn transfer_error(error: ProtoError) -> EngineError {
    match error {
        too_large @ ProtoError::TransferTooLarge { .. } => EngineError::Agent {
            code: "TOO_LARGE".to_owned(),
            message: too_large.to_string(),
        },
        other => protocol_error(other),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use dosker_agent_proto::{
        AGENT_VERSION, Frame, FrameType, Hello, Incoming, ShutdownAck, encode_payload, read_frame,
        write_frame,
    };
    use dosker_core::ContainerId;
    use dosker_engine::{DisplayMode, EngineCommand, EngineSpec};

    use super::*;

    fn delayed_agent(delay: Duration) -> (AgentClient<SerialLog<EngineStream>>, tempfile::TempDir) {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.path().join("serial.log"))
            .expect("open serial log");
        let (host, mut agent) = UnixStream::pair().expect("create a socket pair");
        agent
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("set the agent read timeout");

        thread::spawn(move || {
            let hello = Hello {
                protocol_version: 1,
                agent_version: AGENT_VERSION.to_owned(),
                dos: "FreeDOS 1.3".to_owned(),
                memory_kb: 640,
                control_port: 2,
                max_payload: 4096,
                features: Vec::new(),
            };
            let frame = Frame::new(
                FrameType::Hello,
                1,
                0,
                encode_payload(&hello).expect("encode hello"),
            )
            .expect("build hello");
            if write_frame(&mut agent, &frame).is_err() {
                return;
            }

            let Ok(Incoming::Frame(reply)) = read_frame(&mut agent) else {
                return;
            };
            let ack = Frame::new(FrameType::Ack, reply.request_id, reply.sequence, Vec::new())
                .expect("build hello ack");
            if write_frame(&mut agent, &ack).is_err() {
                return;
            }

            let Ok(Incoming::Frame(shutdown)) = read_frame(&mut agent) else {
                return;
            };
            let ack = Frame::new(
                FrameType::Ack,
                shutdown.request_id,
                shutdown.sequence,
                Vec::new(),
            )
            .expect("build shutdown ack");
            if write_frame(&mut agent, &ack).is_err() {
                return;
            }

            thread::sleep(delay);
            let payload =
                encode_payload(&ShutdownAck { ready: true }).expect("encode shutdown ack");
            let ack = Frame::new(
                FrameType::ShutdownAck,
                shutdown.request_id,
                shutdown.sequence.wrapping_add(1),
                payload,
            )
            .expect("build shutdown ack");
            let _ = write_frame(&mut agent, &ack);
            let _ = read_frame(&mut agent);
        });

        let mut client = AgentClient::new(SerialLog::new(EngineStream::Unix(host), log));
        client
            .handshake(Duration::from_secs(1))
            .expect("handshake succeeds");

        (client, directory)
    }

    #[test]
    fn stop_uses_one_deadline_for_the_agent_and_the_exit_poll() {
        const GRACE: Duration = Duration::from_secs(1);

        let (agent, directory) = delayed_agent(Duration::from_secs(5));
        let spec = session_spec(directory.path());
        let mut session = session_with(Some(agent), spec);

        let started = Instant::now();
        match session.stop(GRACE) {
            Err(EngineError::StopTimeout(grace)) => assert_eq!(grace, GRACE),
            other => panic!("expected a stop timeout, got {other:?}"),
        }

        let elapsed = started.elapsed();
        assert!(
            elapsed >= GRACE,
            "stop returned before the grace: {elapsed:?}"
        );
        assert!(
            elapsed < GRACE * 2,
            "stop re-anchored its deadline: {elapsed:?}"
        );
    }

    #[test]
    fn stop_does_not_wait_for_a_stalled_qmp_greeting() {
        const GRACE: Duration = Duration::from_millis(300);

        let directory = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(directory.path());
        let listener = bind_stalled_qmp(&spec);

        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            // Accept the connection but never send the QMP greeting.
            let mut buffer = [0u8; 1];
            let _ = stream.read(&mut buffer);
        });

        let mut session = session_with(None, spec);

        let started = Instant::now();
        match session.stop(GRACE) {
            Err(EngineError::StopTimeout(grace)) => assert_eq!(grace, GRACE),
            other => panic!("expected a stop timeout, got {other:?}"),
        }
        assert_stopped_within_grace(started, GRACE);
    }

    #[test]
    fn stop_does_not_wait_for_a_stalled_qmp_quit() {
        const GRACE: Duration = Duration::from_millis(300);

        let directory = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(directory.path());
        let listener = bind_stalled_qmp(&spec);

        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let greeting =
                b"{\"QMP\":{\"version\":{\"qemu\":{\"major\":8,\"minor\":0,\"patch\":0}}}}\n";
            if stream.write_all(greeting).is_err() {
                return;
            }
            let _ = stream.flush();

            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            if reader.get_mut().write_all(b"{\"return\":{}}\n").is_err() {
                return;
            }
            let _ = reader.get_mut().flush();

            line.clear();
            let _ = reader.read_line(&mut line);
            // Read the quit command but never answer it.
            let mut buffer = [0u8; 1];
            let _ = reader.get_mut().read(&mut buffer);
        });

        let mut session = session_with(None, spec);

        let started = Instant::now();
        match session.stop(GRACE) {
            Err(EngineError::StopTimeout(grace)) => assert_eq!(grace, GRACE),
            other => panic!("expected a stop timeout, got {other:?}"),
        }
        assert_stopped_within_grace(started, GRACE);
    }

    #[test]
    fn debug_endpoint_reports_unsupported_for_a_missing_gdb_path() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(directory.path());
        let gdb = QemuEndpoints::for_spec(&spec)
            .expect("resolve the engine endpoints")
            .gdb
            .unix_path()
            .expect("the gdb endpoint is a Unix socket")
            .to_path_buf();
        let _ = fs::remove_file(&gdb);

        let mut session = session_with(None, spec);

        match session.debug_endpoint(DebugKind::Gdb) {
            Err(EngineError::Unsupported("debugger")) => {}
            other => panic!("expected an unsupported debugger, got {other:?}"),
        }
    }

    fn session_spec(root: &Path) -> EngineSpec {
        EngineSpec {
            root: root.to_path_buf(),
            container_id: ContainerId::generate(),
            workdir: "C:\\".to_owned(),
            env: BTreeMap::new(),
            cmd: None,
            expects_agent: true,
            log_dir: root.to_path_buf(),
            disk_path: root.join("disk.img"),
            booter: false,
            display: DisplayMode::None,
            vnc_port: None,
        }
    }

    fn session_with(
        agent: Option<AgentClient<SerialLog<EngineStream>>>,
        spec: EngineSpec,
    ) -> QemuSession {
        let child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("start sleep");
        let endpoints = QemuEndpoints::for_spec(&spec).expect("resolve the engine endpoints");
        let command = EngineCommand {
            program: PathBuf::from("qemu-system-i386"),
            args: Vec::new(),
            env: Vec::new(),
            endpoints: endpoints.engine_endpoints(),
        };

        QemuSession {
            child,
            binary: QemuBinary {
                path: PathBuf::from("qemu-system-i386"),
                version: QemuVersion::new(8, 0, 0),
            },
            command,
            endpoints,
            spec,
            agent,
            qmp: None,
            exit_code: None,
        }
    }

    fn bind_stalled_qmp(spec: &EngineSpec) -> UnixListener {
        let path = QemuEndpoints::for_spec(spec)
            .expect("resolve the engine endpoints")
            .qmp
            .unix_path()
            .expect("the QMP endpoint is a Unix socket")
            .to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the socket directory");
        }

        UnixListener::bind(&path).expect("bind the stalled QMP socket")
    }

    fn assert_stopped_within_grace(started: Instant, grace: Duration) {
        let elapsed = started.elapsed();
        assert!(
            elapsed >= grace,
            "stop returned before the grace: {elapsed:?}"
        );
        assert!(
            elapsed < grace + Duration::from_secs(1),
            "stop waited for the stalled QMP server instead of the grace: {elapsed:?}"
        );
    }
}
