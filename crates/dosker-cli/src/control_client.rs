//! A one-request client for the supervisor control endpoint.

use std::io::BufReader;
use std::time::Duration;

use dosker_platform::Endpoint;
use dosker_platform::control::connect;
use dosker_platform::endpoint::EndpointRecord;
use dosker_runtime::protocol::{FrameRead, Reply, Request, Response, read_frame, write_message};

use crate::error::CliError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const IO_MARGIN: Duration = Duration::from_secs(5);

/// Connects to an endpoint, sends one request, and reads one response.
pub fn request(
    endpoint: &EndpointRecord,
    request: &Request,
    timeout: Duration,
) -> Result<Response, CliError> {
    let endpoint = Endpoint::from(endpoint.clone());
    let mut connection = connect(&endpoint, CONNECT_TIMEOUT)?;
    let io_timeout = timeout.saturating_add(IO_MARGIN);
    connection.set_read_timeout(Some(io_timeout))?;
    connection.set_write_timeout(Some(io_timeout))?;
    write_message(&mut connection, request)?;

    let mut reader = BufReader::new(connection);
    let line = match read_frame(&mut reader) {
        Ok(FrameRead::Line(line)) => line,
        Ok(FrameRead::Eof) => {
            return Err(CliError::Message(String::from(
                "the supervisor closed the connection",
            )));
        }
        Err(error) => return Err(CliError::Message(error.to_string())),
    };

    serde_json::from_str(&line)
        .map_err(|error| CliError::Message(format!("invalid response: {error}")))
}

/// Sends one ping with a hard budget for both connect and I/O.
pub fn ping(endpoint: &EndpointRecord, budget: Duration) -> Result<(), CliError> {
    let endpoint = Endpoint::from(endpoint.clone());
    let mut connection = connect(&endpoint, budget)?;
    connection.set_read_timeout(Some(budget))?;
    connection.set_write_timeout(Some(budget))?;
    write_message(&mut connection, &Request::Ping)?;

    let mut reader = BufReader::new(connection);
    let line = match read_frame(&mut reader) {
        Ok(FrameRead::Line(line)) => line,
        Ok(FrameRead::Eof) => {
            return Err(CliError::Message(String::from(
                "the supervisor closed the connection",
            )));
        }
        Err(error) => return Err(CliError::Message(error.to_string())),
    };

    let response: Response = serde_json::from_str(&line)
        .map_err(|error| CliError::Message(format!("invalid response: {error}")))?;
    if response.ok && matches!(response.result, Some(Reply::Pong)) {
        Ok(())
    } else {
        Err(CliError::Message(String::from(
            "the supervisor did not answer the ping",
        )))
    }
}
