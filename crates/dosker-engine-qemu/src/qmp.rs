//! A minimal QEMU machine protocol client.

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use dosker_agent_proto::Transport;
use dosker_engine::{EngineEndpointKind, EngineError};
use serde_json::{Map, Value};

use crate::stream::EngineStream;

/// The time allowed for a QMP connection and handshake.
pub const QMP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A JSON line client for the QEMU machine protocol.
pub struct QmpClient {
    reader: BufReader<EngineStream>,
}

impl QmpClient {
    /// Connects to a QMP endpoint, reads the greeting, and enables
    /// capabilities.
    ///
    /// The connect itself is non-blocking on Unix and deadline-bound on TCP,
    /// so a QEMU that stops accepting connections cannot hold the caller past
    /// `timeout`. The read and write timeouts are set before the handshake.
    pub fn connect(kind: &EngineEndpointKind, timeout: Duration) -> Result<Self, EngineError> {
        Self::connect_until(kind, Instant::now() + timeout)
    }

    /// Connects to a QMP endpoint, finishing the handshake before `deadline`.
    ///
    /// This is the deadline-aware form used by a caller that has already
    /// captured an overall deadline, such as `QemuSession::stop`.
    pub fn connect_until(
        kind: &EngineEndpointKind,
        deadline: Instant,
    ) -> Result<Self, EngineError> {
        let mut stream = EngineStream::connect(kind, deadline)?;

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(connect_timeout());
        }

        // Keep the handshake inside the same deadline, but never pass a
        // sub-millisecond duration that the OS may round down to "no timeout".
        let handshake_timeout = remaining.max(Duration::from_millis(1));
        stream.set_read_timeout(Some(handshake_timeout))?;
        stream.set_write_timeout(Some(handshake_timeout))?;

        let mut client = Self {
            reader: BufReader::new(stream),
        };

        match client.read_object()? {
            Some(_greeting) => {}
            None => {
                return Err(EngineError::Protocol(
                    "QMP closed the connection before the greeting".to_owned(),
                ));
            }
        }

        client.execute("qmp_capabilities", None)?;
        Ok(client)
    }

    /// Restricts subsequent reads and writes to `timeout`.
    ///
    /// Connecting leaves the socket at the handshake deadline, so a command
    /// that runs later (for example `quit` after an agent shutdown) must
    /// refresh the I/O budget before it runs.
    pub fn set_io_timeout(&mut self, timeout: Duration) -> Result<(), EngineError> {
        let timeout = timeout.max(Duration::from_millis(1));
        let stream = self.reader.get_mut();
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(())
    }

    /// Runs one QMP command and returns its `return` value.
    pub fn execute(
        &mut self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<Value, EngineError> {
        self.send_command(command, arguments)?;

        loop {
            let value = self.read_object()?.ok_or_else(|| {
                EngineError::Protocol(format!("QMP closed the connection during {command}"))
            })?;

            if value.get("event").is_some() {
                continue;
            }

            if let Some(error) = value.get("error") {
                return Err(EngineError::Protocol(format!(
                    "QMP {command} failed: {}",
                    describe_error(error)
                )));
            }

            if let Some(result) = value.get("return") {
                return Ok(result.clone());
            }

            return Err(EngineError::Protocol(format!(
                "unexpected QMP response to {command}: {value}"
            )));
        }
    }

    /// Sends `quit` and accepts an EOF as a successful exit.
    pub fn quit(&mut self) -> Result<(), EngineError> {
        if let Err(error) = self.send_command("quit", None) {
            return if is_closed(&error) {
                Ok(())
            } else {
                Err(error)
            };
        }

        loop {
            match self.read_object() {
                Ok(Some(value)) => {
                    if value.get("event").is_some() {
                        continue;
                    }

                    if let Some(error) = value.get("error") {
                        return Err(EngineError::Protocol(format!(
                            "QMP quit failed: {}",
                            describe_error(error)
                        )));
                    }

                    return Ok(());
                }
                Ok(None) => return Ok(()),
                Err(error) if is_closed(&error) => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    }

    /// Captures the guest screen to a file.
    pub fn screendump(&mut self, path: &Path, format: Option<&str>) -> Result<(), EngineError> {
        let mut arguments = Map::new();
        arguments.insert(
            "filename".to_owned(),
            Value::String(path.to_string_lossy().into_owned()),
        );
        if let Some(format) = format {
            arguments.insert("format".to_owned(), Value::String(format.to_owned()));
        }

        self.execute("screendump", Some(Value::Object(arguments)))?;
        Ok(())
    }

    fn send_command(&mut self, command: &str, arguments: Option<Value>) -> Result<(), EngineError> {
        let mut object = Map::new();
        object.insert("execute".to_owned(), Value::String(command.to_owned()));
        if let Some(arguments) = arguments {
            object.insert("arguments".to_owned(), arguments);
        }

        let mut encoded = serde_json::to_vec(&Value::Object(object))
            .map_err(|error| EngineError::Protocol(format!("encode QMP command: {error}")))?;
        encoded.push(b'\n');

        let stream = self.reader.get_mut();
        stream.write_all(&encoded)?;
        stream.flush()?;
        Ok(())
    }

    fn read_object(&mut self) -> Result<Option<Value>, EngineError> {
        let mut line = String::new();
        let count = self.reader.read_line(&mut line)?;
        if count == 0 {
            return Ok(None);
        }

        let value = serde_json::from_str(line.trim())
            .map_err(|error| EngineError::Protocol(format!("invalid QMP response: {error}")))?;
        Ok(Some(value))
    }
}

fn connect_timeout() -> EngineError {
    EngineError::Io(io::Error::new(
        io::ErrorKind::TimedOut,
        "the QMP connection timed out",
    ))
}

fn describe_error(error: &Value) -> String {
    error
        .get("desc")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string())
}

fn is_closed(error: &EngineError) -> bool {
    let EngineError::Io(error) = error else {
        return false;
    };

    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    )
}
