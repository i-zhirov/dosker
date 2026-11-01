//! Core domain errors.

use crate::status::Status;

/// An error raised while validating core domain values.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid container id: {0}")]
    InvalidContainerId(String),
    #[error("invalid container name: {0}")]
    InvalidContainerName(String),
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition { from: Status, to: Status },
}
