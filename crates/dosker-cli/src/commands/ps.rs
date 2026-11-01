//! `dosker ps`: list containers from the state store.

use chrono::{DateTime, SecondsFormat, Utc};
use dosker_runtime::stale::{HostProbe, recover_stale};

use crate::cli::{EXIT_OK, PsArgs};
use crate::context::Context;
use crate::error::CliError;

const HEADER: &str = "ID NAME IMAGE STATUS CREATED";

pub fn execute(
    args: PsArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    recover_stale(&context.store, probe, now)?;
    let records = context.store.list()?;

    context.console.line(HEADER)?;
    for record in records {
        if !args.all && !record.status.is_active() {
            continue;
        }

        let created_at = record.created_at.to_rfc3339_opts(SecondsFormat::Secs, true);
        context.console.line(&format!(
            "{} {} {} {} {}",
            record.id, record.name, record.image, record.status, created_at
        ))?;
    }

    Ok(EXIT_OK)
}
