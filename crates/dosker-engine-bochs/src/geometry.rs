//! Disk geometry for Bochs ATA disks.

use dosker_engine::EngineError;

/// The number of heads reported for every disk.
const HEADS: u8 = 16;

/// The number of sectors per track reported for every disk.
const SECTORS_PER_TRACK: u8 = 32;

/// The size of one sector in bytes.
const SECTOR_BYTES: u64 = 512;

/// The size of one cylinder in bytes.
const CHS_BYTES: u64 = HEADS as u64 * SECTORS_PER_TRACK as u64 * SECTOR_BYTES;

/// The largest cylinder count a disk may use.
const MAX_CYLINDERS: u32 = 16_383;

/// The CHS geometry of a Bochs disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskGeometry {
    pub cylinders: u32,
    pub heads: u8,
    pub sectors: u8,
}

impl DiskGeometry {
    /// Derives the geometry for a disk of `bytes` bytes.
    pub fn from_size(bytes: u64) -> Result<Self, EngineError> {
        if bytes == 0 || !bytes.is_multiple_of(CHS_BYTES) {
            return Err(EngineError::Setup(format!(
                "disk size {bytes} is not a positive multiple of {CHS_BYTES}"
            )));
        }

        let cylinders = bytes / CHS_BYTES;
        if cylinders > u64::from(MAX_CYLINDERS) {
            return Err(EngineError::Setup(format!(
                "disk size {bytes} needs {cylinders} cylinders; the maximum is {MAX_CYLINDERS}"
            )));
        }

        Ok(Self {
            cylinders: cylinders as u32,
            heads: HEADS,
            sectors: SECTORS_PER_TRACK,
        })
    }
}
