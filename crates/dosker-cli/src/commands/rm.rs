//! `dosker rm`: remove a container and its private disk.

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{EXIT_OK, RmArgs};
use crate::context::Context;
use crate::error::CliError;
use crate::resolve::resolve;

pub fn execute(
    args: RmArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;

    match record.status {
        Status::Starting => {
            return Err(CliError::Message(format!(
                "container {} is starting",
                record.name
            )));
        }
        Status::Running => {
            return Err(CliError::Message(format!(
                "container {} is running",
                record.name
            )));
        }
        Status::Stopping => {
            return Err(CliError::Message(format!(
                "container {} is stopping",
                record.name
            )));
        }
        _ => {}
    }

    context.store.remove(&record)?;
    context.console.line(record.name.as_str())?;

    Ok(EXIT_OK)
}
