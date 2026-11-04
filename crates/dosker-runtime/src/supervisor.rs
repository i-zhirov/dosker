//! The per-container supervisor: one engine, one control endpoint, one record.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use dosker_core::{ContainerId, CoreError, Status};
use dosker_engine::{
    DebugKind, EngineAdapter, EngineError, EngineSession, EngineSpec, ExecRequest,
};
use dosker_image::{DEFAULT_BAUD, DEFAULT_CONTROL_PORT, write_config};
use dosker_platform::control::{ControlConnection, bind, remove_endpoint};
use dosker_platform::{Endpoint, EndpointRecord, PlatformError};

use crate::paths::RuntimePaths;
use crate::protocol::{
    ContainerSummary, CpDirection, DEFAULT_EXEC_TIMEOUT_MS, ErrorCode, FrameError, FrameRead,
    MAX_CP_BYTES, Reply, Request, RequestParse, Response, parse_request, read_frame_into,
    write_message,
};
use crate::record::ContainerRecord;
use crate::store::{StateStore, StoreError};

/// The default deadline for an engine to report readiness.
pub const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the supervisor waits for a client before polling the engine.
pub const ACCEPT_POLL: Duration = Duration::from_millis(50);

/// How long the supervisor waits for the next request from a client.
pub const SERVER_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How often an attached connection checks the container while it is idle.
pub const ATTACH_POLL: Duration = Duration::from_secs(1);

/// The inputs a supervisor needs to own one container.
#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    pub root: PathBuf,
    pub container_id: ContainerId,
    pub ready_timeout: Duration,
}

/// A failure while supervising a container.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Platform(#[from] PlatformError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Protocol(#[from] io::Error),
}

/// Owns one container from start to stop.
pub struct Supervisor {
    config: SupervisorConfig,
    adapter: Box<dyn EngineAdapter>,
}

impl Supervisor {
    /// Builds a supervisor for one container and engine adapter.
    pub fn new(config: SupervisorConfig, adapter: Box<dyn EngineAdapter>) -> Self {
        Self { config, adapter }
    }

    /// Runs the container until it stops, exits, or fails.
    ///
    /// Returns 0 for a stopped or exited container and 1 for a failed start.
    pub fn run(self) -> Result<u8, SupervisorError> {
        let Self {
            config,
            mut adapter,
        } = self;
        let store = StateStore::new(config.root.clone());
        let paths = RuntimePaths::new(config.root.clone());
        let mut record = store.load(&config.container_id)?;

        let endpoint = match record.control_endpoint() {
            Some(endpoint) => Endpoint::from(endpoint.clone()),
            None => {
                return Err(SupervisorError::Protocol(io::Error::new(
                    io::ErrorKind::NotFound,
                    "the container record has no control endpoint",
                )));
            }
        };

        supervise(
            &store,
            &paths,
            &mut record,
            &endpoint,
            &mut *adapter,
            &config,
        )
    }
}

struct EndpointGuard<'a> {
    endpoint: &'a Endpoint,
}

impl Drop for EndpointGuard<'_> {
    fn drop(&mut self) {
        let _ = remove_endpoint(self.endpoint);
    }
}

struct SupervisorState {
    record: ContainerRecord,
    session: Box<dyn EngineSession>,
    container_log: PathBuf,
    shutdown: bool,
    shutdown_code: u8,
    /// The number of workers still flushing terminal replies. The main loop
    /// must not return until every one has been written and `shutdown` is set.
    terminal_pending: usize,
}

fn lock_state(
    state: &Mutex<SupervisorState>,
) -> Result<MutexGuard<'_, SupervisorState>, SupervisorError> {
    state.lock().map_err(|_| {
        SupervisorError::Protocol(io::Error::other("the supervisor state lock is poisoned"))
    })
}

fn supervise(
    store: &StateStore,
    paths: &RuntimePaths,
    record: &mut ContainerRecord,
    endpoint: &Endpoint,
    adapter: &mut dyn EngineAdapter,
    config: &SupervisorConfig,
) -> Result<u8, SupervisorError> {
    record.pid = Some(std::process::id());
    record.transition(Status::Starting, Utc::now())?;
    record.engine_version = adapter.version().to_owned();
    store.save(record)?;

    ensure_logs(paths, &record.id)?;
    let serial_log = paths.serial_log(&record.id);

    let disk_path = match record.disk_path.clone() {
        Some(path) if path.is_file() => path,
        Some(path) => {
            let error = EngineError::Setup(format!(
                "the container disk is not a regular file: {}",
                path.display()
            ));
            return failed_start(store, record, &serial_log, &error, None);
        }
        None => {
            let error = EngineError::Setup(format!("the container {} has no disk path", record.id));
            return failed_start(store, record, &serial_log, &error, None);
        }
    };

    if !record.booter
        && let Err(error) = write_config(&disk_path, DEFAULT_CONTROL_PORT, DEFAULT_BAUD)
    {
        let error = EngineError::Setup(format!("cannot write the agent configuration: {error}"));
        return failed_start(store, record, &serial_log, &error, None);
    }

    if let Err(error) = adapter.probe() {
        return failed_start(store, record, &serial_log, &error, None);
    }

    let listener = match bind(endpoint) {
        Ok(listener) => listener,
        Err(error) => {
            let error = EngineError::Setup(format!("cannot bind the control endpoint: {error}"));
            return failed_start(store, record, &serial_log, &error, None);
        }
    };
    let _endpoint_guard = EndpointGuard { endpoint };

    let spec = EngineSpec {
        root: config.root.clone(),
        container_id: record.id.clone(),
        workdir: String::from(r"C:\"),
        env: BTreeMap::new(),
        cmd: record.cmd.clone(),
        expects_agent: !record.booter,
        log_dir: paths.container_dir(&record.id),
        disk_path: disk_path.clone(),
        booter: record.booter,
        display: record.display,
        vnc_port: record.vnc_port,
    };

    let command = match adapter.build_command(&spec) {
        Ok(command) => command,
        Err(error) => return failed_start(store, record, &serial_log, &error, None),
    };
    let rendered = command.render();
    record.engine_command = Some(rendered.clone());
    log_line(&paths.engine_log(&record.id), &format!("engine {rendered}"));

    let mut session = match adapter.start_with_command(&spec, &command) {
        Ok(session) => session,
        Err(error) => return failed_start(store, record, &serial_log, &error, None),
    };

    record.engine_pid = session.pid();
    record.engine_pgid = session.process_group();

    for endpoint in session.endpoints() {
        record.sockets.insert(
            endpoint.name,
            EndpointRecord::UnixSocket {
                path: endpoint.path,
            },
        );
    }

    // Persist the engine identities before waiting for readiness so a
    // supervisory timeout can always find and kill the engine group.
    store.save(record)?;

    if let Err(error) = session.ready(config.ready_timeout) {
        return failed_start(store, record, &serial_log, &error, Some(&mut session));
    }

    record.transition(Status::Running, Utc::now())?;
    store.save(record)?;

    if let Some(command) = spec.cmd.as_deref().filter(|_| !record.booter) {
        match session.exec(&ExecRequest {
            command: command.to_owned(),
            env: spec.env.clone(),
            workdir: spec.workdir.clone(),
            timeout_ms: DEFAULT_EXEC_TIMEOUT_MS,
        }) {
            Ok(result) => log_line(
                &paths.container_log(&record.id),
                &format!(
                    "cmd exit={} timed_out={} command={command}",
                    result.exit_code, result.timed_out
                ),
            ),
            Err(EngineError::ExecTimeout(_)) => {
                let _ = session.kill();
                record.exit_code = Some(124);
                mark_exited(store, record)?;
                return Ok(0);
            }
            Err(EngineError::GuestRebooted) => {
                let _ = session.kill();
                mark_exited(store, record)?;
                return Ok(0);
            }
            Err(error @ EngineError::Agent { .. })
                if engine_error_code(&error) == ErrorCode::Busy =>
            {
                log_line(
                    &paths.container_log(&record.id),
                    &format!("cmd busy command={command}"),
                );
            }
            Err(error) => {
                let _ = session.kill();
                log_line(
                    &paths.container_log(&record.id),
                    &format!("cmd failed: {error} command={command}"),
                );
                mark_exited(store, record)?;
                return Ok(0);
            }
        }
    }

    let container_log = paths.container_log(&record.id);
    let state = Arc::new(Mutex::new(SupervisorState {
        record: record.clone(),
        session,
        container_log,
        shutdown: false,
        shutdown_code: 0,
        terminal_pending: 0,
    }));

    loop {
        {
            let mut state = lock_state(&state)?;
            if state.shutdown {
                return Ok(state.shutdown_code);
            }
            if state.terminal_pending == 0 {
                match state.session.try_exit() {
                    Ok(Some(code)) => {
                        state.record.exit_code = Some(code);
                        mark_exited(store, &mut state.record)?;
                        return Ok(0);
                    }
                    Err(_) => {
                        mark_exited(store, &mut state.record)?;
                        return Ok(0);
                    }
                    Ok(None) => {}
                }
            }
        }

        match listener.accept(ACCEPT_POLL) {
            Ok(Some(connection)) => {
                let state = Arc::clone(&state);
                let store = store.clone();
                thread::spawn(move || {
                    let _ = serve(connection, &state, &store);
                });
            }
            Ok(None) => {}
            Err(error) => return Err(SupervisorError::Protocol(error)),
        }
    }
}

fn failed_start(
    store: &StateStore,
    record: &mut ContainerRecord,
    serial_log: &Path,
    error: &EngineError,
    session: Option<&mut Box<dyn EngineSession>>,
) -> Result<u8, SupervisorError> {
    if let Some(session) = session {
        let _ = session.kill();
    }

    let container_log = store.paths().container_log(&record.id);
    log_line(&container_log, &format!("start failed: {error}"));
    if matches!(error, EngineError::ReadyTimeout(_)) {
        match last_serial_line(serial_log) {
            Some(line) => log_line(
                &container_log,
                &format!("last serial line: {line}; retry with a larger --boot-timeout"),
            ),
            None => log_line(
                &container_log,
                "last serial line: <none>; retry with a larger --boot-timeout",
            ),
        }
    }

    record.transition(Status::Failed, Utc::now())?;
    record.pid = None;
    record.engine_pid = None;
    record.engine_pgid = None;
    store.save(record)?;
    Ok(1)
}

fn last_serial_line(path: &Path) -> Option<String> {
    let contents = std::fs::read(path).ok()?;

    contents
        .split(|byte| *byte == b'\r' || *byte == b'\n')
        .map(|segment| segment.trim_ascii())
        .rfind(|segment| !segment.is_empty())
        .map(|segment| String::from_utf8_lossy(segment).into_owned())
}

fn log_line(path: &Path, line: &str) {
    if let Err(error) = append_line(path, line) {
        eprintln!(
            "dosker: warning: cannot append to the log {}: {error}",
            path.display()
        );
    }
}

fn append_line(path: &Path, line: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    file.flush()
}

fn ensure_logs(paths: &RuntimePaths, id: &ContainerId) -> Result<(), SupervisorError> {
    for path in [
        paths.engine_log(id),
        paths.serial_log(id),
        paths.container_log(id),
    ] {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(SupervisorError::Protocol)?;
    }
    Ok(())
}

enum Dispatch {
    Reply(Response),
    Close(Response),
    /// Write the reply, then set the shutdown flag with this process code.
    ReplyThenShutdown(Response, u8),
    Attach(Response),
}

fn is_read_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

/// Marks a reply as one the supervisor must flush before it returns.
fn terminal(state: &mut SupervisorState, response: Response, code: u8) -> Dispatch {
    state.terminal_pending += 1;
    Dispatch::ReplyThenShutdown(response, code)
}

fn serve(
    mut connection: Box<dyn ControlConnection>,
    state: &Arc<Mutex<SupervisorState>>,
    store: &StateStore,
) -> Result<(), SupervisorError> {
    match connection.set_read_timeout(Some(SERVER_READ_TIMEOUT)) {
        Ok(()) => {}
        // A peer can close before the timeout is applied; on macOS that is
        // reported as InvalidInput. Drop the unusable connection and serve
        // the next one.
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            return Ok(());
        }
        Err(error) => return Err(SupervisorError::Protocol(error)),
    }
    let mut reader = BufReader::new(connection);
    let mut attached = false;
    let mut frame = Vec::new();

    loop {
        let line = match read_frame_into(&mut reader, &mut frame) {
            Ok(FrameRead::Line(line)) => line,
            Ok(FrameRead::Eof) => return Ok(()),
            Err(FrameError::Io(error)) if attached && is_read_timeout(&error) => {
                let mut state = lock_state(state)?;
                if state.shutdown {
                    return Ok(());
                }
                match state.session.try_exit() {
                    Ok(Some(_)) | Err(_) => return Ok(()),
                    Ok(None) => {}
                }
                drop(state);
                continue;
            }
            Err(_) => return Ok(()),
        };

        let dispatch = match parse_request(&line) {
            RequestParse::Known(request) => {
                let mut state = lock_state(state)?;
                dispatch_request(request, &mut state, store)?
            }
            RequestParse::UnknownType => Dispatch::Reply(Response::error(
                ErrorCode::UnknownMessage,
                "unknown message type",
            )),
            RequestParse::InvalidBody => Dispatch::Close(Response::error(
                ErrorCode::InvalidMessage,
                "invalid request body",
            )),
            RequestParse::Malformed => return Ok(()),
        };

        match dispatch {
            Dispatch::Reply(response) => {
                let _ = write_message(reader.get_mut(), &response);
            }
            Dispatch::Attach(response) => {
                let _ = write_message(reader.get_mut(), &response);
                attached = true;
                reader
                    .get_mut()
                    .set_read_timeout(Some(ATTACH_POLL))
                    .map_err(SupervisorError::Protocol)?;
            }
            Dispatch::Close(response) => {
                let _ = write_message(reader.get_mut(), &response);
                return Ok(());
            }
            Dispatch::ReplyThenShutdown(response, code) => {
                let _ = write_message(reader.get_mut(), &response);
                let mut state = lock_state(state)?;
                state.terminal_pending = state.terminal_pending.saturating_sub(1);
                if state.terminal_pending == 0 {
                    state.shutdown = true;
                    state.shutdown_code = code;
                }
                return Ok(());
            }
        }
    }
}

fn dispatch_request(
    request: Request,
    state: &mut SupervisorState,
    store: &StateStore,
) -> Result<Dispatch, SupervisorError> {
    match request {
        Request::Ping => Ok(Dispatch::Reply(Response::ok(Reply::Pong))),
        Request::Status => Ok(Dispatch::Reply(Response::ok(Reply::Status {
            container: summary(&state.record),
        }))),
        Request::Exec {
            command,
            env,
            workdir,
            timeout_ms,
        } => run_exec(
            state,
            store,
            ExecRequest {
                command,
                env,
                workdir,
                timeout_ms,
            },
        ),
        Request::Stop { grace_ms } => run_stop(state, store, grace_ms),
        Request::Attach => run_attach(state),
        Request::Cp {
            direction,
            source,
            destination,
        } => run_cp(state, direction, &source, &destination),
        Request::Debug { kind } => run_debug(state, kind),
        Request::Screenshot { path } => run_screenshot(state, &path),
    }
}

fn run_attach(state: &mut SupervisorState) -> Result<Dispatch, SupervisorError> {
    if state.record.status != Status::Running {
        return Ok(Dispatch::Reply(Response::error(
            ErrorCode::NotRunning,
            format!("container is {}", state.record.status),
        )));
    }
    if state.record.booter {
        return Ok(Dispatch::Reply(Response::error(
            ErrorCode::NoAgent,
            "the image has no agent",
        )));
    }
    Ok(Dispatch::Attach(Response::ok(Reply::Attach)))
}

fn run_exec(
    state: &mut SupervisorState,
    store: &StateStore,
    request: ExecRequest,
) -> Result<Dispatch, SupervisorError> {
    if state.record.status != Status::Running {
        return Ok(Dispatch::Reply(Response::error(
            ErrorCode::NotRunning,
            format!("container is {}", state.record.status),
        )));
    }
    if state.record.booter {
        return Ok(Dispatch::Reply(Response::error(
            ErrorCode::NoAgent,
            "the image has no agent",
        )));
    }

    match state.session.exec(&request) {
        Ok(result) => {
            append_exec_log(
                &state.container_log,
                result.exit_code,
                result.timed_out,
                &request.command,
            )
            .map_err(SupervisorError::Protocol)?;
            Ok(Dispatch::Reply(Response::ok(Reply::Exec {
                exit_code: result.exit_code,
                output: result.output,
                timed_out: result.timed_out,
                truncated: result.truncated,
            })))
        }
        Err(EngineError::ExecTimeout(_)) => {
            let _ = state.session.kill();
            mark_exited(store, &mut state.record)?;
            Ok(terminal(
                state,
                Response::ok(Reply::Exec {
                    exit_code: 124,
                    output: String::new(),
                    timed_out: true,
                    truncated: false,
                }),
                0,
            ))
        }
        Err(EngineError::GuestRebooted) => {
            let _ = state.session.kill();
            mark_exited(store, &mut state.record)?;
            Ok(terminal(
                state,
                error_response(EngineError::GuestRebooted),
                0,
            ))
        }
        Err(error @ EngineError::Agent { .. }) if engine_error_code(&error) == ErrorCode::Busy => {
            Ok(Dispatch::Reply(error_response(error)))
        }
        Err(error) => {
            let _ = state.session.kill();
            mark_exited(store, &mut state.record)?;
            Ok(terminal(state, error_response(error), 0))
        }
    }
}

fn run_cp(
    state: &mut SupervisorState,
    direction: CpDirection,
    source: &str,
    destination: &str,
) -> Result<Dispatch, SupervisorError> {
    if state.record.status != Status::Running {
        return Ok(Dispatch::Reply(Response::error(
            ErrorCode::NotRunning,
            format!("container is {}", state.record.status),
        )));
    }
    if state.record.booter {
        return Ok(Dispatch::Reply(Response::error(
            ErrorCode::NoAgent,
            "the image has no agent",
        )));
    }

    match direction {
        CpDirection::In => {
            let metadata = match std::fs::metadata(source) {
                Ok(metadata) => metadata,
                Err(error) => {
                    return Ok(Dispatch::Reply(Response::error(
                        ErrorCode::Internal,
                        format!("cannot inspect {source}: {error}"),
                    )));
                }
            };
            if metadata.len() > MAX_CP_BYTES {
                return Ok(Dispatch::Reply(Response::error(
                    ErrorCode::TooLarge,
                    format!("{source} is larger than {MAX_CP_BYTES} bytes"),
                )));
            }
            let data = match std::fs::read(source) {
                Ok(data) => data,
                Err(error) => {
                    return Ok(Dispatch::Reply(Response::error(
                        ErrorCode::Internal,
                        format!("cannot read {source}: {error}"),
                    )));
                }
            };
            match state.session.put_file(destination, &data) {
                Ok(bytes) => Ok(Dispatch::Reply(Response::ok(Reply::Cp { bytes }))),
                Err(error) => Ok(Dispatch::Reply(error_response(error))),
            }
        }
        CpDirection::Out => match state.session.get_file(source) {
            Ok(data) => {
                if data.len() as u64 > MAX_CP_BYTES {
                    return Ok(Dispatch::Reply(Response::error(
                        ErrorCode::TooLarge,
                        format!("{source} is larger than {MAX_CP_BYTES} bytes"),
                    )));
                }
                if let Err(error) = std::fs::write(destination, &data) {
                    return Ok(Dispatch::Reply(Response::error(
                        ErrorCode::Internal,
                        format!("cannot write {destination}: {error}"),
                    )));
                }
                Ok(Dispatch::Reply(Response::ok(Reply::Cp {
                    bytes: data.len() as u64,
                })))
            }
            Err(error) => Ok(Dispatch::Reply(error_response(error))),
        },
    }
}

fn run_debug(state: &mut SupervisorState, kind: DebugKind) -> Result<Dispatch, SupervisorError> {
    if state.record.status != Status::Running {
        return Ok(Dispatch::Reply(Response::error(
            ErrorCode::NotRunning,
            format!("container is {}", state.record.status),
        )));
    }
    match state.session.debug_endpoint(kind) {
        Ok(endpoint) => Ok(Dispatch::Reply(Response::ok(Reply::Debug {
            kind,
            endpoint,
        }))),
        Err(error) => Ok(Dispatch::Reply(error_response(error))),
    }
}

fn run_screenshot(state: &mut SupervisorState, path: &Path) -> Result<Dispatch, SupervisorError> {
    if state.record.status != Status::Running {
        return Ok(Dispatch::Reply(Response::error(
            ErrorCode::NotRunning,
            format!("container is {}", state.record.status),
        )));
    }
    match state.session.screenshot(path) {
        Ok(()) => Ok(Dispatch::Reply(Response::ok(Reply::Screenshot {
            path: path.to_path_buf(),
        }))),
        Err(error) => Ok(Dispatch::Reply(error_response(error))),
    }
}

fn run_stop(
    state: &mut SupervisorState,
    store: &StateStore,
    grace_ms: u64,
) -> Result<Dispatch, SupervisorError> {
    if matches!(
        state.record.status,
        Status::Stopped | Status::Exited | Status::Failed
    ) {
        let status = state.record.status;
        let code = if status == Status::Failed { 1 } else { 0 };
        return Ok(terminal(state, Response::ok(Reply::Stop { status }), code));
    }

    state.record.transition(Status::Stopping, Utc::now())?;
    store.save(&state.record)?;

    let status = match state.session.stop(Duration::from_millis(grace_ms)) {
        Ok(exit_code) => {
            state.record.transition(Status::Stopped, Utc::now())?;
            state.record.exit_code = Some(exit_code);
            state.record.dirty = false;
            Status::Stopped
        }
        Err(_) => {
            let _ = state.session.kill();
            state.record.transition(Status::Exited, Utc::now())?;
            state.record.dirty = true;
            Status::Exited
        }
    };

    state.record.pid = None;
    state.record.engine_pid = None;
    state.record.engine_pgid = None;
    store.save(&state.record)?;

    let code = if status == Status::Failed { 1 } else { 0 };
    Ok(terminal(state, Response::ok(Reply::Stop { status }), code))
}

fn error_response(error: EngineError) -> Response {
    Response::error(engine_error_code(&error), error.to_string())
}

fn engine_error_code(error: &EngineError) -> ErrorCode {
    match error {
        EngineError::NoAgent => ErrorCode::NoAgent,
        EngineError::Unsupported(_) => ErrorCode::Unsupported,
        EngineError::ExecTimeout(_) => ErrorCode::Timeout,
        EngineError::GuestRebooted => ErrorCode::GuestRebooted,
        EngineError::Agent { code, .. } if code == "TOO_LARGE" => ErrorCode::TooLarge,
        EngineError::Agent { code, .. } if code == "NO_FILE" => ErrorCode::NoFile,
        EngineError::Agent { code, .. } if code == "BUSY" => ErrorCode::Busy,
        _ => ErrorCode::EngineError,
    }
}

fn mark_exited(store: &StateStore, record: &mut ContainerRecord) -> Result<(), SupervisorError> {
    record.transition(Status::Exited, Utc::now())?;
    record.dirty = true;
    record.pid = None;
    record.engine_pid = None;
    record.engine_pgid = None;
    store.save(record)?;
    Ok(())
}

fn append_exec_log(path: &Path, exit_code: i32, timed_out: bool, command: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "exec exit={exit_code} timed_out={timed_out} command={command}"
    )?;
    file.flush()
}

fn summary(record: &ContainerRecord) -> ContainerSummary {
    ContainerSummary {
        id: record.id.clone(),
        name: record.name.clone(),
        image: record.image.clone(),
        status: record.status,
        pid: record.pid,
        engine_pid: record.engine_pid,
        exit_code: record.exit_code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_error_code_covers_each_typed_case() {
        assert_eq!(engine_error_code(&EngineError::NoAgent), ErrorCode::NoAgent);
        assert_eq!(
            engine_error_code(&EngineError::Unsupported("screenshot")),
            ErrorCode::Unsupported
        );
        assert_eq!(
            engine_error_code(&EngineError::ExecTimeout(Duration::from_secs(1))),
            ErrorCode::Timeout
        );
        assert_eq!(
            engine_error_code(&EngineError::GuestRebooted),
            ErrorCode::GuestRebooted
        );
        assert_eq!(
            engine_error_code(&EngineError::Agent {
                code: String::from("TOO_LARGE"),
                message: String::new(),
            }),
            ErrorCode::TooLarge
        );
        assert_eq!(
            engine_error_code(&EngineError::Agent {
                code: String::from("NO_FILE"),
                message: String::new(),
            }),
            ErrorCode::NoFile
        );
        assert_eq!(
            engine_error_code(&EngineError::Agent {
                code: String::from("BUSY"),
                message: String::new(),
            }),
            ErrorCode::Busy
        );
        assert_eq!(
            engine_error_code(&EngineError::Agent {
                code: String::from("OTHER"),
                message: String::new(),
            }),
            ErrorCode::EngineError
        );
    }
}
