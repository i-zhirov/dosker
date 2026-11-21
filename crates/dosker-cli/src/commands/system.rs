//! `dosker system prune`: remove stopped containers, unused images, and stale
//! cache entries.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use dosker_core::Status;
use dosker_image::{CacheStore, ImageDigest, ImageRef};
use dosker_runtime::recover_stale;
use dosker_runtime::stale::HostProbe;

use crate::cli::{EXIT_OK, SystemArgs, SystemCommand};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(
    args: SystemArgs,
    context: &mut Context,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    match args.command {
        SystemCommand::Prune(_) => prune(context, probe, now),
    }
}

fn prune(context: &mut Context, probe: &dyn HostProbe, now: DateTime<Utc>) -> Result<u8, CliError> {
    recover_stale(&context.store, probe, now)?;

    let mut containers = 0;
    let mut remaining = Vec::new();
    for record in context.store.list()? {
        if matches!(
            record.status,
            Status::Created | Status::Stopped | Status::Exited | Status::Failed
        ) {
            context.store.remove(&record)?;
            containers += 1;
        } else {
            remaining.push(record);
        }
    }

    let in_use: BTreeSet<String> = remaining
        .iter()
        .map(|record| record.image_digest.clone())
        .filter(|digest| !digest.is_empty())
        .collect();

    let mut images = 0;
    for digest in context.images.untagged_manifest_digests(&in_use)? {
        let reference = ImageRef::Digest(ImageDigest::parse(&digest)?);
        let outcome = context.images.remove(&reference)?;
        if outcome.removed_manifest {
            images += 1;
        }
    }

    let cache = CacheStore::new(context.paths.root().to_path_buf()).prune(&context.images)?;

    context.console.line(&format!(
        "removed {containers} containers, {images} images, {cache} cache entries"
    ))?;
    Ok(EXIT_OK)
}
