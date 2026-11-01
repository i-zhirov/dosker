//! Platform error type.

use std::io;
use std::path::PathBuf;

/// A failure in a host platform operation.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// An operation failed at a named path.
    #[error("i/o error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },

    /// An operation failed without a path.
    #[error(transparent)]
    IoPlain(#[from] io::Error),

    /// A required environment variable is absent or empty.
    #[error("environment variable {0} is not set")]
    MissingEnvironment(&'static str),

    /// The host has no control endpoint implementation.
    #[error("control endpoint is not supported on this host")]
    UnsupportedEndpoint,

    /// The host does not support an operation.
    #[error("{0} is not supported on this host")]
    Unsupported(&'static str),

    /// An endpoint value is malformed.
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),
}
