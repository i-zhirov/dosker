//! `dosker port`: print the effective published ports of a container.

use chrono::{DateTime, Utc};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{EXIT_OK, PortArgs};
use crate::context::Context;
use crate::error::CliError;
use crate::resolve::resolve;

pub fn execute(
    args: PortArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let record = resolve(&context.store, &args.container)?;
    recover_stale(&context.store, probe, now)?;
    let record = context.store.load(&record.id)?;

    for mapping in &record.ports {
        context.console.line(&mapping.endpoint())?;
    }

    Ok(EXIT_OK)
}
