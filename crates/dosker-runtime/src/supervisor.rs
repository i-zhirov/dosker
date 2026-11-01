//! The per-container supervisor: one engine, one control endpoint, one record.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use dosker_core::{ContainerId, CoreError, Status};
use dosker_engine::{EngineAdapter, EngineError, EngineSession, EngineSpec, ExecRequest};
use dosker_platform::control::{ControlConnection, bind, remove_endpoint};
use dosker_platform::{Endpoint, PlatformError};

use crate::paths::RuntimePaths;
use crate::protocol::{
    ContainerSummary, ErrorCode, FrameRead, Reply, Request, RequestParse, Response, parse_request,
    read_frame, write_message,
};
use crate::record::ContainerRecord;
use crate::store::{StateStore, StoreError};

/// The default deadline for an engine to report readiness.
pub const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the supervisor waits for a client before polling the engine.
pub const ACCEPT_POLL: Duration = Duration::from_millis(50);

/// How long the supervisor waits for the next request from a client.
pub const SERVER_READ_TIMEOUT: Duration = Duration::from_secs(5);

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

    if adapter.probe().is_err() {
        return failed_start(store, record, None);
    }

    ensure_logs(paths, &record.id)?;

    let listener = match bind(endpoint) {
        Ok(listener) => listener,
        Err(_) => return failed_start(store, record, None),
    };
    let _endpoint_guard = EndpointGuard { endpoint };

    let spec = EngineSpec {
        container_id: record.id.clone(),
        workdir: String::from(r"C:\"),
        env: BTreeMap::new(),
        cmd: None,
        expects_agent: false,
        log_dir: paths.container_dir(&record.id),
    };

    let mut session = match adapter.start(&spec) {
        Ok(session) => session,
        Err(_) => return failed_start(store, record, None),
    };

    record.engine_pid = session.pid();
    record.engine_pgid = session.process_group();

    if session.ready(config.ready_timeout).is_err() {
        return failed_start(store, record, Some(&mut session));
    }

    record.transition(Status::Running, Utc::now())?;
    store.save(record)?;

    let container_log = paths.container_log(&record.id);

    loop {
        match session.try_exit() {
            Ok(Some(code)) => {
                record.exit_code = Some(code);
                mark_exited(store, record)?;
                return Ok(0);
            }
            Err(_) => {
                mark_exited(store, record)?;
                return Ok(0);
            }
            Ok(None) => {}
        }

        match listener.accept(ACCEPT_POLL) {
            Ok(Some(connection)) => {
                if let ConnectionOutcome::Shutdown(code) =
                    serve(connection, record, &mut *session, store, &container_log)?
                {
                    return Ok(code);
                }
            }
            Ok(None) => {}
            Err(error) => return Err(SupervisorError::Protocol(error)),
        }
    }
}

fn failed_start(
    store: &StateStore,
    record: &mut ContainerRecord,
    session: Option<&mut Box<dyn EngineSession>>,
) -> Result<u8, SupervisorError> {
    if let Some(session) = session {
        let _ = session.kill();
    }
    record.transition(Status::Failed, Utc::now())?;
    record.pid = None;
    record.engine_pid = None;
    record.engine_pgid = None;
    store.save(record)?;
    Ok(1)
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

enum ConnectionOutcome {
    Continue,
    Shutdown(u8),
}

enum Dispatch {
    Reply(Response),
    Close(Response),
    Shutdown(Response, u8),
}

fn serve(
    mut connection: Box<dyn ControlConnection>,
    record: &mut ContainerRecord,
    session: &mut dyn EngineSession,
    store: &StateStore,
    container_log: &Path,
) -> Result<ConnectionOutcome, SupervisorError> {
    match connection.set_read_timeout(Some(SERVER_READ_TIMEOUT)) {
        Ok(()) => {}
        // A peer can close before the timeout is applied; on macOS that is
        // reported as InvalidInput. Drop the unusable connection and serve
        // the next one.
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            return Ok(ConnectionOutcome::Continue);
        }
        Err(error) => return Err(SupervisorError::Protocol(error)),
    }
    let mut reader = BufReader::new(connection);

    loop {
        let line = match read_frame(&mut reader) {
            Ok(FrameRead::Line(line)) => line,
            Ok(FrameRead::Eof) => return Ok(ConnectionOutcome::Continue),
            Err(_) => return Ok(ConnectionOutcome::Continue),
        };

        let dispatch = match parse_request(&line) {
            RequestParse::Known(request) => {
                dispatch_request(request, record, session, store, container_log)?
            }
            RequestParse::UnknownType => Dispatch::Reply(Response::error(
                ErrorCode::UnknownMessage,
                "unknown message type",
            )),
            RequestParse::InvalidBody => Dispatch::Close(Response::error(
                ErrorCode::InvalidMessage,
                "invalid request body",
            )),
            RequestParse::Malformed => return Ok(ConnectionOutcome::Continue),
        };

        match dispatch {
            Dispatch::Reply(response) => {
                let _ = write_message(reader.get_mut(), &response);
            }
            Dispatch::Close(response) => {
                let _ = write_message(reader.get_mut(), &response);
                return Ok(ConnectionOutcome::Continue);
            }
            Dispatch::Shutdown(response, code) => {
                let _ = write_message(reader.get_mut(), &response);
                return Ok(ConnectionOutcome::Shutdown(code));
            }
        }
    }
}

fn dispatch_request(
    request: Request,
    record: &mut ContainerRecord,
    session: &mut dyn EngineSession,
    store: &StateStore,
    container_log: &Path,
) -> Result<Dispatch, SupervisorError> {
    match request {
        Request::Ping => Ok(Dispatch::Reply(Response::ok(Reply::Pong))),
        Request::Status => Ok(Dispatch::Reply(Response::ok(Reply::Status {
            container: summary(record),
        }))),
        Request::Exec {
            command,
            env,
            workdir,
            timeout_ms,
        } => run_exec(
            record,
            session,
            store,
            container_log,
            ExecRequest {
                command,
                env,
                workdir,
                timeout_ms,
            },
        ),
        Request::Stop { grace_ms } => run_stop(record, session, store, grace_ms),
    }
}

fn run_exec(
    record: &mut ContainerRecord,
    session: &mut dyn EngineSession,
    store: &StateStore,
    container_log: &Path,
    request: ExecRequest,
) -> Result<Dispatch, SupervisorError> {
    if record.status != Status::Running {
        return Ok(Dispatch::Reply(Response::error(
            ErrorCode::NotRunning,
            format!("container is {}", record.status),
        )));
    }

    match session.exec(&request) {
        Ok(result) => {
            append_exec_log(
                container_log,
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
            let _ = session.kill();
            mark_exited(store, record)?;
            Ok(Dispatch::Shutdown(
                Response::ok(Reply::Exec {
                    exit_code: 124,
                    output: String::new(),
                    timed_out: true,
                    truncated: false,
                }),
                0,
            ))
        }
        Err(_) => {
            mark_exited(store, record)?;
            Ok(Dispatch::Shutdown(
                Response::error(ErrorCode::EngineError, "engine command failed"),
                0,
            ))
        }
    }
}

fn run_stop(
    record: &mut ContainerRecord,
    session: &mut dyn EngineSession,
    store: &StateStore,
    grace_ms: u64,
) -> Result<Dispatch, SupervisorError> {
    if matches!(
        record.status,
        Status::Stopped | Status::Exited | Status::Failed
    ) {
        let status = record.status;
        let code = if status == Status::Failed { 1 } else { 0 };
        return Ok(Dispatch::Shutdown(
            Response::ok(Reply::Stop { status }),
            code,
        ));
    }

    record.transition(Status::Stopping, Utc::now())?;
    store.save(record)?;

    let status = match session.stop(Duration::from_millis(grace_ms)) {
        Ok(exit_code) => {
            record.transition(Status::Stopped, Utc::now())?;
            record.exit_code = Some(exit_code);
            record.dirty = false;
            Status::Stopped
        }
        Err(_) => {
            let _ = session.kill();
            record.transition(Status::Exited, Utc::now())?;
            record.dirty = true;
            Status::Exited
        }
    };

    record.pid = None;
    record.engine_pid = None;
    record.engine_pgid = None;
    store.save(record)?;

    let code = if status == Status::Failed { 1 } else { 0 };
    Ok(Dispatch::Shutdown(
        Response::ok(Reply::Stop { status }),
        code,
    ))
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
