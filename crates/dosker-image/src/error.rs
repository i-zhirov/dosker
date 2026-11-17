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

    /// A DOS 8.3 path is invalid.
    #[error("invalid DOS path: {path}: {reason}")]
    InvalidDosPath { path: String, reason: String },

    /// A value cannot be serialized as canonical JSON.
    #[error("cannot serialize canonical JSON: {0}")]
    CanonicalJson(String),

    /// The image config is invalid.
    #[error("invalid image config: {reason}")]
    InvalidImageConfig { reason: String },

    /// The OCI media type is not supported.
    #[error("unsupported media type: {0}")]
    UnsupportedMediaType(String),

    /// The OCI manifest or index is invalid.
    #[error("invalid OCI object: {reason}")]
    InvalidManifest { reason: String },

    /// A layer tar or sidecar is malformed.
    #[error("corrupt layer at {path}: {message}")]
    LayerCorrupt { path: String, message: String },

    /// A layer member is unsafe.
    #[error("unsafe layer entry at {path}: {reason}")]
    UnsafeLayerEntry { path: String, reason: String },

    /// A layer tar has no sidecar.
    #[error("layer sidecar is missing")]
    MissingLayerSidecar,

    /// A content digest does not match its bytes.
    #[error("digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },

    /// An image cannot be found by digest.
    #[error("image not found: {0}")]
    ImageNotFound(String),

    /// A non-booter image has no raw disk blob.
    #[error("image {0} has no raw disk; import it again or rebuild it")]
    MissingRawDisk(String),

    /// The image has no tree.
    #[error("image {0} has no tree")]
    NoTreeImage(String),

    /// An image still has a tag.
    #[error("image {0} is tagged; remove the tags first")]
    ImageInUse(String),

    /// The image index lock cannot be taken.
    #[error("cannot lock {}: {source}", path.display())]
    Lock { path: PathBuf, source: io::Error },

    /// Two layer entries conflict at one path.
    #[error("layer path conflict at {path}: {reason}")]
    LayerConflict { path: String, reason: String },

    /// Boot metadata is invalid.
    #[error("invalid boot metadata: {reason}")]
    InvalidBootMetadata { reason: String },

    /// The tree cannot be projected onto the disk.
    #[error("cannot project the tree onto the disk: {reason}")]
    Projection { reason: String },

    /// The image content is malformed.
    #[error("corrupt image at {}: {message}", path.display())]
    Corrupt { path: PathBuf, message: String },

    /// A file operation failed at a named path.
    #[error("i/o error at {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// The FreeDOS source variable is unset.
    #[error("DOSKER_FREEDOS_SOURCE is not set; set it to a local FreeDOS 1.3 boot floppy image")]
    FreedosSourceNotSet,

    /// The FreeDOS source path does not exist.
    #[error(
        "FreeDOS source {} is missing; set DOSKER_FREEDOS_SOURCE to a local FreeDOS 1.3 boot floppy image",
        path.display()
    )]
    FreedosSourceMissing { path: PathBuf },

    /// A named FreeDOS input file is absent.
    #[error("{file} is missing from {}", path.display())]
    FreedosFileMissing { path: PathBuf, file: String },

    /// A FreeDOS input does not match its pinned digest.
    #[error("{file} digest mismatch: expected {expected}, got {actual}")]
    FreedosPinMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    /// The FreeDOS release is not pinned.
    #[error("unsupported FreeDOS version {version}; supported: {supported}")]
    UnsupportedFreedosVersion { version: String, supported: String },

    /// A FreeDOS boot template is malformed.
    #[error("invalid FreeDOS boot template: {reason}")]
    FreedosTemplate { reason: String },

    /// The agent cannot be injected into an imported image.
    #[error("agent injection: {reason}")]
    AgentInjection { reason: String },

    /// A disk import has no agent executable.
    #[error("a disk import requires the agent executable")]
    AgentMissing,

    /// A host platform operation failed.
    #[error(transparent)]
    Platform(#[from] PlatformError),
}
