//! A running Bochs process and its host endpoints.

#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::Mutex;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
#[cfg(unix)]
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dosker_agent_proto::{
    AgentClient, ExecRequest as ProtoExecRequest, MAX_OUTPUT_BYTES, ProtoError,
};
use dosker_engine::{
    DebugKind, EngineCommand, EngineEndpoint, EngineError, EngineSession, EngineSpec, ExecRequest,
    ExecResult,
};
#[cfg(unix)]
use dosker_platform::engine_socket_path;
#[cfg(windows)]
use dosker_platform::process::JobObject;
use dosker_platform::process::ProcessSupervisor;

use crate::endpoints::BochsEndpoints;
#[cfg(unix)]
use crate::pty::PtyPair;
use crate::serial_log::SerialLog;
use crate::stream::BochsStream;

/// How long a booter must stay alive before it is considered ready.
const BOOTER_ALIVE_PROBE: Duration = Duration::from_millis(250);
/// How long one connect attempt may take while the child is polled.
const READY_CONNECT_POLL: Duration = Duration::from_millis(20);
/// How often the stop wait polls for the process.
const STOP_POLL: Duration = Duration::from_millis(20);
/// How often the debug bridge checks its stop flag.
#[cfg(unix)]
const DEBUG_BRIDGE_POLL: Duration = Duration::from_millis(20);
/// How long one debug bridge write may block.
#[cfg(unix)]
const DEBUG_BRIDGE_WRITE: Duration = Duration::from_secs(1);
/// The debug bridge transfer buffer size.
#[cfg(unix)]
const DEBUG_BRIDGE_BUFFER: usize = 8192;
/// The most bytes the debug bridge sends to the master in one write.
#[cfg(unix)]
const DEBUG_BRIDGE_CHUNK: usize = 1024;
/// The most debugger output held before a client connects.
#[cfg(unix)]
const DEBUG_PENDING_LIMIT: usize = 65_536;

/// Holds debugger output for the connected client.
///
/// One mutex covers the client and the buffered output, so a client attach and
/// a reader publish cannot interleave. Output that arrives before a client
/// connects is kept, up to the pending limit, and replayed on attach.
#[cfg(unix)]
#[derive(Default)]
struct DebugChannel {
    state: Mutex<DebugChannelState>,
}

#[cfg(unix)]
#[derive(Default)]
struct DebugChannelState {
    client: Option<UnixStream>,
    pending: Vec<u8>,
}

#[cfg(unix)]
impl DebugChannel {
    fn lock(&self) -> std::sync::MutexGuard<'_, DebugChannelState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Delivers debugger output to the client, or buffers it until one
    /// connects.
    fn publish(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        let mut state = self.lock();
        let delivered = match state.client.as_mut() {
            Some(client) => client
                .write_all(bytes)
                .and_then(|()| client.flush())
                .is_ok(),
            None => false,
        };
        if delivered {
            return;
        }

        state.client = None;
        state.pending.extend_from_slice(bytes);
        if state.pending.len() > DEBUG_PENDING_LIMIT {
            let excess = state.pending.len() - DEBUG_PENDING_LIMIT;
            state.pending.drain(..excess);
        }
    }

    /// Replays buffered output and stores the connected client.
    fn attach(&self, mut client: UnixStream) {
        let mut state = self.lock();
        let pending = std::mem::take(&mut state.pending);
        if !pending.is_empty()
            && client
                .write_all(&pending)
                .and_then(|()| client.flush())
                .is_err()
        {
            // Keep the bytes for the next client and drop the failed one.
            state.pending = pending;
            return;
        }

        state.client = Some(client);
    }

    /// Drops the connected client.
    fn detach(&self) {
        self.lock().client = None;
    }
}

/// Drains the debugger pty into the engine log and the debug channel.
#[cfg(unix)]
struct DebugReader {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

#[cfg(unix)]
impl DebugReader {
    /// Starts the reader on a blocking master descriptor.
    fn start(master: File, log: File, channel: Arc<DebugChannel>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || reader_loop(master, log, channel, &thread_stop));

        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(unix)]
impl Drop for DebugReader {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
fn reader_loop(
    mut master: File,
    mut log: File,
    channel: Arc<DebugChannel>,
    stop: &Arc<AtomicBool>,
) {
    use std::os::fd::AsRawFd;

    let timeout = i32::try_from(DEBUG_BRIDGE_POLL.as_millis()).unwrap_or(20);
    let mut buffer = [0u8; DEBUG_BRIDGE_BUFFER];

    while !stop.load(Ordering::SeqCst) {
        let mut descriptor = libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `descriptor` points to one initialized `pollfd` and the
        // finite timeout keeps `poll` from blocking indefinitely.
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        if ready == 0 {
            continue;
        }

        match master.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => {
                let chunk = &buffer[..read];
                let _ = log.write_all(chunk).and_then(|()| log.flush());
                channel.publish(chunk);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                thread::sleep(DEBUG_BRIDGE_POLL);
            }
            Err(_) => return,
        }
    }
}

/// Bridges a Unix socket client to the debugger pty master.
#[cfg(unix)]
struct DebugBridge {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    socket: PathBuf,
}

#[cfg(unix)]
impl DebugBridge {
    /// Binds the debug socket and serves one client at a time.
    fn start(socket: &Path, channel: Arc<DebugChannel>, master: File) -> Result<Self, EngineError> {
        if let Some(parent) = socket.parent() {
            dosker_platform::fs::ensure_dir_mode(parent, 0o700).map_err(|error| {
                EngineError::Setup(format!(
                    "could not prepare the debug socket directory: {error}"
                ))
            })?;
        }

        match fs::remove_file(socket) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(EngineError::Io(error)),
        }

        let listener = UnixListener::bind(socket)?;
        listener.set_nonblocking(true)?;

        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_channel = Arc::clone(&channel);
        let handle = thread::spawn(move || {
            bridge_loop(&listener, &thread_channel, &master, &thread_stop);
        });

        Ok(Self {
            stop,
            handle: Some(handle),
            socket: socket.to_path_buf(),
        })
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(unix)]
impl Drop for DebugBridge {
    fn drop(&mut self) {
        self.stop();
        match fs::remove_file(&self.socket) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }
}

#[cfg(unix)]
fn bridge_loop(
    listener: &UnixListener,
    channel: &Arc<DebugChannel>,
    master: &File,
    stop: &Arc<AtomicBool>,
) {
    while !stop.load(Ordering::SeqCst) {
        let Some(client) = accept_client(listener, stop) else {
            return;
        };
        let Ok(forward) = client.try_clone() else {
            continue;
        };
        let _ = forward.set_write_timeout(Some(DEBUG_BRIDGE_WRITE));
        channel.attach(forward);
        pump_client(client, master, stop);
        channel.detach();
    }
}

#[cfg(unix)]
fn accept_client(listener: &UnixListener, stop: &Arc<AtomicBool>) -> Option<UnixStream> {
    loop {
        if stop.load(Ordering::SeqCst) {
            return None;
        }

        match listener.accept() {
            Ok((client, _address)) => return Some(client),
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

#[cfg(unix)]
fn pump_client(mut client: UnixStream, master: &File, stop: &Arc<AtomicBool>) {
    let _ = client.set_read_timeout(Some(DEBUG_BRIDGE_POLL));
    // The status flags are shared by every dup of the master, so the reader
    // and every master writer poll from this point on.
    if set_nonblocking(master).is_err() {
        return;
    }
    let mut buffer = [0u8; DEBUG_BRIDGE_BUFFER];

    while !stop.load(Ordering::SeqCst) {
        match client.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if !write_master(master, &buffer[..read], stop) {
                    break;
                }
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

/// Makes writes to the master non-blocking so `poll` bounds the wait.
#[cfg(unix)]
fn set_nonblocking(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let fd = file.as_raw_fd();
    // SAFETY: `fd` is a valid open descriptor and `F_GETFL` takes no argument.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above, and `F_SETFL` only changes the status flags.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

/// Writes client input to the master, waiting at most `DEBUG_BRIDGE_WRITE`
/// for the guest to accept more.
#[cfg(unix)]
fn write_master(mut master: &File, mut bytes: &[u8], stop: &Arc<AtomicBool>) -> bool {
    use std::os::fd::AsRawFd;

    let fd = master.as_raw_fd();
    let timeout = i32::try_from(DEBUG_BRIDGE_WRITE.as_millis()).unwrap_or(1_000);

    while !bytes.is_empty() {
        if stop.load(Ordering::SeqCst) {
            return false;
        }

        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: `descriptor` points to one initialized `pollfd` and the
        // finite timeout keeps `poll` from blocking indefinitely.
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        if ready == 0 {
            return false;
        }

        let chunk = bytes.len().min(DEBUG_BRIDGE_CHUNK);
        match master.write(&bytes[..chunk]) {
            Ok(0) => return false,
            Ok(written) => bytes = &bytes[written..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) => {}
            Err(_) => return false,
        }
    }

    true
}

/// A Bochs process attached to one container disk.
pub struct BochsSession {
    child: Child,
    spec: EngineSpec,
    command: EngineCommand,
    endpoints: BochsEndpoints,
    agent: Option<AgentClient<SerialLog<BochsStream>>>,
    #[cfg(unix)]
    debug: Option<DebugSession>,
    exit_code: Option<i32>,
    #[cfg(windows)]
    job: Option<JobObject>,
}

/// The debugger pty and its bridge state.
#[cfg(unix)]
struct DebugSession {
    pty: PtyPair,
    channel: Arc<DebugChannel>,
    reader: DebugReader,
    bridge: Option<DebugBridge>,
    socket: PathBuf,
}

impl BochsSession {
    /// Starts Bochs for a spec and a prepared command line.
    pub fn spawn(
        spec: &EngineSpec,
        processes: &dyn ProcessSupervisor,
        command: EngineCommand,
    ) -> Result<Self, EngineError> {
        fs::create_dir_all(&spec.log_dir)?;
        remove_image_lock(&spec.disk_path)?;
        let endpoints = BochsEndpoints::from_command(&command)?;
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
            .envs(command.env.iter().map(|(key, value)| (key, value)));

        #[cfg(unix)]
        let mut debug: Option<DebugSession> = None;
        #[cfg(unix)]
        let mut stdin = None;
        #[cfg(unix)]
        let mut stdout = None;

        #[cfg(unix)]
        if command.args.iter().any(|argument| argument == "-dbg") {
            let mut pty = PtyPair::open()?;
            let channel = Arc::new(DebugChannel::default());
            let reader =
                DebugReader::start(pty.master_clone()?, log.try_clone()?, Arc::clone(&channel));
            let socket = engine_socket_path(&spec.root, &spec.container_id, "debug");
            let slave = pty.take_slave().ok_or_else(|| {
                EngineError::Setup("the debugger pty has no slave end".to_owned())
            })?;
            stdout = Some(Stdio::from(slave.try_clone()?));
            stdin = Some(Stdio::from(slave));
            debug = Some(DebugSession {
                pty,
                channel,
                reader,
                bridge: None,
                socket,
            });
        }

        #[cfg(unix)]
        process
            .stdin(stdin.take().unwrap_or_else(Stdio::null))
            .stdout(stdout.take().unwrap_or_else(|| Stdio::from(log)))
            .stderr(Stdio::from(log_err));

        #[cfg(not(unix))]
        process
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));

        processes.prepare_engine(&mut process).map_err(|error| {
            EngineError::Setup(format!("could not prepare the Bochs process: {error}"))
        })?;

        #[cfg(windows)]
        let job = JobObject::kill_on_close().map_err(|error| {
            EngineError::Setup(format!("could not create the Bochs job object: {error}"))
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
                "could not assign the Bochs process to its job object: {error}"
            )));
        }

        #[allow(unused_mut)]
        let mut session = Self {
            child,
            spec: spec.clone(),
            command,
            endpoints,
            agent: None,
            #[cfg(unix)]
            debug,
            exit_code: None,
            #[cfg(windows)]
            job: Some(job),
        };

        #[cfg(unix)]
        if let Some(debug) = session.debug.as_mut() {
            // The internal debugger stops before the first instruction, so the
            // guest only boots once the session sends `c`.
            debug.pty.write_line("c")?;
            let mut log = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)?;
            writeln!(log, "debug pty {}", debug.pty.slave_path().display())?;
            log.flush()?;
        }

        Ok(session)
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

    fn agent_mut(&mut self) -> Result<&mut AgentClient<SerialLog<BochsStream>>, EngineError> {
        if self.spec.booter {
            return Err(EngineError::NoAgent);
        }

        self.agent.as_mut().ok_or(EngineError::NoAgent)
    }
}

impl Drop for BochsSession {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }

        #[cfg(unix)]
        if let Some(mut debug) = self.debug.take() {
            if let Some(mut bridge) = debug.bridge.take() {
                bridge.stop();
            }
            debug.reader.stop();
            match fs::remove_file(&debug.socket) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => {}
            }
        }

        // Close the job after the child is reaped so kill-on-close also ends
        // any process the engine started in the job.
        #[cfg(windows)]
        drop(self.job.take());
    }
}

impl EngineSession for BochsSession {
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
                // A setup error cannot succeed on a retry, so report it now.
                Err(error @ EngineError::Setup(_)) => return Err(error),
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
                #[cfg(unix)]
                {
                    if self.poll_exit()?.is_some() {
                        return Err(EngineError::Unsupported("debugger"));
                    }

                    let Some(debug) = self.debug.as_mut() else {
                        return Err(EngineError::Unsupported("debugger"));
                    };

                    let socket = debug.socket.clone();
                    if debug.bridge.is_none() {
                        let master = debug.pty.master_clone()?;
                        debug.bridge = Some(DebugBridge::start(
                            &socket,
                            Arc::clone(&debug.channel),
                            master,
                        )?);
                    }

                    // Break the running guest so the debugger prompt answers
                    // the client that is about to connect.
                    interrupt_process_group(self.child.id());

                    Ok(socket.display().to_string())
                }
                #[cfg(windows)]
                {
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

/// Removes the lock file Bochs leaves beside a disk image when it is killed.
fn remove_image_lock(disk_path: &Path) -> Result<(), EngineError> {
    let mut lock = disk_path.as_os_str().to_owned();
    lock.push(".lock");

    match fs::remove_file(Path::new(&lock)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(EngineError::Io(error)),
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

/// Sends `SIGINT` to the engine process group to break the guest.
#[cfg(unix)]
fn interrupt_process_group(pid: u32) {
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };

    let _ = unsafe { libc::kill(-pid, libc::SIGINT) };
}

fn status_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
    }

    #[cfg(windows)]
    {
        status.code().unwrap_or(-1)
    }
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
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::pty::PtyPair;

    use dosker_agent_proto::{
        AGENT_VERSION, ExecEnd, Frame, FrameType, GetEnd, Hello, Incoming, PutEnd, PutRequest,
        ShutdownAck, decode_payload, encode_payload, read_frame, write_frame,
    };
    use dosker_core::ContainerId;
    use dosker_engine::{DebugKind, DisplayMode, EngineEndpointKind, EngineSpec, ExecRequest};
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

    fn agent_endpoints(listener: &TcpListener) -> BochsEndpoints {
        let port = listener
            .local_addr()
            .expect("read the listener address")
            .port();

        BochsEndpoints {
            agent_serial: Some(EngineEndpointKind::Tcp {
                host: Ipv4Addr::LOCALHOST,
                port,
            }),
        }
    }

    fn session_with(spec: EngineSpec, endpoints: BochsEndpoints, child: Child) -> BochsSession {
        let command = EngineCommand {
            program: PathBuf::from("bochs"),
            args: Vec::new(),
            env: Vec::new(),
            endpoints: endpoints.engine_endpoints(),
        };

        BochsSession {
            child,
            spec,
            command,
            endpoints,
            agent: None,
            debug: None,
            exit_code: None,
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

    fn wait_for_log(path: &Path, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            if text.contains(needle) {
                return;
            }

            assert!(
                Instant::now() < deadline,
                "{} did not contain {needle:?}",
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

    fn debug_command(program: &Path) -> EngineCommand {
        EngineCommand {
            program: program.to_path_buf(),
            args: vec!["-dbg".to_owned()],
            env: Vec::new(),
            endpoints: Vec::new(),
        }
    }

    fn spawn_session(spec: &EngineSpec, command: EngineCommand) -> BochsSession {
        BochsSession::spawn(spec, &system(), command).expect("spawn the session")
    }

    fn start_debug_bridge(session: &mut BochsSession) -> PathBuf {
        let debug = session.debug.as_mut().expect("the session has a debugger");
        let master = debug.pty.master_clone().expect("clone the master");
        let socket = debug.socket.clone();
        debug.bridge = Some(
            DebugBridge::start(&socket, Arc::clone(&debug.channel), master)
                .expect("start the debug bridge"),
        );

        socket
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
    fn stop_sends_shutdown_then_sigterm_and_records_143() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let ready = dir.path().join("ready");
        let script = write_script(
            dir.path(),
            "engine.sh",
            &format!("touch '{}'\nwhile :; do sleep 0.05; done", ready.display()),
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
        wait_for_file(&ready);
        let code = session
            .stop(Duration::from_secs(2))
            .expect("stop the session");

        assert_eq!(code, 143);
        assert!(
            agent.requests().contains(&FrameType::Shutdown),
            "the agent received SHUTDOWN: {:?}",
            agent.requests()
        );

        drop(session);
        let _ = server.join();
    }

    #[test]
    fn stop_times_out_and_kills_a_guest_that_ignores_sigterm() {
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
        let endpoints = BochsEndpoints { agent_serial: None };
        let mut session = session_with(spec, endpoints, child);
        wait_for_file(&ready);

        match session.stop(GRACE) {
            Err(EngineError::StopTimeout(grace)) => assert_eq!(grace, GRACE),
            other => panic!("expected a stop timeout, got {other:?}"),
        }
        assert_eq!(
            session
                .try_exit()
                .expect("poll the exit after the stop timeout"),
            Some(137),
            "the guest is killed and reaped after the stop timeout"
        );
    }

    #[test]
    fn kill_ends_the_process() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let endpoints = BochsEndpoints { agent_serial: None };
        let mut session = session_with(spec, endpoints, child);

        let code = session.kill().expect("kill the process");

        assert_eq!(code, 137);
        assert_eq!(session.try_exit().expect("poll the exit"), Some(137));
    }

    #[test]
    fn status_code_maps_a_signal_to_128_plus_the_signal() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let script = write_script(dir.path(), "signal.sh", "kill -TERM $$");
        let mut child = spawn_child(&script, &[]);
        let status = child.wait().expect("wait for the child");

        assert_eq!(status_code(status), 143);
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
    fn ready_reports_setup_for_a_unix_socket_endpoint() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let endpoints = BochsEndpoints {
            agent_serial: Some(EngineEndpointKind::UnixSocket(
                dir.path().join("agent.sock"),
            )),
        };
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let mut session = session_with(spec, endpoints, child);

        let started = Instant::now();
        match session.ready(Duration::from_secs(60)) {
            Err(EngineError::Setup(_)) => {}
            other => panic!("expected a setup error, got {other:?}"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "ready waited {:?} for a setup error",
            started.elapsed()
        );
    }

    #[test]
    fn booter_ready_accepts_a_live_process() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), true);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let endpoints = BochsEndpoints { agent_serial: None };
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
        let endpoints = BochsEndpoints { agent_serial: None };
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
        let endpoints = BochsEndpoints { agent_serial: None };
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
        let endpoints = BochsEndpoints { agent_serial: None };
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
    fn spawn_writes_the_continue_command_to_the_debugger() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let marker = dir.path().join("command");
        let script = write_script(
            dir.path(),
            "debugger.sh",
            &format!(
                "read line\nprintf '%s' \"$line\" > '{}'\nsleep 30",
                marker.display()
            ),
        );
        let session = spawn_session(&spec, debug_command(&script));

        wait_for_file(&marker);
        let line = std::fs::read_to_string(&marker).expect("read the command marker");
        assert_eq!(line, "c");

        drop(session);
    }

    #[test]
    fn spawn_removes_a_stale_image_lock() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let lock = dir.path().join("disk.img.lock");
        std::fs::write(&lock, b"stale").expect("write the stale lock");
        let command = EngineCommand {
            program: PathBuf::from("sleep"),
            args: vec!["30".to_owned()],
            env: Vec::new(),
            endpoints: Vec::new(),
        };

        let session = spawn_session(&spec, command);

        assert!(!lock.exists(), "the stale image lock is removed");

        drop(session);
    }

    #[test]
    fn spawn_drains_the_debugger_output_to_the_engine_log() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let script = write_script(
            dir.path(),
            "debugger.sh",
            "printf 'debugger ready\\n'\nsleep 30",
        );
        let session = spawn_session(&spec, debug_command(&script));

        wait_for_log(&dir.path().join("engine.log"), "debugger ready");

        drop(session);
    }

    #[test]
    fn debug_endpoint_returns_the_debug_socket_and_signals_the_engine() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let ready = dir.path().join("ready");
        let marker = dir.path().join("interrupted");
        let script = write_script(
            dir.path(),
            "debugger.sh",
            &format!(
                "trap 'touch \"{}\"' INT\ntouch '{}'\nwhile :; do sleep 0.05; done",
                marker.display(),
                ready.display()
            ),
        );
        let mut session = spawn_session(&spec, debug_command(&script));
        wait_for_file(&ready);

        let socket = session
            .debug_endpoint(DebugKind::Native)
            .expect("the native debug endpoint is available");

        assert_eq!(
            socket,
            engine_socket_path(&spec.root, &spec.container_id, "debug")
                .display()
                .to_string()
        );
        wait_for_file(&marker);

        drop(session);
    }

    #[test]
    fn debug_bridge_forwards_a_client_to_the_debugger() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let ready = dir.path().join("ready");
        let marker = dir.path().join("commands");
        let script = write_script(
            dir.path(),
            "debugger.sh",
            &format!(
                "trap '' INT\ntouch '{}'\nwhile read line; do printf '%s\\n' \"$line\" >> '{}'; done",
                ready.display(),
                marker.display()
            ),
        );
        let mut session = spawn_session(&spec, debug_command(&script));
        wait_for_file(&ready);
        let socket = start_debug_bridge(&mut session);

        let mut client = UnixStream::connect(&socket).expect("connect the debug client");
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set the client read timeout");
        client.write_all(b"r\n").expect("write the debug command");

        wait_for_file(&marker);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let text = std::fs::read_to_string(&marker).unwrap_or_default();
            if text.lines().any(|line| line == "r") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the debugger did not receive the command: {text:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }

        drop(session);
    }

    #[test]
    fn debug_bridge_replays_output_buffered_before_the_client_connects() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let script = write_script(
            dir.path(),
            "debugger.sh",
            "printf 'debugger ready\\n'\nsleep 30",
        );
        let mut session = spawn_session(&spec, debug_command(&script));
        wait_for_log(&dir.path().join("engine.log"), "debugger ready");
        thread::sleep(Duration::from_millis(50));
        let socket = start_debug_bridge(&mut session);

        let mut client = UnixStream::connect(&socket).expect("connect the debug client");
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set the client read timeout");
        let mut buffer = [0u8; 64];
        let read = client.read(&mut buffer).expect("read the buffered output");
        let text = String::from_utf8_lossy(&buffer[..read]);

        assert!(
            text.contains("debugger ready"),
            "the buffered output is replayed: {text:?}"
        );

        drop(session);
    }

    #[test]
    fn pump_client_detaches_when_the_guest_stops_reading() {
        let pty = PtyPair::open().expect("open a pty pair");
        let master = pty.master_clone().expect("clone the master");
        let (client, mut peer) = UnixStream::pair().expect("create a socket pair");
        let stop = Arc::new(AtomicBool::new(false));

        let writer = thread::spawn(move || {
            let chunk = vec![b'x'; DEBUG_BRIDGE_BUFFER];
            for _ in 0..512 {
                if peer.write_all(&chunk).is_err() {
                    break;
                }
            }
        });

        let started = Instant::now();
        pump_client(client, &master, &stop);

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the pump waited {:?} for a guest that stopped reading",
            started.elapsed()
        );

        let _ = writer.join();
        drop(pty);
    }

    #[test]
    fn debug_channel_restores_pending_when_attach_fails() {
        let channel = DebugChannel::default();
        channel.publish(b"debugger ready\n");

        let (client, peer) = UnixStream::pair().expect("create a socket pair");
        client
            .shutdown(std::net::Shutdown::Write)
            .expect("close the client write side");
        drop(peer);
        channel.attach(client);

        let state = channel.lock();
        assert!(state.client.is_none(), "the failed client is not stored");
        assert_eq!(
            state.pending, b"debugger ready\n",
            "the pending bytes are kept for the next client"
        );
    }

    #[test]
    fn debug_endpoint_sets_owner_only_socket_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let script = write_script(dir.path(), "debugger.sh", "trap '' INT\nsleep 30");
        let mut session = spawn_session(&spec, debug_command(&script));

        let socket = PathBuf::from(
            session
                .debug_endpoint(DebugKind::Native)
                .expect("the native debug endpoint is available"),
        );
        let parent = socket.parent().expect("the socket has a parent");
        let parent_mode = std::fs::metadata(parent)
            .expect("read the socket directory metadata")
            .permissions()
            .mode()
            & 0o777;
        let socket_mode = std::fs::metadata(&socket)
            .expect("read the socket metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(parent_mode, 0o700, "the socket directory is owner-only");
        assert_eq!(socket_mode, 0o600, "the socket is owner-only");

        drop(session);
    }

    #[test]
    fn debug_socket_is_removed_when_the_session_drops() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let script = write_script(dir.path(), "debugger.sh", "sleep 30");
        let mut session = spawn_session(&spec, debug_command(&script));
        let socket = start_debug_bridge(&mut session);

        assert!(
            socket.exists(),
            "the debug socket exists while the session runs"
        );

        drop(session);

        assert!(!socket.exists(), "the debug socket is removed on drop");
    }

    #[test]
    fn debug_endpoint_rejects_native_without_a_pty() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let endpoints = BochsEndpoints { agent_serial: None };
        let mut session = session_with(spec, endpoints, child);

        match session.debug_endpoint(DebugKind::Native) {
            Err(EngineError::Unsupported("debugger")) => {}
            other => panic!("expected an unsupported debugger, got {other:?}"),
        }
    }

    #[test]
    fn debug_endpoint_rejects_gdb_without_a_gdbstub() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let spec = session_spec(dir.path(), false);
        let child = spawn_child(Path::new("sleep"), &["30"]);
        let endpoints = BochsEndpoints { agent_serial: None };
        let mut session = session_with(spec, endpoints, child);

        match session.debug_endpoint(DebugKind::Gdb) {
            Err(EngineError::Unsupported("debugger")) => {}
            other => panic!("expected an unsupported debugger, got {other:?}"),
        }
    }
}
