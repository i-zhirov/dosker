//! A running DOSBox-X process and its host endpoints.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dosker_agent_proto::{
    AgentClient, ExecRequest as ProtoExecRequest, MAX_OUTPUT_BYTES, ProtoError,
};
use dosker_engine::{
    Capability, DebugKind, EngineCommand, EngineEndpoint, EngineEndpointKind, EngineError,
    EngineSession, EngineSpec, ExecRequest, ExecResult,
};
#[cfg(windows)]
use dosker_platform::process::JobObject;
use dosker_platform::process::ProcessSupervisor;

use crate::adapter::capabilities;
use crate::endpoints::DosboxXEndpoints;
use crate::serial_log::SerialLog;
use crate::stream::DosboxXStream;

/// How long a booter must stay alive before it is considered ready.
const BOOTER_ALIVE_PROBE: Duration = Duration::from_millis(250);
/// How long one connect attempt may take while the child is polled.
const READY_CONNECT_POLL: Duration = Duration::from_millis(100);
/// How often the stop wait polls for the process.
const STOP_POLL: Duration = Duration::from_millis(20);
/// How often the debug bridge checks its stop flag.
const DEBUG_BRIDGE_POLL: Duration = Duration::from_millis(20);
/// How long one debug bridge write may block.
const DEBUG_BRIDGE_WRITE: Duration = Duration::from_secs(1);
/// The debug bridge transfer buffer size.
const DEBUG_BRIDGE_BUFFER: usize = 8192;

/// Bridges the debugger client to the DOSBox-X MCP connection.
///
/// DOSBox-X connects out to the debug endpoint as a TCP client. The bridge
/// listens on that endpoint, accepts the DOSBox-X connection, then accepts one
/// debugger client and copies bytes in both directions. If DOSBox-X connects
/// again after a session ends, the bridge accepts it again.
struct DebugBridge {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl DebugBridge {
    fn start(kind: &EngineEndpointKind) -> Result<Self, EngineError> {
        let address = match kind {
            EngineEndpointKind::Tcp { host, port } => SocketAddr::from((*host, *port)),
            EngineEndpointKind::UnixSocket(_) => {
                return Err(EngineError::Setup(
                    "the DOSBox-X debugger endpoint must be TCP".to_owned(),
                ));
            }
        };

        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || bridge_loop(&listener, &thread_stop));

        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DebugBridge {
    fn drop(&mut self) {
        self.stop();
    }
}

fn bridge_loop(listener: &TcpListener, stop: &Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        let Some(engine) = accept_connection(listener, stop) else {
            return;
        };
        let Some(client) = accept_connection(listener, stop) else {
            return;
        };

        pump(engine, client, stop);
    }
}

fn accept_connection(listener: &TcpListener, stop: &Arc<AtomicBool>) -> Option<TcpStream> {
    loop {
        if stop.load(Ordering::SeqCst) {
            return None;
        }

        match listener.accept() {
            Ok((stream, _address)) => {
                // A connection that cannot be prepared is dropped; it must not
                // end the bridge.
                if prepare_stream(&stream).is_err() {
                    continue;
                }
                return Some(stream);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(DEBUG_BRIDGE_POLL);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            // Any other accept error is transient; keep serving until the
            // bridge is stopped.
            Err(_) => thread::sleep(DEBUG_BRIDGE_POLL),
        }
    }
}

fn prepare_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(DEBUG_BRIDGE_POLL))?;
    stream.set_write_timeout(Some(DEBUG_BRIDGE_WRITE))
}

fn pump(engine: TcpStream, client: TcpStream, stop: &Arc<AtomicBool>) {
    let pair_stop = Arc::new(AtomicBool::new(false));
    let (Ok(engine_reader), Ok(client_writer)) = (engine.try_clone(), client.try_clone()) else {
        return;
    };

    let forward_pair = Arc::clone(&pair_stop);
    let forward_stop = Arc::clone(stop);
    let forward = thread::spawn(move || {
        copy_stream(engine_reader, client_writer, &forward_pair, &forward_stop);
        forward_pair.store(true, Ordering::SeqCst);
    });

    copy_stream(client, engine, &pair_stop, stop);
    pair_stop.store(true, Ordering::SeqCst);
    let _ = forward.join();
}

fn copy_stream(
    mut reader: TcpStream,
    mut writer: TcpStream,
    pair_stop: &AtomicBool,
    global_stop: &AtomicBool,
) {
    let mut buffer = [0u8; DEBUG_BRIDGE_BUFFER];
    while !pair_stop.load(Ordering::SeqCst) && !global_stop.load(Ordering::SeqCst) {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if writer.write_all(&buffer[..read]).is_err() {
                    break;
                }
                let _ = writer.flush();
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(_) => break,
        }
    }
}

/// A DOSBox-X process attached to one container disk.
pub struct DosboxXSession {
    child: Child,
    spec: EngineSpec,
    command: EngineCommand,
    endpoints: DosboxXEndpoints,
    agent: Option<AgentClient<SerialLog<DosboxXStream>>>,
    debug_bridge: Option<DebugBridge>,
    exit_code: Option<i32>,
    #[cfg(windows)]
    job: Option<JobObject>,
}

impl DosboxXSession {
    /// Starts DOSBox-X for a spec and a prepared command line.
    pub fn spawn(
        spec: &EngineSpec,
        processes: &dyn ProcessSupervisor,
        command: EngineCommand,
    ) -> Result<Self, EngineError> {
        fs::create_dir_all(&spec.log_dir)?;
        let endpoints = DosboxXEndpoints::from_command(&command)?;
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
            EngineError::Setup(format!("could not prepare the DOSBox-X process: {error}"))
        })?;

        #[cfg(windows)]
        let job = JobObject::kill_on_close().map_err(|error| {
            EngineError::Setup(format!("could not create the DOSBox-X job object: {error}"))
        })?;

        #[cfg(windows)]
        let mut child = process.spawn()?;
        #[cfg(not(windows))]
        let child = process.spawn()?;

        #[cfg(windows)]
        if let Err(error) = job.assign_child(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(EngineError::Setup(format!(
                "could not assign the DOSBox-X process to its job object: {error}"
            )));
        }

        Ok(Self {
            child,
            spec: spec.clone(),
            command,
            endpoints,
            agent: None,
            debug_bridge: None,
            exit_code: None,
            #[cfg(windows)]
            job: Some(job),
        })
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

    fn record_exit(&mut self, code: i32) -> Result<i32, EngineError> {
        self.exit_code = Some(code);
        Ok(code)
    }

    fn agent_mut(&mut self) -> Result<&mut AgentClient<SerialLog<DosboxXStream>>, EngineError> {
        if self.spec.booter {
            return Err(EngineError::NoAgent);
        }

        self.agent.as_mut().ok_or(EngineError::NoAgent)
    }
}

impl Drop for DosboxXSession {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }

        if let Some(mut bridge) = self.debug_bridge.take() {
            bridge.stop();
        }

        // Close the job after the child is reaped so kill-on-close also ends
        // any process the engine started in the job.
        #[cfg(windows)]
        drop(self.job.take());
    }
}

impl EngineSession for DosboxXSession {
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
            .clone()
            .ok_or(EngineError::NoAgent)?;
        let deadline = Instant::now() + timeout;
        let stream = loop {
            if let Some(code) = self.poll_exit()? {
                return Err(EngineError::Exited(code));
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(EngineError::ReadyTimeout(timeout));
            }

            let attempt_deadline = Instant::now() + remaining.min(READY_CONNECT_POLL);
            match crate::stream::connect(&agent_serial, attempt_deadline) {
                Ok(stream) => break stream,
                Err(_) => {
                    if let Some(code) = self.poll_exit()? {
                        return Err(EngineError::Exited(code));
                    }

                    if Instant::now() >= deadline {
                        return Err(EngineError::ReadyTimeout(timeout));
                    }
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
            Err(error) => {
                if let Some(code) = self.poll_exit()? {
                    return Err(EngineError::Exited(code));
                }

                Err(protocol_error(error))
            }
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
        if let Some(code) = self.exit_code {
            return Ok(code);
        }
        if let Some(code) = self.poll_exit()? {
            return Ok(code);
        }

        // The grace window covers the agent shutdown, the signal, and the wait
        // for the process to exit. Capture it before any of those steps so no
        // step can extend the stop by another full grace period.
        let deadline = Instant::now() + grace;

        if let Some(agent) = self.agent.as_mut() {
            let grace_ms = u64::try_from(grace.as_millis()).unwrap_or(u64::MAX);
            let remaining = deadline.saturating_duration_since(Instant::now());
            let _ = agent.shutdown_with_timeout(grace_ms, remaining);
        }

        #[cfg(unix)]
        terminate_process_group(self.child.id());
        #[cfg(windows)]
        terminate_process_group(&mut self.child);

        loop {
            if let Some(code) = self.poll_exit()? {
                return Ok(code);
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

    fn debug_endpoint(&mut self, kind: DebugKind) -> Result<String, EngineError> {
        match kind {
            DebugKind::Gdb => Err(EngineError::Unsupported("debugger")),
            DebugKind::Native => {
                if !capabilities().contains(Capability::DebugNative) {
                    return Err(EngineError::Unsupported("debugger"));
                }

                let Some(kind) = self.endpoints.debug.as_ref() else {
                    return Err(EngineError::Unsupported("debugger"));
                };

                // The listener stays closed until the debugger is actually
                // requested, so it cannot pair an unrelated local connection.
                // DOSBox-X reconnects to the MCP port within about a second,
                // so a listener started here still accepts it.
                if self.debug_bridge.is_none() {
                    self.debug_bridge = Some(DebugBridge::start(kind)?);
                }

                Ok(kind.display())
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

/// Sends `SIGTERM` to the engine process group.
///
/// A group that has already exited returns `ESRCH`, which is the outcome the
/// caller wants, so the result is ignored. The caller falls back to killing
/// the child when the grace window passes.
#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };

    let _ = unsafe { libc::kill(-pid, libc::SIGTERM) };
}

/// Ends the engine process when the platform has no signal to send.
#[cfg(windows)]
fn terminate_process_group(child: &mut Child) {
    let _ = child.kill();
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
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use dosker_agent_proto::{
        AGENT_VERSION, ExecEnd, Frame, FrameType, GetEnd, Hello, Incoming, PutEnd, PutRequest,
        ShutdownAck, decode_payload, encode_payload, read_frame, write_frame,
    };
    use dosker_core::ContainerId;
    use dosker_engine::{Capability, DisplayMode, EngineEndpointKind, EngineSpec};
    use dosker_platform::process::{ProcessSupervisor, system};

    use super::*;

    #[derive(Clone)]
    struct FakeAgent {
        exec_output: Vec<u8>,
        exec_status: i32,
        shutdown_delay: Duration,
        requests: Arc<Mutex<Vec<FrameType>>>,
        stored: Arc<Mutex<Vec<u8>>>,
    }

    impl FakeAgent {
        fn new() -> Self {
            Self {
                exec_output: Vec::new(),
                exec_status: 0,
                shutdown_delay: Duration::ZERO,
                requests: Arc::new(Mutex::new(Vec::new())),
                stored: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn requests(&self) -> Vec<FrameType> {
            self.requests.lock().expect("lock the request log").clone()
        }

        fn run(self, listener: TcpListener) -> thread::JoinHandle<()> {
            thread::spawn(move || {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));

                let hello = Hello {
                    protocol_version: 1,
                    agent_version: AGENT_VERSION.to_owned(),
                    dos: "FreeDOS 1.3".to_owned(),
                    memory_kb: 640,
                    control_port: 2,
                    max_payload: 4096,
                    features: Vec::new(),
                };
                let hello = Frame::new(
                    FrameType::Hello,
                    1,
                    0,
                    encode_payload(&hello).expect("encode hello"),
                )
                .expect("build hello");
                if write_frame(&mut stream, &hello).is_err() {
                    return;
                }

                loop {
                    let Ok(Incoming::Frame(frame)) = read_frame(&mut stream) else {
                        return;
                    };
                    if matches!(frame.kind, FrameType::Ack | FrameType::Nak) {
                        continue;
                    }

                    self.requests
                        .lock()
                        .expect("lock the request log")
                        .push(frame.kind);
                    if !ack(&mut stream, &frame) {
                        return;
                    }

                    match frame.kind {
                        FrameType::ExecReq => {
                            let output = self.exec_output.clone();
                            if !output.is_empty() {
                                let data = Frame::new(
                                    FrameType::ExecData,
                                    frame.request_id,
                                    0,
                                    output.clone(),
                                )
                                .expect("build exec data");
                                if write_frame(&mut stream, &data).is_err() {
                                    return;
                                }
                                if read_frame(&mut stream).is_err() {
                                    return;
                                }
                            }
                            let end = ExecEnd {
                                exit_code: self.exec_status,
                                output_bytes: output.len() as u64,
                                truncated: false,
                                duration_ms: 0,
                                command_not_found: false,
                            };
                            let end = Frame::new(
                                FrameType::ExecEnd,
                                frame.request_id,
                                1,
                                encode_payload(&end).expect("encode exec end"),
                            )
                            .expect("build exec end");
                            if write_frame(&mut stream, &end).is_err() {
                                return;
                            }
                            if read_frame(&mut stream).is_err() {
                                return;
                            }
                        }
                        FrameType::PutReq => {
                            let Ok(request) = decode_payload::<PutRequest>(&frame.payload) else {
                                return;
                            };
                            let mut received = Vec::new();
                            while (received.len() as u64) < request.size {
                                let Ok(Incoming::Frame(data)) = read_frame(&mut stream) else {
                                    return;
                                };
                                if data.kind != FrameType::PutData {
                                    continue;
                                }
                                received.extend_from_slice(&data.payload);
                                if !ack(&mut stream, &data) {
                                    return;
                                }
                            }
                            *self.stored.lock().expect("lock the stored file") = received.clone();
                            let end = PutEnd {
                                ok: true,
                                bytes: received.len() as u64,
                            };
                            let end = Frame::new(
                                FrameType::PutEnd,
                                frame.request_id,
                                0,
                                encode_payload(&end).expect("encode put end"),
                            )
                            .expect("build put end");
                            if write_frame(&mut stream, &end).is_err() {
                                return;
                            }
                            if read_frame(&mut stream).is_err() {
                                return;
                            }
                        }
                        FrameType::GetReq => {
                            let data = self.stored.lock().expect("lock the stored file").clone();
                            if !data.is_empty() {
                                let chunk = Frame::new(
                                    FrameType::GetData,
                                    frame.request_id,
                                    0,
                                    data.clone(),
                                )
                                .expect("build get data");
                                if write_frame(&mut stream, &chunk).is_err() {
                                    return;
                                }
                                if read_frame(&mut stream).is_err() {
                                    return;
                                }
                            }
                            let end = GetEnd {
                                ok: true,
                                size: data.len() as u64,
                            };
                            let end = Frame::new(
                                FrameType::GetEnd,
                                frame.request_id,
                                1,
                                encode_payload(&end).expect("encode get end"),
                            )
                            .expect("build get end");
                            if write_frame(&mut stream, &end).is_err() {
                                return;
                            }
                            if read_frame(&mut stream).is_err() {
                                return;
                            }
                        }
                        FrameType::Shutdown => {
                            thread::sleep(self.shutdown_delay);
                            let ack = ShutdownAck { ready: true };
                            let ack = Frame::new(
                                FrameType::ShutdownAck,
                                frame.request_id,
                                0,
                                encode_payload(&ack).expect("encode shutdown ack"),
                            )
                            .expect("build shutdown ack");
                            if write_frame(&mut stream, &ack).is_err() {
                                return;
                            }
                            let _ = read_frame(&mut stream);
                        }
                        _ => {}
                    }
                }
            })
        }
    }

    fn ack(stream: &mut TcpStream, frame: &Frame) -> bool {
        let ack = Frame::new(FrameType::Ack, frame.request_id, frame.sequence, Vec::new())
            .expect("build ack");

        write_frame(stream, &ack).is_ok()
    }

    fn session_spec(root: &Path, booter: bool) -> EngineSpec {
        EngineSpec {
            root: root.to_path_buf(),
            container_id: ContainerId::generate(),
            workdir: "C:\\".to_owned(),
            env: BTreeMap::new(),
            cmd: None,
            expects_agent: !booter,
            log_dir: root.to_path_buf(),
            disk_path: root.join("disk.img"),
            disk_interface: "ide".to_owned(),
            serial_ports: 2,
            control_port: 2,
            booter,
            display: DisplayMode::None,
            vnc_port: None,
            allow_reboot: false,
            net: "none".to_owned(),
            ports: Vec::new(),
            volumes: Vec::new(),
            engine_config: BTreeMap::new(),
        }
    }

    fn agent_endpoints(listener: &TcpListener) -> DosboxXEndpoints {
        let port = listener
            .local_addr()
            .expect("read the listener address")
            .port();

        DosboxXEndpoints {
            agent_serial: Some(EngineEndpointKind::Tcp {
                host: Ipv4Addr::LOCALHOST,
                port,
            }),
            debug: None,
        }
    }

    fn debug_endpoints(listener: &TcpListener) -> DosboxXEndpoints {
        let port = listener
            .local_addr()
            .expect("read the listener address")
            .port();

        DosboxXEndpoints {
            agent_serial: None,
            debug: Some(EngineEndpointKind::Tcp {
                host: Ipv4Addr::LOCALHOST,
                port,
            }),
        }
    }

    fn connect_loopback(port: u16) -> TcpStream {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match TcpStream::connect((Ipv4Addr::LOCALHOST, port)) {
                Ok(stream) => return stream,
                Err(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("connect to the bridge: {error}"),
            }
        }
    }

    fn session_with(spec: EngineSpec, endpoints: DosboxXEndpoints, child: Child) -> DosboxXSession {
        let command = EngineCommand {
            program: PathBuf::from("dosbox-x"),
            args: Vec::new(),
            env: Vec::new(),
            endpoints: endpoints.engine_endpoints(),
        };

        DosboxXSession {
            child,
            spec,
            command,
            endpoints,
            agent: None,
            exit_code: None,
            debug_bridge: None,
        }
    }

    fn spawn_child(program: &Path, args: &[&str]) -> Child {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        system()
            .prepare_engine(&mut command)
            .expect("prepare the child process");

        command.spawn().expect("spawn the child process")
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
        let mut permissions = std::fs::metadata(&path)
            .expect("read the script metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("set the script permissions");

        path
    }

    fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "the marker file {} did not appear",
                path.display()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn exec_request() -> ExecRequest {
        ExecRequest {
            command: "ECHO HELLO".to_owned(),
            env: BTreeMap::new(),
            workdir: "C:\\".to_owned(),
            timeout_ms: 2_000,
        }
    }

    #[test]
    fn ready_performs_the_agent_handshake() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind the listener");
        let endpoints = agent_endpoints(&listener);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let mut session = session_with(spec, endpoints, child);

        let agent = FakeAgent::new();
        let server = agent.clone().run(listener);

        session
            .ready(Duration::from_secs(2))
            .expect("the handshake completes");

        assert!(session.agent.is_some(), "the session stores the agent");
        assert!(
            agent.requests().contains(&FrameType::HelloAck),
            "the host answered the greeting: {:?}",
            agent.requests()
        );

        drop(session);
        let _ = server.join();
    }

    #[test]
    fn exec_returns_the_agent_output_and_status() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind the listener");
        let endpoints = agent_endpoints(&listener);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let mut session = session_with(spec, endpoints, child);

        let mut agent = FakeAgent::new();
        agent.exec_output = b"HELLO".to_vec();
        let server = agent.clone().run(listener);

        session
            .ready(Duration::from_secs(2))
            .expect("the handshake completes");
        let result = session.exec(&exec_request()).expect("execute the command");

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.output, "HELLO");
        assert!(!result.timed_out);
        assert!(!result.truncated);
        assert!(agent.requests().contains(&FrameType::ExecReq));

        drop(session);
        let _ = server.join();
    }

    #[test]
    fn put_and_get_round_trip_a_file() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind the listener");
        let endpoints = agent_endpoints(&listener);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let mut session = session_with(spec, endpoints, child);

        let agent = FakeAgent::new();
        let server = agent.clone().run(listener);

        session
            .ready(Duration::from_secs(2))
            .expect("the handshake completes");
        let written = session
            .put_file("C:\\HELLO.TXT", b"HELLO")
            .expect("upload the file");
        let data = session
            .get_file("C:\\HELLO.TXT")
            .expect("download the file");

        assert_eq!(written, 5);
        assert_eq!(data, b"HELLO");

        drop(session);
        let _ = server.join();
    }

    #[test]
    fn stop_sends_shutdown_and_reports_the_exit() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let ready = dir.path().join("ready");
        let script = write_script(
            dir.path(),
            "engine.sh",
            &format!(
                "trap 'exit 0' TERM\ntouch '{}'\nwhile :; do sleep 0.05; done",
                ready.display()
            ),
        );
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind the listener");
        let endpoints = agent_endpoints(&listener);
        let child = spawn_child(&script, &[]);
        let mut session = session_with(spec, endpoints, child);

        let agent = FakeAgent::new();
        let server = agent.clone().run(listener);

        session
            .ready(Duration::from_secs(2))
            .expect("the handshake completes");
        // The fake agent makes readiness immediate; wait until the shell script
        // has installed its TERM trap before the stop signal arrives.
        wait_for_file(&ready);
        let code = session
            .stop(Duration::from_secs(2))
            .expect("stop the session");

        assert_eq!(code, 0);
        assert!(
            agent.requests().contains(&FrameType::Shutdown),
            "the agent received SHUTDOWN: {:?}",
            agent.requests()
        );

        drop(session);
        let _ = server.join();
    }

    #[test]
    fn stop_times_out_and_kills_a_guest_that_does_not_exit() {
        const GRACE: Duration = Duration::from_millis(300);

        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let ready = dir.path().join("ready");
        let script = write_script(
            dir.path(),
            "engine.sh",
            &format!(
                "trap '' TERM\ntouch '{}'\nwhile :; do sleep 0.05; done",
                ready.display()
            ),
        );
        let child = spawn_child(&script, &[]);
        let endpoints = DosboxXEndpoints {
            agent_serial: None,
            debug: None,
        };
        let mut session = session_with(spec, endpoints, child);
        // Wait until the child has installed its TERM trap before the stop
        // signal arrives.
        wait_for_file(&ready);

        match session.stop(GRACE) {
            Err(EngineError::StopTimeout(grace)) => assert_eq!(grace, GRACE),
            other => panic!("expected a stop timeout, got {other:?}"),
        }
    }

    #[test]
    fn kill_ends_the_process() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let endpoints = DosboxXEndpoints {
            agent_serial: None,
            debug: None,
        };
        let mut session = session_with(spec, endpoints, child);

        let code = session.kill().expect("kill the process");

        assert_eq!(code, 137);
        assert_eq!(session.try_exit().expect("poll the exit"), Some(137));
    }

    #[test]
    fn ready_reports_an_early_exit() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind the listener");
        let endpoints = agent_endpoints(&listener);
        // Drop the listener so no agent accepts the connection.
        drop(listener);
        let child = spawn_child(Path::new("/bin/sh"), &["-c", "exit 7"]);
        let mut session = session_with(spec, endpoints, child);

        let started = Instant::now();
        match session.ready(Duration::from_secs(60)) {
            Err(EngineError::Exited(7)) => {}
            other => panic!("expected an exit code of 7, got {other:?}"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "ready waited {:?} for an early exit",
            started.elapsed()
        );
    }

    #[test]
    fn ready_times_out_without_an_agent() {
        const TIMEOUT: Duration = Duration::from_millis(300);

        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind the listener");
        let endpoints = agent_endpoints(&listener);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let mut session = session_with(spec, endpoints, child);

        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buffer = [0u8; 1];
            let _ = stream.read(&mut buffer);
        });

        match session.ready(TIMEOUT) {
            Err(EngineError::ReadyTimeout(timeout)) => assert_eq!(timeout, TIMEOUT),
            other => panic!("expected a ready timeout, got {other:?}"),
        }
    }

    #[test]
    fn booter_ready_accepts_a_live_process() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), true);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let endpoints = DosboxXEndpoints {
            agent_serial: None,
            debug: None,
        };
        let mut session = session_with(spec, endpoints, child);

        session
            .ready(Duration::from_millis(50))
            .expect("a live booter is ready");
    }

    #[test]
    fn booter_ready_reports_an_exited_process() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), true);
        let script = write_script(dir.path(), "exit.sh", "exit 3");
        let child = spawn_child(&script, &[]);
        let endpoints = DosboxXEndpoints {
            agent_serial: None,
            debug: None,
        };
        let mut session = session_with(spec, endpoints, child);

        match session.ready(Duration::from_millis(50)) {
            Err(EngineError::Exited(3)) => {}
            other => panic!("expected an exit code of 3, got {other:?}"),
        }
    }

    #[test]
    fn booter_exec_returns_no_agent() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), true);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let endpoints = DosboxXEndpoints {
            agent_serial: None,
            debug: None,
        };
        let mut session = session_with(spec, endpoints, child);

        match session.exec(&exec_request()) {
            Err(EngineError::NoAgent) => {}
            other => panic!("expected no agent, got {other:?}"),
        }
    }

    #[test]
    fn booter_transfer_returns_no_agent() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), true);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let endpoints = DosboxXEndpoints {
            agent_serial: None,
            debug: None,
        };
        let mut session = session_with(spec, endpoints, child);

        match session.put_file("C:\\HELLO.TXT", b"HELLO") {
            Err(EngineError::NoAgent) => {}
            other => panic!("expected no agent, got {other:?}"),
        }
        match session.get_file("C:\\HELLO.TXT") {
            Err(EngineError::NoAgent) => {}
            other => panic!("expected no agent, got {other:?}"),
        }
    }

    #[test]
    fn debug_bridge_pumps_bytes_between_two_connections() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind the listener");
        let port = listener
            .local_addr()
            .expect("read the listener address")
            .port();
        drop(listener);

        let mut bridge = DebugBridge::start(&EngineEndpointKind::Tcp {
            host: Ipv4Addr::LOCALHOST,
            port,
        })
        .expect("start the debug bridge");

        // The first accepted connection is the engine, the second the client.
        let mut engine = connect_loopback(port);
        let mut client = connect_loopback(port);
        for stream in [&engine, &client] {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set the read timeout");
        }

        client
            .write_all(b"request")
            .expect("write the client request");
        let mut buffer = [0u8; 7];
        engine.read_exact(&mut buffer).expect("read the request");
        assert_eq!(&buffer, b"request");

        engine
            .write_all(b"response")
            .expect("write the engine response");
        let mut buffer = [0u8; 8];
        client.read_exact(&mut buffer).expect("read the response");
        assert_eq!(&buffer, b"response");

        let started = Instant::now();
        bridge.stop();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the bridge stop took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn debug_endpoint_returns_the_native_endpoint() {
        if !capabilities().contains(Capability::DebugNative) {
            return;
        }

        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind the listener");
        let endpoints = debug_endpoints(&listener);
        let port = listener
            .local_addr()
            .expect("read the listener address")
            .port();
        let expected = endpoints
            .debug
            .as_ref()
            .expect("the debug endpoint is set")
            .display();
        // The bridge binds the port lazily, so the probe listener must be
        // closed before the endpoint is requested.
        drop(listener);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let mut session = session_with(spec, endpoints, child);

        let endpoint = session
            .debug_endpoint(DebugKind::Native)
            .expect("the native debug endpoint is available");

        assert_eq!(endpoint, expected);
        assert!(
            session.debug_bridge.is_some(),
            "the bridge starts when the endpoint is requested"
        );
        let probe = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .expect("the bridge accepts a connection");
        drop(probe);
    }

    #[test]
    fn debug_endpoint_rejects_gdb_without_a_gdbstub() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let endpoints = DosboxXEndpoints {
            agent_serial: None,
            debug: None,
        };
        let mut session = session_with(spec, endpoints, child);

        match session.debug_endpoint(DebugKind::Gdb) {
            Err(EngineError::Unsupported("debugger")) => {}
            other => panic!("expected an unsupported debugger, got {other:?}"),
        }
    }
}
