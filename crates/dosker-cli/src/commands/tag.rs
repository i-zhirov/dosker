//! `dosker tag`: add a local image tag.

use dosker_image::{ImageRef, ImageReference};

use crate::cli::{EXIT_OK, TagArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: TagArgs, context: &mut Context) -> Result<u8, CliError> {
    let source = ImageRef::parse(&args.source)?;
    let target = ImageReference::parse(&args.target)?;
    context.images.tag(&source, &target)?;
    context.console.line(&format!("tagged {target}"))?;
    Ok(EXIT_OK)
}
