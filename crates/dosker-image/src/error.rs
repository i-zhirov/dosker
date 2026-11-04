//! Image error type.

use std::io;
use std::path::PathBuf;

use dosker_platform::PlatformError;

use crate::reference::ImageReference;

/// A failure in an image operation.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// An image reference does not match the accepted form.
    #[error("invalid image reference: {0}")]
    InvalidReference(String),

    /// No image record exists for the reference.
    #[error("image not found: {0}")]
    NotFound(ImageReference),

    /// An image record already exists for the reference.
    #[error("image already exists: {0}")]
    AlreadyExists(ImageReference),

    /// The file has no recognizable hard-disk partition or FAT boot sector.
    #[error("image is not bootable: {}", .0.display())]
    NotBootable(PathBuf),

    /// The image layout cannot be used for the requested purpose.
    #[error("unsupported image layout: {}", .0.display())]
    UnsupportedLayout(PathBuf),

    /// The image content is malformed.
    #[error("corrupt image at {}: {message}", path.display())]
    Corrupt { path: PathBuf, message: String },

    /// A file operation failed at a named path.
    #[error("i/o error at {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// A host platform operation failed.
    #[error(transparent)]
    Platform(#[from] PlatformError),
}
