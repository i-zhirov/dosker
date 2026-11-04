//! `dosker image`: import image blobs.

use chrono::{DateTime, Utc};
use dosker_image::{ImageMedia, ImageReference};

use crate::cli::{EXIT_OK, ImageArgs, ImageCommand};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: ImageArgs, context: &mut Context, now: DateTime<Utc>) -> Result<u8, CliError> {
    match args.command {
        ImageCommand::Import(import) => import_image(import, context, now),
    }
}

fn import_image(
    args: crate::cli::ImageImportArgs,
    context: &mut Context,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let reference = ImageReference::parse(&args.reference)?;
    let media = if args.booter {
        ImageMedia::Booter
    } else {
        ImageMedia::Disk
    };

    let record = context.images.import(&args.file, &reference, media, now)?;
    context.console.line(&format!(
        "imported {}:{} {} {} bytes",
        record.name, record.tag, record.digest, record.size
    ))?;

    Ok(EXIT_OK)
}
