//! `dosker rmi`: untag an image or remove an untagged digest.

use dosker_image::ImageRef;

use crate::cli::{EXIT_OK, RmiArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: RmiArgs, context: &mut Context) -> Result<u8, CliError> {
    let reference = ImageRef::parse(&args.reference)?;
    let outcome = context.images.remove(&reference)?;
    if outcome.removed_manifest {
        context.console.line(&format!("removed {}", reference))?;
    } else {
        context.console.line(&format!("untagged {}", reference))?;
    }
    Ok(EXIT_OK)
}
