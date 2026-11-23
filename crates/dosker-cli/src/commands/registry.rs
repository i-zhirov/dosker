//! `dosker registry validate`: check registry image metadata and licenses.

use dosker_build::{RegistryError, resolve_target, validate_target};

use crate::cli::{EXIT_ERROR, EXIT_OK, RegistryArgs, RegistryCommand, RegistryValidateArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: RegistryArgs, context: &mut Context) -> Result<u8, CliError> {
    match args.command {
        RegistryCommand::Validate(args) => validate(args, context),
    }
}

fn validate(args: RegistryValidateArgs, context: &mut Context) -> Result<u8, CliError> {
    let target = match resolve_target(&args.path, args.registry_root.as_deref()) {
        Ok(target) => target,
        Err(error) => return fail(&error, context),
    };

    match validate_target(&target, &context.images) {
        Ok(validations) => {
            for validation in validations {
                context
                    .console
                    .line(&format!("validated {}", validation.image))?;
            }
            Ok(EXIT_OK)
        }
        Err(error) => fail(&error, context),
    }
}

fn fail(error: &RegistryError, context: &mut Context) -> Result<u8, CliError> {
    context.console.error_line(&format!("error: {error}"))?;
    Ok(EXIT_ERROR)
}
