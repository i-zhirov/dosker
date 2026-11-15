//! Image media kinds.

use serde::{Deserialize, Serialize};

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
