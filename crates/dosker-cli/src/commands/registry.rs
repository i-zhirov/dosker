//! `dosker registry validate` and `dosker registry check-final`.

use dosker_build::{RegistryError, resolve_target, validate_final, validate_target};
use dosker_image::ImageRef;

use crate::cli::{
    CheckFinalArgs, EXIT_ERROR, EXIT_OK, RegistryArgs, RegistryCommand, RegistryValidateArgs,
};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: RegistryArgs, context: &mut Context) -> Result<u8, CliError> {
    match args.command {
        RegistryCommand::Validate(args) => validate(args, context),
        RegistryCommand::CheckFinal(args) => check_final(args, context),
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

fn check_final(args: CheckFinalArgs, context: &mut Context) -> Result<u8, CliError> {
    let reference = match ImageRef::parse(&args.image) {
        Ok(reference) => reference,
        Err(error) => {
            context.console.error_line(&format!("error: {error}"))?;
            return Ok(EXIT_ERROR);
        }
    };

    match validate_final(&reference, &context.images) {
        Ok(validation) => {
            context
                .console
                .line(&format!("checked {}", validation.image))?;
            Ok(EXIT_OK)
        }
        Err(error) => fail(&error, context),
    }
}

fn fail(error: &RegistryError, context: &mut Context) -> Result<u8, CliError> {
    context.console.error_line(&format!("error: {error}"))?;
    Ok(EXIT_ERROR)
}
