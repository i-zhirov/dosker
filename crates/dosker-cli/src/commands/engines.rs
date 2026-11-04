//! `dosker engines`: report discovered engines and their capabilities.

use dosker_engine::EngineError;
use dosker_engine_qemu::{QemuBinary, capabilities};

use crate::cli::{EXIT_ERROR, EXIT_OK, EnginesArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(_args: EnginesArgs, context: &mut Context) -> Result<u8, CliError> {
    match QemuBinary::discover() {
        Ok(binary) => {
            let capability_names = capabilities().names().collect::<Vec<_>>().join(",");
            context.console.line(&format!(
                "qemu {} {} {}",
                binary.version_string(),
                binary.path.display(),
                capability_names
            ))?;
            Ok(EXIT_OK)
        }
        Err(error @ EngineError::NotFound(_)) => {
            context.console.line(&format!("qemu not found: {error}"))?;
            Ok(EXIT_ERROR)
        }
        Err(error) => Err(CliError::Message(error.to_string())),
    }
}
