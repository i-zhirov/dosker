//! `dosker save`: write one tagged image to an OCI image-layout tar.

use std::path::PathBuf;

use dosker_image::{ImageRef, save_image};

use crate::cli::{EXIT_OK, SaveArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: SaveArgs, context: &mut Context) -> Result<u8, CliError> {
    let reference = match ImageRef::parse(&args.image)? {
        ImageRef::Tag(reference) => reference,
        ImageRef::Digest(_) => {
            return Err(CliError::Usage(String::from(
                "save requires a tag reference",
            )));
        }
    };
    let destination = args.output.unwrap_or_else(|| {
        PathBuf::from(format!(
            "{}-{}.tar",
            reference.name().as_str(),
            reference.tag().as_str()
        ))
    });
    let report = save_image(&context.images, &reference, &destination)?;
    context.console.line(&format!(
        "saved {} to {}",
        report.reference,
        destination.display()
    ))?;
    Ok(EXIT_OK)
}
