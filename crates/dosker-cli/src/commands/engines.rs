//! `dosker engines`: report discovered engines and their capabilities.

use dosker_engine::{EngineError, IMPLEMENTED_ENGINES};

use crate::adapter;
use crate::cli::{EXIT_ERROR, EXIT_OK, EnginesArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(_args: EnginesArgs, context: &mut Context) -> Result<u8, CliError> {
    let mut found = false;

    for provider in IMPLEMENTED_ENGINES {
        match adapter::discover(provider.name) {
            Ok(engine) => {
                found = true;
                let capability_names = engine.capabilities.names().collect::<Vec<_>>().join(",");
                context.console.line(&format!(
                    "{} {} {} {}",
                    engine.name,
                    engine.version,
                    engine.path.display(),
                    capability_names
                ))?;
            }
            Err(error @ EngineError::NotFound(_)) => {
                context
                    .console
                    .line(&format!("{} not found: {error}", provider.name))?;
            }
            Err(error) => {
                context
                    .console
                    .line(&format!("{} unavailable: {error}", provider.name))?;
            }
        }
    }

    if found { Ok(EXIT_OK) } else { Ok(EXIT_ERROR) }
}
