//! Build engine error type.

use std::io;
use std::path::PathBuf;

use dosker_core::{DoskerfileError, LockFileError};
use dosker_image::ImageError;
use dosker_platform::PlatformError;

/// A failure in a build operation.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// The Doskerfile is invalid.
    #[error(transparent)]
    Doskerfile(#[from] DoskerfileError),

    /// The lock file is invalid.
    #[error(transparent)]
    LockFile(#[from] LockFileError),

    /// An image operation failed.
    #[error(transparent)]
    Image(#[from] ImageError),

    /// A host platform operation failed.
    #[error(transparent)]
    Platform(#[from] PlatformError),

    /// A file operation failed at a named path.
    #[error("i/o error at {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// Writing build progress failed.
    #[error("reporter error: {0}")]
    Reporter(io::Error),

    /// The build context is invalid.
    #[error("build context error: {reason}")]
    Context { reason: String },

    /// A lock pin does not match the resolved value.
    #[error("{message}")]
    LockMismatch { message: String },

    /// The base image is not supported by the build.
    #[error("unsupported base: {reason}")]
    UnsupportedBase { reason: String },

    /// The selected engine cannot be used.
    #[error("engine {engine}: {reason}")]
    Engine { engine: String, reason: String },

    /// A Doskerfile instruction failed.
    #[error("Doskerfile line {line}: {instruction}: {reason}")]
    Instruction {
        line: usize,
        instruction: String,
        reason: String,
    },

    /// The build cache is invalid.
    #[error("build cache: {message}")]
    Cache { message: String },
}
