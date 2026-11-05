//! `dosker attach`: open a line shell on a running container.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_platform::Endpoint;
use dosker_platform::console::Console;
use dosker_platform::control::{ControlConnection, connect};
use dosker_platform::endpoint::EndpointRecord;
use dosker_runtime::protocol::{
    ErrorCode, FrameError, FrameRead, Reply, Request, Response, read_frame, write_message,
};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{AttachArgs, EXIT_NOT_RUNNING, EXIT_OK, EXIT_TIMEOUT};
use crate::context::Context;
use crate::control_client;
use crate::error::CliError;
use crate::resolve::resolve;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The deadline for one command entered in an attached shell.
pub const ATTACH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// The action a line of shell input asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellAction {
    /// Run a command in the shell workdir.
    Exec { command: String, workdir: String },
    /// Leave the shell without stopping the container.
    Detach,
}

/// The local state of an attached line shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineShell {
    workdir: String,
}

impl LineShell {
    /// Starts a shell in a workdir.
    pub fn new(workdir: &str) -> Self {
        Self {
            workdir: workdir.to_owned(),
        }
    }

    /// Interprets one input line.
    pub fn handle(&mut self, line: &str) -> ShellAction {
        let line = line.trim();

        if line.eq_ignore_ascii_case("exit")
            || line.eq_ignore_ascii_case("quit")
            || line.eq_ignore_ascii_case("detach")
        {
            return ShellAction::Detach;
        }

        if line.is_empty() {
            return ShellAction::Exec {
                command: String::new(),
                workdir: self.workdir.clone(),
            };
        }

        let mut words = line.splitn(2, char::is_whitespace);
        let head = words.next().unwrap_or_default();
        if head.eq_ignore_ascii_case("cd") {
            let path = words.next().unwrap_or_default().trim();
            if !path.is_empty() {
                self.workdir = path.to_owned();
            }
            return ShellAction::Exec {
                command: String::new(),
                workdir: self.workdir.clone(),
            };
        }

        ShellAction::Exec {
            command: line.to_owned(),
            workdir: self.workdir.clone(),
        }
    }
}

pub fn execute(
    args: AttachArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;

    if record.status != Status::Running {
        return Ok(EXIT_NOT_RUNNING);
    }

    let Some(endpoint) = record.control_endpoint() else {
        return Ok(EXIT_NOT_RUNNING);
    };

    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    run_shell(endpoint, r"C:\", &mut input, &mut context.console)
}

/// Runs the attached line shell over an existing control endpoint.
pub fn run_shell(
    endpoint: &EndpointRecord,
    workdir: &str,
    mut input: impl BufRead,
    output: &mut Console,
) -> Result<u8, CliError> {
    let endpoint = Endpoint::try_from(endpoint.clone())?;
    let mut connection = match connect(&endpoint, CONNECT_TIMEOUT) {
        Ok(connection) => connection,
        Err(_) => return Ok(EXIT_NOT_RUNNING),
    };
    let io_timeout = ATTACH_COMMAND_TIMEOUT.saturating_add(control_client::IO_MARGIN);
    connection.set_read_timeout(Some(io_timeout))?;
    connection.set_write_timeout(Some(io_timeout))?;

    let mut reader = BufReader::new(connection);
    write_message(reader.get_mut(), &Request::Attach)?;
    match read_response(&mut reader)? {
        ReadOutcome::Response(response)
            if response.ok && matches!(response.result, Some(Reply::Attach)) => {}
        ReadOutcome::Response(response) => return handle_error(response, output),
        ReadOutcome::TimedOut => return Ok(EXIT_TIMEOUT),
    }

    let mut shell = LineShell::new(workdir);
    let mut line = String::new();
    loop {
        output.raw(r"C:\>")?;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break;
        }

        match shell.handle(&line) {
            ShellAction::Detach => break,
            ShellAction::Exec { command, .. } if command.is_empty() => {}
            ShellAction::Exec { command, workdir } => {
                let request = Request::Exec {
                    command,
                    env: BTreeMap::new(),
                    workdir,
                    timeout_ms: u64::try_from(ATTACH_COMMAND_TIMEOUT.as_millis())
                        .unwrap_or(u64::MAX),
                };
                write_message(reader.get_mut(), &request)?;
                let response = match read_response(&mut reader)? {
                    ReadOutcome::Response(response) => response,
                    ReadOutcome::TimedOut => return Ok(EXIT_TIMEOUT),
                };
                if !response.ok {
                    return handle_error(response, output);
                }
                match response.result {
                    Some(Reply::Exec {
                        output: text,
                        timed_out,
                        ..
                    }) => {
                        write_guest_output(output, &text)?;
                        if timed_out {
                            return Ok(EXIT_TIMEOUT);
                        }
                    }
                    _ => {
                        return Err(CliError::Message(String::from(
                            "unexpected supervisor reply",
                        )));
                    }
                }
            }
        }
    }

    Ok(EXIT_OK)
}

enum ReadOutcome {
    Response(Response),
    TimedOut,
}

fn read_response(
    reader: &mut BufReader<Box<dyn ControlConnection>>,
) -> Result<ReadOutcome, CliError> {
    match read_frame(reader) {
        Ok(FrameRead::Line(line)) => serde_json::from_str(&line)
            .map(ReadOutcome::Response)
            .map_err(|error| CliError::Message(format!("invalid response: {error}"))),
        Ok(FrameRead::Eof) => Err(CliError::Message(String::from(
            "the supervisor closed the connection",
        ))),
        Err(FrameError::Io(error)) if is_timeout(&error) => Ok(ReadOutcome::TimedOut),
        Err(error) => Err(CliError::Message(error.to_string())),
    }
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn handle_error(response: Response, output: &mut Console) -> Result<u8, CliError> {
    let Some(error) = response.error else {
        return Err(CliError::Message(String::from(
            "unexpected supervisor reply",
        )));
    };

    match error.code {
        ErrorCode::NoAgent => {
            output.error_line(&error.message)?;
            Ok(EXIT_NOT_RUNNING)
        }
        ErrorCode::NotRunning => Ok(EXIT_NOT_RUNNING),
        _ => Err(CliError::Message(error.message)),
    }
}

fn write_guest_output(output: &mut Console, text: &str) -> Result<(), CliError> {
    output.raw(text)?;
    Ok(())
}
