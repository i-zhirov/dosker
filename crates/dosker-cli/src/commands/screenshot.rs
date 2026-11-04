//! `dosker screenshot`: capture the guest screen.

use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_runtime::protocol::{ErrorCode, Reply, Request};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{EXIT_NOT_RUNNING, EXIT_OK, ScreenshotArgs};
use crate::context::Context;
use crate::control_client;
use crate::error::CliError;
use crate::resolve::resolve;

const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(30);

pub fn execute(
    args: ScreenshotArgs,
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

    let path = std::path::absolute(&args.output)?;
    let request = Request::Screenshot { path: path.clone() };

    let response = match control_client::request(endpoint, &request, SCREENSHOT_TIMEOUT) {
        Ok(response) => response,
        Err(CliError::Platform(_)) => return Ok(EXIT_NOT_RUNNING),
        Err(error) => return Err(error),
    };

    if !response.ok {
        return match response.error {
            Some(error) => match error.code {
                ErrorCode::Unsupported => Err(CliError::Message(String::from(
                    "screenshot is not supported by this engine",
                ))),
                ErrorCode::NotRunning => Ok(EXIT_NOT_RUNNING),
                _ => Err(CliError::Message(error.message)),
            },
            None => Err(CliError::Message(String::from(
                "unexpected supervisor reply",
            ))),
        };
    }

    match response.result {
        Some(Reply::Screenshot { path }) => {
            context
                .console
                .line(&format!("screenshot {}", path.display()))?;
            Ok(EXIT_OK)
        }
        _ => Err(CliError::Message(String::from(
            "unexpected supervisor reply",
        ))),
    }
}
