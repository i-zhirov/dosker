//! `dosker status`: report one container.

use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_runtime::protocol::{Reply, Request};
use dosker_runtime::stale::HostProbe;
use dosker_runtime::{ContainerRecord, recover_stale};

use crate::cli::{EXIT_OK, StatusArgs};
use crate::context::Context;
use crate::control_client;
use crate::error::CliError;
use crate::resolve::resolve;

const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

pub fn execute(
    args: StatusArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;

    let (status, pid, engine_pid) = report(&record)?;

    let pid = pid.map_or_else(|| String::from("-"), |value| value.to_string());
    let engine_pid = engine_pid.map_or_else(|| String::from("-"), |value| value.to_string());

    context.console.line(&format!(
        "{} {} {} pid={} engine_pid={}",
        record.id, record.name, status, pid, engine_pid
    ))?;

    Ok(EXIT_OK)
}

fn report(
    record: &ContainerRecord,
) -> Result<(dosker_core::Status, Option<u32>, Option<u32>), CliError> {
    if !record.status.is_active() {
        return Ok((record.status, record.pid, record.engine_pid));
    }

    let Some(endpoint) = record.control_endpoint() else {
        return Ok((record.status, record.pid, record.engine_pid));
    };

    match control_client::request(endpoint, &Request::Status, STATUS_TIMEOUT) {
        Ok(response) => {
            if let Some(Reply::Status { container }) = response.result {
                return Ok((container.status, container.pid, container.engine_pid));
            }
            let message = response
                .error
                .map(|error| error.message)
                .unwrap_or_else(|| String::from("unexpected supervisor reply"));
            Err(CliError::Message(message))
        }
        Err(_) => Ok((record.status, record.pid, record.engine_pid)),
    }
}
