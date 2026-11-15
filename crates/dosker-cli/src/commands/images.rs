//! `dosker images`: list local image tags.

use chrono::SecondsFormat;

use crate::cli::{EXIT_OK, ImagesArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(_args: ImagesArgs, context: &mut Context) -> Result<u8, CliError> {
    let images = context.images.list()?;
    context.console.line("REFERENCE DIGEST SIZE CREATED")?;
    for image in images {
        context.console.line(&format!(
            "{} {} {} {}",
            image.reference,
            image.manifest_digest,
            image.size,
            image.created_at.to_rfc3339_opts(SecondsFormat::Secs, true)
        ))?;
    }
    Ok(EXIT_OK)
}
