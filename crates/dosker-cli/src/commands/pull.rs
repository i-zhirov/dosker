//! `dosker pull`: fetch, verify, and store one registry image.

use chrono::{DateTime, Utc};
use dosker_image::remote::{self, PullError, PullRequest, RegistryClient};
use dosker_image::{ImageDigest, RegistryReference};

use crate::cli::{EXIT_OK, PullArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: PullArgs, context: &mut Context, now: DateTime<Utc>) -> Result<u8, CliError> {
    let reference = RegistryReference::parse(&args.reference)?;
    if reference.host().is_none() && !reference.repository().is_namespaced() {
        return Err(CliError::Usage(
            "pull needs a namespaced or host-qualified reference".to_owned(),
        ));
    }
    let accept = match &args.accept_license {
        Some(value) => Some(ImageDigest::parse(value)?),
        None => None,
    };
    let client = RegistryClient::for_reference(&reference)
        .map_err(|error| CliError::Pull(PullError::Remote(error)))?;
    let request = PullRequest { reference, accept };
    let report = remote::pull(
        &client,
        &context.images,
        &context.registry_cache,
        &context.acceptance,
        &request,
        now,
    )?;
    let tag = report.tag.as_deref().unwrap_or("");
    let cached = if report.cached { ", cached" } else { "" };
    context.console.line(&format!(
        "pulled {}:{} {} ({} bytes{})",
        report.image, tag, report.digest, report.asset_bytes, cached
    ))?;
    Ok(EXIT_OK)
}
