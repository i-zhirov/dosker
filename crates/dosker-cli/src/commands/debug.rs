//! `dosker debug`: expose the engine's native debugger.

use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_engine::{Capability, DebugKind};
use dosker_runtime::protocol::{ErrorCode, Reply, Request};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::adapter;
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

    let kind = if args.gdb {
        DebugKind::Gdb
    } else {
        DebugKind::Native
    };
    let capability = match kind {
        DebugKind::Gdb => Capability::GdbStub,
        DebugKind::Native => Capability::DebugNative,
    };

    let Some(capabilities) = adapter::declared_capabilities(&record.engine) else {
        return Err(CliError::Usage(format!(
            "engine {} is not available; supported engines are {}",
            record.engine,
            adapter::supported_engines_message()
        )));
    };
    if !capabilities.contains(capability) {
        return Err(CliError::Message(dosker_engine::capability_message(
            &record.engine,
            capability,
        )));
    }

    let request = Request::Debug { kind };
    let response = match control_client::request(endpoint, &request, DEBUG_TIMEOUT) {
        Ok(response) => response,
        Err(CliError::Platform(_)) => return Ok(EXIT_NOT_RUNNING),
        Err(error) => return Err(error),
    };

    if !response.ok {
        return match response.error {
            Some(error) => match error.code {
                ErrorCode::NotRunning => Ok(EXIT_NOT_RUNNING),
                // The pre-check above already proved the engine declares the
                // capability, so an unsupported reply here means the endpoint
                // is unavailable at run time, not that the capability is
                // missing. Report the supervisor's message.
                _ => Err(CliError::Message(error.message)),
            },
            None => Err(CliError::Message(String::from(
                "unexpected supervisor reply",
            ))),
        };
    }

    match (response.result, kind) {
        (Some(Reply::Debug { endpoint, .. }), DebugKind::Gdb) => {
            context.console.line(&format!("target remote {endpoint}"))?;
            Ok(EXIT_OK)
        }
        (Some(Reply::Debug { endpoint, .. }), DebugKind::Native) => {
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
