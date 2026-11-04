//! `dosker cp`: copy one file between the host and a running container.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_runtime::protocol::{CpDirection, ErrorCode, MAX_CP_BYTES, Reply, Request, Response};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;
use dosker_runtime::{ContainerRecord, StoreError};

use crate::cli::{CpArgs, EXIT_NOT_RUNNING, EXIT_OK};
use crate::context::Context;
use crate::control_client;
use crate::error::CliError;
use crate::resolve::{Selector, not_found, parse_selector};

/// The deadline for one file transfer.
pub const CP_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// One side of a `cp` command.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum CpOperand {
    /// A path inside a resolved container.
    Container {
        record: ContainerRecord,
        path: String,
    },
    /// A path on the host.
    Host(PathBuf),
}

/// Parses one `cp` operand.
///
/// A value with a syntactically valid container reference before its first
/// colon is a container operand. A missing container is reported as such
/// instead of falling back to a host path. A Windows drive-letter path is
/// always a host path.
pub fn parse_operand(context: &Context, value: &str) -> Result<CpOperand, CliError> {
    if is_windows_drive_path(value) {
        return Ok(CpOperand::Host(PathBuf::from(value)));
    }

    if let Some((prefix, path)) = value.split_once(':')
        && !prefix.is_empty()
        && let Ok(selector) = parse_selector(prefix)
    {
        let record = match selector {
            Selector::Id(id) => match context.store.load(&id) {
                Ok(record) => record,
                Err(StoreError::NotFound(_)) => return Err(not_found(prefix)),
                Err(error) => return Err(error.into()),
            },
            Selector::Name(name) => match context.store.find_by_name(&name)? {
                Some(record) => record,
                None => return Err(not_found(prefix)),
            },
        };
        if path.is_empty() {
            return Err(CliError::Usage(String::from(
                "a container cp operand needs a path after the colon",
            )));
        }
        return Ok(CpOperand::Container {
            record,
            path: path.to_owned(),
        });
    }

    Ok(CpOperand::Host(PathBuf::from(value)))
}

fn is_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

pub fn execute(
    args: CpArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let source = parse_operand(context, &args.source)?;
    let destination = parse_operand(context, &args.destination)?;

    let (record, direction, host_path, guest_path) = match (source, destination) {
        (CpOperand::Container { record, path }, CpOperand::Host(host_path)) => {
            (record, CpDirection::Out, host_path, path)
        }
        (CpOperand::Host(host_path), CpOperand::Container { record, path }) => {
            (record, CpDirection::In, host_path, path)
        }
        _ => {
            return Err(CliError::Usage(String::from(
                "exactly one cp operand must name a container",
            )));
        }
    };

    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;
    if record.status != Status::Running {
        return Ok(EXIT_NOT_RUNNING);
    }
    let Some(endpoint) = record.control_endpoint() else {
        return Ok(EXIT_NOT_RUNNING);
    };

    let host_path = std::path::absolute(&host_path)?;
    match direction {
        CpDirection::In => check_source_file(&host_path)?,
        CpDirection::Out => check_destination_parent(&host_path)?,
    }

    let host = host_path.to_string_lossy().into_owned();
    let (source, destination) = match direction {
        CpDirection::In => (host, guest_path),
        CpDirection::Out => (guest_path, host),
    };
    let request = Request::Cp {
        direction,
        source,
        destination,
    };

    let response = match control_client::request(endpoint, &request, CP_TIMEOUT) {
        Ok(response) => response,
        Err(CliError::Platform(_)) => return Ok(EXIT_NOT_RUNNING),
        Err(error) => return Err(error),
    };

    if !response.ok {
        return handle_error(response, &mut context.console);
    }

    match response.result {
        Some(Reply::Cp { bytes }) => {
            context.console.line(&format!("copied {bytes} bytes"))?;
            Ok(EXIT_OK)
        }
        _ => Err(CliError::Message(String::from(
            "unexpected supervisor reply",
        ))),
    }
}

fn check_source_file(path: &Path) -> Result<(), CliError> {
    let metadata = fs::metadata(path).map_err(|error| io_error(path, error))?;
    if !metadata.is_file() {
        return Err(io_error(
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"),
        ));
    }
    if metadata.len() > MAX_CP_BYTES {
        return Err(CliError::Message(format!(
            "{} is larger than {MAX_CP_BYTES} bytes",
            path.display()
        )));
    }
    Ok(())
}

fn check_destination_parent(path: &Path) -> Result<(), CliError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let metadata = fs::metadata(parent).map_err(|error| io_error(parent, error))?;
    if !metadata.is_dir() {
        return Err(CliError::Message(format!(
            "{} is not a directory",
            parent.display()
        )));
    }
    Ok(())
}

fn io_error(path: &Path, error: io::Error) -> CliError {
    CliError::Io(io::Error::new(
        error.kind(),
        format!("cannot access {}: {error}", path.display()),
    ))
}

fn handle_error(
    response: Response,
    console: &mut dosker_platform::console::Console,
) -> Result<u8, CliError> {
    let Some(error) = response.error else {
        return Err(CliError::Message(String::from(
            "unexpected supervisor reply",
        )));
    };

    match error.code {
        ErrorCode::NoAgent => {
            console.error_line(&error.message)?;
            Ok(EXIT_NOT_RUNNING)
        }
        ErrorCode::NotRunning => Ok(EXIT_NOT_RUNNING),
        _ => Err(CliError::Message(error.message)),
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::is_windows_drive_path;

    #[test]
    fn windows_drive_paths_are_recognized_on_a_non_windows_host() {
        assert!(is_windows_drive_path(r"C:\TEMP\FILE.TXT"));
        assert!(is_windows_drive_path("C:/TEMP/FILE.TXT"));
        assert!(is_windows_drive_path("z:\\FILE.TXT"));
    }

    #[test]
    fn a_host_path_without_a_drive_slash_is_not_a_windows_drive_path() {
        assert!(!is_windows_drive_path("/tmp/file"));
        assert!(!is_windows_drive_path("file.txt"));
        assert!(!is_windows_drive_path("c:file"));
        assert!(!is_windows_drive_path("1:\\file"));
        assert!(!is_windows_drive_path(""));
    }
}
