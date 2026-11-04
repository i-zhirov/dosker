//! `dosker restart`: stop a running container and start it again.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_runtime::protocol::{Reply, Request};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{EXIT_ERROR, EXIT_OK, RestartArgs};
use crate::context::Context;
use crate::control_client;
use crate::error::CliError;
use crate::resolve::resolve;

const STOP_WAIT: Duration = Duration::from_secs(5);
const POLL_STEP: Duration = Duration::from_millis(20);

pub fn execute(
    args: RestartArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let mut record = context.store.load(&record.id)?;

    match record.status {
        Status::Starting => {
            return Err(CliError::Message(format!(
                "container {} is starting",
                record.name
            )));
        }
        Status::Stopping => {
            return Err(CliError::Message(format!(
                "container {} is stopping",
                record.name
            )));
        }
        Status::Failed => {
            return Err(CliError::Message(format!(
                "container {} failed; remove it with `dosker rm {}`",
                record.name, record.name
            )));
        }
        _ => {}
    }

    let mut warned_dirty = false;
    if record.dirty {
        crate::commands::start::warn_if_dirty(context, &record)?;
        warned_dirty = true;
    }

    if record.status == Status::Running {
        stop_running(context, probe, &record, args.time)?;
        record = context.store.load(&record.id)?;
        if record.dirty && !warned_dirty {
            crate::commands::start::warn_if_dirty(context, &record)?;
        }
    }

    let outcome = crate::commands::start::start_container(
        context,
        &record,
        Duration::from_secs(args.boot_timeout),
    )?;
    context.console.line(&format!(
        "{} {}",
        outcome.record.name, outcome.record.status
    ))?;

    if outcome.needs_log() {
        crate::commands::start::print_container_log(context, &outcome.record.id)?;
    }

    if outcome.record.status == Status::Running {
        Ok(EXIT_OK)
    } else {
        Ok(EXIT_ERROR)
    }
}

fn stop_running(
    context: &mut Context,
    probe: &dyn HostProbe,
    record: &dosker_runtime::ContainerRecord,
    time: u64,
) -> Result<(), CliError> {
    let endpoint = record.control_endpoint().ok_or_else(|| {
        CliError::Message(format!(
            "container {} has no control endpoint; remove it with `dosker rm {}`",
            record.name, record.name
        ))
    })?;

    let request = Request::Stop {
        grace_ms: time.saturating_mul(1000),
    };
    let response = control_client::request(endpoint, &request, Duration::from_secs(time)).map_err(
        |error| CliError::Message(format!("cannot stop container {}: {error}", record.name)),
    )?;

    let stopped = matches!(
        response.result,
        Some(Reply::Stop { status }) if matches!(status, Status::Stopped | Status::Exited)
    );
    if !stopped {
        let fresh = context.store.load(&record.id)?;
        if !matches!(fresh.status, Status::Stopped | Status::Exited) {
            let message = response
                .error
                .map(|error| error.message)
                .unwrap_or_else(|| String::from("the supervisor did not stop the container"));
            return Err(CliError::Message(message));
        }
    }

    if let Some(pid) = record.pid {
        let deadline = Instant::now() + STOP_WAIT;
        while probe.process_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(POLL_STEP);
        }
    }

    Ok(())
}
