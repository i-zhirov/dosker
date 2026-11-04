//! The persisted description of an imported image.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The version of the image record format.
pub const IMAGE_SCHEMA_VERSION: u32 = 1;

/// How an image is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMedia {
    /// A raw DOS disk image with an MBR partition and a FAT volume.
    Disk,
    /// An opaque booter blob with no filesystem assumptions.
    Booter,
}

impl ImageMedia {
    /// Returns true when the image is an opaque booter blob.
    pub fn is_booter(self) -> bool {
        matches!(self, Self::Booter)
    }

    /// Returns true when the image is expected to run the agent.
    pub fn expects_agent(self) -> bool {
        !self.is_booter()
    }
}

/// The minimal record written next to an imported image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRecord {
    /// The record format version.
    pub schema_version: u32,
    /// The image name.
    pub name: String,
    /// The image tag.
    pub tag: String,
    /// How the image is stored.
    pub media: ImageMedia,
    /// The `sha256:<hex>` digest of the stored blob.
    pub digest: String,
    /// The size of the stored blob in bytes.
    pub size: u64,
    /// The imported source path as text.
    pub source: String,
    /// The agent version recorded by the image builder, when present.
    pub agent_version: Option<String>,
    /// The startup command recorded by the image builder, when present.
    pub cmd: Option<String>,
    /// When the image was imported.
    pub imported_at: DateTime<Utc>,
}

impl ImageRecord {
    /// Returns true when the image is an opaque booter blob.
    pub fn is_booter(&self) -> bool {
        self.media.is_booter()
    }

    /// Returns true when the image is expected to run the agent.
    pub fn expects_agent(&self) -> bool {
        self.media.expects_agent()
    }
}
