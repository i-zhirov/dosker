//! `dosker inspect`: print a container record as JSON.

use chrono::{DateTime, Utc};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{EXIT_OK, InspectArgs};
use crate::context::Context;
use crate::error::CliError;
use crate::resolve::resolve;

pub fn execute(
    args: InspectArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;

    let json = serde_json::to_string_pretty(&record)
        .map_err(|error| CliError::Message(error.to_string()))?;
    context.console.line(&json)?;

    Ok(EXIT_OK)
}
