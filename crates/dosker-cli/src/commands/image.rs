//! `dosker image`: import and inspect images.

use chrono::{DateTime, Utc};
use dosker_core::{LicenseEntry, LicenseStatus, license_summary};
use dosker_image::remote::{PullError, RegistryClient};
use dosker_image::{ImageMedia, ImageRef, ImageReference, ReferenceSelector, RegistryReference};

use crate::cli::{EXIT_OK, ImageArgs, ImageCommand, ImageImportArgs, ImageInspectArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: ImageArgs, context: &mut Context, now: DateTime<Utc>) -> Result<u8, CliError> {
    match args.command {
        ImageCommand::Import(import) => import_image(import, context, now),
        ImageCommand::Inspect(inspect) => inspect_image(inspect, context),
    }
}

fn import_image(
    args: ImageImportArgs,
    context: &mut Context,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let reference = ImageReference::parse(&args.reference)?;
    let media = if args.booter {
        ImageMedia::Booter
    } else {
        ImageMedia::Disk
    };

    let agent = if media == ImageMedia::Disk {
        Some(dosker_image::freedos::load_agent_executable()?)
    } else {
        None
    };
    let record = context
        .images
        .import(&args.file, &reference, media, agent.as_deref(), now)?;
    context.console.line(&format!(
        "imported {}:{} {} {} bytes",
        record.name, record.tag, record.manifest_digest, record.size
    ))?;

    Ok(EXIT_OK)
}

fn inspect_image(args: ImageInspectArgs, context: &mut Context) -> Result<u8, CliError> {
    let registry = RegistryReference::parse(&args.reference)?;
    if registry.host().is_none() && !registry.repository().is_namespaced() {
        return inspect_local(&args.reference, context);
    }

    let client = RegistryClient::for_reference(&registry)
        .map_err(|error| CliError::Pull(PullError::Remote(error)))?;
    let index = client
        .image_index(registry.repository())
        .map_err(|error| CliError::Pull(PullError::Remote(error)))?;
    let version = client
        .resolve(&index, registry.selector())
        .map_err(|error| CliError::Pull(PullError::Remote(error)))?
        .clone();
    client
        .manifest(&version)
        .map_err(|error| CliError::Pull(PullError::Remote(error)))?;
    let config = client
        .config(&version)
        .map_err(|error| CliError::Pull(PullError::Remote(error)))?;
    let summary = license_summary(&config.licenses)
        .ok_or_else(|| CliError::Message("the image has no license entries".to_owned()))?;
    let selector = match registry.selector() {
        ReferenceSelector::Tag(tag) => tag.as_str(),
        ReferenceSelector::Digest(digest) => digest.as_str(),
    };

    context.console.line(&format!("image {}", index.image))?;
    context.console.line(&format!("reference {selector}"))?;
    context
        .console
        .line(&format!("version {}", version.version))?;
    context
        .console
        .line(&format!("digest {}", version.digest))?;
    context
        .console
        .line(&format!("config {}", version.config_digest))?;
    context.console.line(&format!(
        "asset {} ({} bytes)",
        version.asset_digest, version.asset_size
    ))?;
    context
        .console
        .line(&format!("asset-url {}", version.asset_url))?;
    context
        .console
        .line(&format!("license {}", summary.as_str()))?;
    print_licenses(&config.licenses, context)?;
    Ok(EXIT_OK)
}

fn inspect_local(value: &str, context: &mut Context) -> Result<u8, CliError> {
    let reference = ImageReference::parse(value)?;
    let image_ref = ImageRef::Tag(reference);
    let image = context.images.load(&image_ref)?;
    let config = context.images.load_config(&image_ref)?;
    let summary = license_summary(&config.licenses);

    context
        .console
        .line(&format!("image {}:{}", image.name, image.tag))?;
    context
        .console
        .line(&format!("digest {}", image.manifest_digest))?;
    context
        .console
        .line(&format!("config {}", image.config_digest))?;
    context
        .console
        .line(&format!("size {} bytes", image.size))?;
    if let Some(summary) = summary {
        context
            .console
            .line(&format!("license {}", summary.as_str()))?;
    }
    print_licenses(&config.licenses, context)?;
    Ok(EXIT_OK)
}

fn print_licenses(entries: &[LicenseEntry], context: &mut Context) -> Result<(), CliError> {
    for entry in entries {
        context.console.line(&format!(
            "  {} {} {}",
            entry.name,
            entry.spdx,
            status_text(entry.status)
        ))?;
        if let Some(holder) = &entry.holder {
            context.console.line(&format!("    holder: {holder}"))?;
        }
        if let Some(notice) = &entry.notice {
            context.console.line(&format!("    notice: {notice}"))?;
        }
    }
    Ok(())
}

fn status_text(status: LicenseStatus) -> &'static str {
    match status {
        LicenseStatus::Redistributable => "redistributable",
        LicenseStatus::Abandonware => "abandonware",
    }
}
