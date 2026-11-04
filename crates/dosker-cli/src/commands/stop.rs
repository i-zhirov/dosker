//! `dosker stop`: stop a container through its supervisor.

use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::{ContainerId, Status};
use dosker_runtime::protocol::{Reply, Request};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{EXIT_ERROR, EXIT_NOT_RUNNING, EXIT_OK, StopArgs};
use crate::context::Context;
use crate::control_client;
use crate::error::CliError;
use crate::resolve::resolve;

pub fn execute(
    args: StopArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;

    if is_terminal(record.status) {
        context
            .console
            .line(&format!("{} {}", record.name, record.status))?;
        return Ok(EXIT_OK);
    }

    // A created container has no supervisor and has never run, so there is
    // nothing to stop. Match the pinned not-running exit code.
    if record.status == Status::Created {
        context
            .console
            .line(&format!("{} {}", record.name, record.status))?;
        return Ok(EXIT_NOT_RUNNING);
    }

    let Some(endpoint) = record.control_endpoint() else {
        return recover_after_connect_failure(context, probe, &record.id, now);
    };

    let request = Request::Stop {
        grace_ms: args.time.saturating_mul(1000),
    };

    match control_client::request(endpoint, &request, Duration::from_secs(args.time)) {
        Ok(response) => match response.result {
            Some(Reply::Stop { status }) => {
                context
                    .console
                    .line(&format!("{} {}", record.name, status))?;
                Ok(EXIT_OK)
            }
            _ => {
                let message = response
                    .error
                    .map(|error| error.message)
                    .unwrap_or_else(|| String::from("unexpected supervisor reply"));
                Err(CliError::Message(message))
            }
        },
        Err(_) => recover_after_connect_failure(context, probe, &record.id, now),
    }
}

fn is_terminal(status: Status) -> bool {
    matches!(status, Status::Stopped | Status::Exited | Status::Failed)
}

fn recover_after_connect_failure(
    context: &mut Context,
    probe: &dyn HostProbe,
    id: &ContainerId,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(id)?;
    context
        .console
        .line(&format!("{} {}", record.name, record.status))?;
    if record.status == Status::Exited {
        Ok(EXIT_OK)
    } else {
        Ok(EXIT_ERROR)
    }
}
