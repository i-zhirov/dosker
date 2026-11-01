//! Errors raised while running a CLI command.

use std::io;

use dosker_core::CoreError;
use dosker_platform::PlatformError;
use dosker_runtime::StoreError;

use crate::cli::{EXIT_ERROR, EXIT_USAGE};

/// A failure in a CLI command.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// The command line is wrong or a reference cannot be parsed.
    #[error("{0}")]
    Usage(String),

    /// The command failed for a runtime reason.
    #[error("{0}")]
    Message(String),

    /// A host platform operation failed.
    #[error(transparent)]
    Platform(#[from] PlatformError),

    /// A domain value was invalid.
    #[error(transparent)]
    Core(#[from] CoreError),

    /// The state store failed.
    #[error(transparent)]
    Store(#[from] StoreError),

    /// An input or output operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl CliError {
    /// Returns the process exit code for this error.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => EXIT_USAGE,
            Self::Core(CoreError::InvalidContainerId(_) | CoreError::InvalidContainerName(_)) => {
                EXIT_USAGE
            }
            _ => EXIT_ERROR,
        }
    }
}
