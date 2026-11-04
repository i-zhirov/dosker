//! The JSON-lines control protocol spoken over the control endpoint.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;

use dosker_core::{ContainerId, ContainerName, Status};
use dosker_engine::DebugKind;
use serde::{Deserialize, Serialize};

/// The largest accepted control line, excluding its terminator.
pub const MAX_MESSAGE_BYTES: usize = 65_536;

/// The largest file accepted in either direction by `cp`.
pub const MAX_CP_BYTES: u64 = 16 * 1024 * 1024;

/// The direction of a `cp` transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpDirection {
    /// Copy from the host into the guest.
    In,
    /// Copy from the guest onto the host.
    Out,
}

/// The default deadline for a guest command.
pub const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;

/// The default grace period for a graceful stop.
pub const DEFAULT_STOP_GRACE_MS: u64 = 10_000;

/// A request sent from the CLI to the supervisor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Status,
    Exec {
        command: String,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default = "default_workdir")]
        workdir: String,
        #[serde(default = "default_exec_timeout_ms")]
        timeout_ms: u64,
    },
    Stop {
        #[serde(default = "default_stop_grace_ms")]
        grace_ms: u64,
    },
    Attach,
    Cp {
        direction: CpDirection,
        source: String,
        destination: String,
    },
    Debug {
        kind: DebugKind,
    },
    Screenshot {
        path: PathBuf,
    },
}

/// The result carried by a successful response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Pong,
    Status {
        container: ContainerSummary,
    },
    Exec {
        exit_code: i32,
        output: String,
        timed_out: bool,
        truncated: bool,
    },
    Stop {
        status: Status,
    },
    Attach,
    Cp {
        bytes: u64,
    },
    Debug {
        kind: DebugKind,
        endpoint: String,
    },
    Screenshot {
        path: PathBuf,
    },
}

/// The container fields reported by a status request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSummary {
    pub id: ContainerId,
    pub name: ContainerName,
    pub image: String,
    pub status: Status,
    pub pid: Option<u32>,
    pub engine_pid: Option<u32>,
    pub exit_code: Option<i32>,
}

/// A machine-readable reason a request failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnknownMessage,
    InvalidMessage,
    NotRunning,
    NoAgent,
    Unsupported,
    TooLarge,
    NoFile,
    GuestRebooted,
    Busy,
    EngineError,
    Timeout,
    Internal,
}

/// The error payload of a failed response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
}

/// A response sent from the supervisor to the CLI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Reply>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl Response {
    /// Builds a successful response carrying a reply.
    pub fn ok(reply: Reply) -> Self {
        Self {
            ok: true,
            result: Some(reply),
            error: None,
        }
    }

    /// Builds a failed response carrying an error code and message.
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            result: None,
            error: Some(ProtocolError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// One frame read from the control endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameRead {
    Line(String),
    Eof,
}

/// A failure while reading a frame.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("control line exceeds the size limit")]
    TooLarge,
    #[error("control line is not valid UTF-8")]
    NotUtf8,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The outcome of interpreting one request line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestParse {
    Known(Request),
    UnknownType,
    Malformed,
    InvalidBody,
}

/// Reads one non-blank line from the control endpoint.
///
/// A line is terminated by `\n` or `\r\n`. The terminator is not part of the
/// returned text, and at most [`MAX_MESSAGE_BYTES`] content bytes are accepted.
pub fn read_frame<R: BufRead>(reader: &mut R) -> Result<FrameRead, FrameError> {
    read_frame_into(reader, &mut Vec::new())
}

/// Reads one non-blank line, keeping a partial line in `buffer`.
///
/// A caller that retries after a read timeout must pass the same buffer so the
/// bytes already received are not discarded. The buffer is empty after a
/// complete frame, a frame error, or end of input; an I/O error leaves the
/// partial bytes in place.
pub fn read_frame_into<R: BufRead>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<FrameRead, FrameError> {
    loop {
        if buffer.len() > MAX_MESSAGE_BYTES {
            buffer.clear();
            return Err(FrameError::TooLarge);
        }
        let limit = (MAX_MESSAGE_BYTES + 2 - buffer.len()) as u64;
        let read = reader.by_ref().take(limit).read_until(b'\n', buffer)?;

        if read == 0 {
            buffer.clear();
            return Ok(FrameRead::Eof);
        }

        let terminated = buffer.last() == Some(&b'\n');
        if terminated {
            buffer.pop();
        }
        if buffer.last() == Some(&b'\r') {
            buffer.pop();
        }
        if buffer.len() > MAX_MESSAGE_BYTES {
            buffer.clear();
            return Err(FrameError::TooLarge);
        }
        if buffer.is_empty() {
            if terminated {
                continue;
            }
            return Ok(FrameRead::Eof);
        }

        let bytes = std::mem::take(buffer);
        let text = String::from_utf8(bytes).map_err(|_| FrameError::NotUtf8)?;
        return Ok(FrameRead::Line(text));
    }
}

/// Interprets one request line.
///
/// Invalid JSON and non-object values are malformed. A missing or unrecognized
/// `type` is unknown. A recognized type with a bad body is invalid.
pub fn parse_request(line: &str) -> RequestParse {
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return RequestParse::Malformed,
    };

    let Some(object) = value.as_object() else {
        return RequestParse::Malformed;
    };

    let Some(kind) = object.get("type").and_then(serde_json::Value::as_str) else {
        return RequestParse::UnknownType;
    };

    if !matches!(
        kind,
        "ping" | "status" | "exec" | "stop" | "attach" | "cp" | "debug" | "screenshot"
    ) {
        return RequestParse::UnknownType;
    }

    match serde_json::from_value(value) {
        Ok(request) => RequestParse::Known(request),
        Err(_) => RequestParse::InvalidBody,
    }
}

/// Writes one compact JSON object followed by a newline, then flushes.
pub fn write_message<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::from)?;
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    writer.flush()
}

fn default_workdir() -> String {
    "C:\\".to_owned()
}

fn default_exec_timeout_ms() -> u64 {
    DEFAULT_EXEC_TIMEOUT_MS
}

fn default_stop_grace_ms() -> u64 {
    DEFAULT_STOP_GRACE_MS
}
