//! `dosker load`: restore tags from an OCI image-layout tar.

use dosker_image::load_image;

use crate::cli::{EXIT_OK, LoadArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: LoadArgs, context: &mut Context) -> Result<u8, CliError> {
    let report = load_image(&context.images, &args.file)?;
    for reference in report.references {
        context.console.line(&format!("loaded {reference}"))?;
    }
    Ok(EXIT_OK)
}
