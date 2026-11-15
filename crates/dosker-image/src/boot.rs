//! Disk boot metadata and content-addressed boot blobs.

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::canonical::sha256_digest;
use crate::error::ImageError;

/// How the FAT volume is placed in the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootLayout {
    MbrPartitioned,
    Superfloppy,
    Opaque,
}

/// The FAT variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FatType {
    Fat12,
    Fat16,
    Fat32,
}

/// Disk geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub cylinders: u32,
    pub heads: u32,
    pub sectors: u32,
    pub sector_size: u32,
}

/// A digest and, for small boot sectors, the inline bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    pub sha256: String,
    pub bytes_inline: Option<String>,
}

impl BlobRef {
    /// Builds a reference with base64 inline bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            sha256: sha256_digest(bytes),
            bytes_inline: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        }
    }

    /// Checks that the bytes match the recorded digest.
    pub fn verify(&self, bytes: &[u8]) -> bool {
        self.sha256 == sha256_digest(bytes)
    }
}

/// Boot metadata for an image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootMetadata {
    pub layout: BootLayout,
    pub disk_size: u64,
    pub geometry: Geometry,
    pub partition_start: Option<u32>,
    pub partition_sectors: Option<u32>,
    pub mbr: Option<BlobRef>,
    pub boot_sector: Option<BlobRef>,
    pub fat_type: Option<FatType>,
    pub volume_label: Option<String>,
    pub media_descriptor: Option<u8>,
}

impl BootMetadata {
    /// Creates metadata for an opaque booter blob.
    pub fn opaque(disk_size: u64) -> Self {
        Self {
            layout: BootLayout::Opaque,
            disk_size,
            geometry: Geometry {
                cylinders: 0,
                heads: 0,
                sectors: 0,
                sector_size: 0,
            },
            partition_start: None,
            partition_sectors: None,
            mbr: None,
            boot_sector: None,
            fat_type: None,
            volume_label: None,
            media_descriptor: None,
        }
    }

    /// Validates the fields of one layout.
    pub fn validate(&self) -> Result<(), ImageError> {
        if self.disk_size == 0 {
            return Err(invalid("disk size must be positive"));
        }
        match self.layout {
            BootLayout::Opaque => {
                if self.partition_start.is_some()
                    || self.partition_sectors.is_some()
                    || self.mbr.is_some()
                    || self.boot_sector.is_some()
                    || self.fat_type.is_some()
                {
                    return Err(invalid("an opaque image has no FAT metadata"));
                }
            }
            BootLayout::MbrPartitioned => {
                if self.partition_start.is_none() || self.partition_sectors.is_none() {
                    return Err(invalid("a partitioned image needs partition bounds"));
                }
                if self.mbr.is_none() || self.boot_sector.is_none() {
                    return Err(invalid("a partitioned image needs an MBR and boot sector"));
                }
                validate_fat(self)?;
            }
            BootLayout::Superfloppy => {
                if self.partition_start.is_some() || self.partition_sectors.is_some() {
                    return Err(invalid("a superfloppy has no partition bounds"));
                }
                if self.mbr.is_some() || self.boot_sector.is_none() {
                    return Err(invalid("a superfloppy needs a boot sector and no MBR"));
                }
                validate_fat(self)?;
            }
        }
        if let Some(label) = &self.volume_label
            && label.len() > 11
        {
            return Err(invalid("the volume label is longer than 11 characters"));
        }
        Ok(())
    }
}

fn validate_fat(metadata: &BootMetadata) -> Result<(), ImageError> {
    if metadata.fat_type.is_none() {
        return Err(invalid("a FAT image needs a FAT type"));
    }
    if metadata.geometry.sector_size == 0 {
        return Err(invalid("the sector size must be positive"));
    }
    if metadata.geometry.heads == 0 || metadata.geometry.sectors == 0 {
        return Err(invalid("the geometry needs heads and sectors"));
    }
    Ok(())
}

fn invalid(reason: &str) -> ImageError {
    ImageError::InvalidBootMetadata {
        reason: reason.to_owned(),
    }
}
