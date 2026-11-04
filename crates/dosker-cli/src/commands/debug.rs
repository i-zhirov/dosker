//! `dosker debug`: expose the engine's native debugger.

use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_engine::DebugKind;
use dosker_runtime::protocol::{ErrorCode, Reply, Request};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{DebugArgs, EXIT_NOT_RUNNING, EXIT_OK};
use crate::context::Context;
use crate::control_client;
use crate::error::CliError;
use crate::resolve::resolve;

const DEBUG_TIMEOUT: Duration = Duration::from_secs(5);

pub fn execute(
    args: DebugArgs,
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

    let request = Request::Debug {
        kind: DebugKind::Gdb,
    };
    let response = match control_client::request(endpoint, &request, DEBUG_TIMEOUT) {
        Ok(response) => response,
        Err(CliError::Platform(_)) => return Ok(EXIT_NOT_RUNNING),
        Err(error) => return Err(error),
    };

    if !response.ok {
        return match response.error {
            Some(error) => match error.code {
                ErrorCode::Unsupported => {
                    let mut message =
                        format!("debug is not supported by the {} engine", record.engine);
                    if !record.engine.eq_ignore_ascii_case("qemu") {
                        message.push_str("; the QEMU engine provides this capability");
                    }
                    Err(CliError::Message(message))
                }
                ErrorCode::NotRunning => Ok(EXIT_NOT_RUNNING),
                _ => Err(CliError::Message(error.message)),
            },
            None => Err(CliError::Message(String::from(
                "unexpected supervisor reply",
            ))),
        };
    }

    match response.result {
        Some(Reply::Debug { endpoint, .. }) if args.gdb => {
            context.console.line(&format!("target remote {endpoint}"))?;
            Ok(EXIT_OK)
        }
        Some(Reply::Debug { endpoint, .. }) => {
            context
                .console
                .line(&format!("native debugger {endpoint}"))?;
            Ok(EXIT_OK)
        }
        _ => Err(CliError::Message(String::from(
            "unexpected supervisor reply",
        ))),
    }
}
