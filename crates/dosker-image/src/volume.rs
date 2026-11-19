//! Named volume disk images.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use fatfs::{FormatVolumeOptions, format_volume};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::boot::{FatType, Geometry};
use crate::error::ImageError;
use crate::fat::{PartitionFile, read_boot_metadata};

/// The default volume size in bytes.
pub const DEFAULT_VOLUME_SIZE: u64 = 67_108_864;
/// The size of a 1.44 MB floppy volume in bytes.
pub const FLOPPY_1440_SIZE: u64 = 1_474_560;
/// The size of a 2.88 MB floppy volume in bytes.
pub const FLOPPY_2880_SIZE: u64 = 2_949_120;
/// The smallest partitioned volume in bytes.
pub const MIN_PARTITIONED_VOLUME_SIZE: u64 = 16_777_216;
/// The largest partitioned volume in bytes.
pub const MAX_PARTITIONED_VOLUME_SIZE: u64 = 1_073_741_824;
/// The first sector of the FAT partition.
pub const PARTITION_START: u32 = 2048;
/// The MBR partition type of a FAT16 volume.
pub const PARTITION_TYPE_FAT16: u8 = 0x06;
/// The volume label text.
pub const VOLUME_LABEL_TEXT: &str = "DOSKER";

const SECTOR_SIZE: u64 = 512;
const SECTOR_BYTES: usize = SECTOR_SIZE as usize;
const PARTITION_TABLE_OFFSET: usize = 446;
const BOOT_SIGNATURE_OFFSET: usize = 510;
const HIDDEN_SECTORS_OFFSET: u64 = 28;
const FAT_COPIES: u8 = 2;
const MEDIA_FIXED: u8 = 0xF8;
const MEDIA_FLOPPY: u8 = 0xF0;
const HEADS: u16 = 16;
const SECTORS_PER_TRACK: u16 = 32;
const CHS_HEADS: u32 = 16;
const CHS_SECTORS_PER_TRACK: u32 = 32;
const CHS_MAX_LBA: u32 = 524_287;

/// How a volume maps its FAT filesystem onto the disk image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeLayout {
    /// The FAT volume starts at an MBR partition offset.
    MbrPartitioned,
    /// The FAT volume starts at the beginning of the image.
    Superfloppy,
}

impl VolumeLayout {
    /// Returns the serialized name of the layout.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MbrPartitioned => "mbr_partitioned",
            Self::Superfloppy => "superfloppy",
        }
    }
}

/// The on-disk shape of a created volume image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeImage {
    pub layout: VolumeLayout,
    pub fat_type: FatType,
    pub disk_size: u64,
    pub geometry: Geometry,
    pub partition_start: Option<u32>,
    pub partition_sectors: Option<u32>,
    pub volume_id: u32,
}

/// Creates a formatted FAT volume image of `size` bytes.
pub fn create_volume(disk: &Path, size: u64, name: &str) -> Result<VolumeImage, ImageError> {
    let layout = validate_size(size)?;
    let volume_id = volume_id(name);

    let file = File::create(disk).map_err(|source| image_io(disk, source))?;
    file.set_len(size)
        .map_err(|source| image_io(disk, source))?;
    drop(file);

    if layout == VolumeLayout::MbrPartitioned {
        write_mbr(disk, size)?;
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(disk)
        .map_err(|source| image_io(disk, source))?;
    let options = format_options(layout, size, name);
    match layout {
        VolumeLayout::MbrPartitioned => {
            let offset = u64::from(PARTITION_START) * SECTOR_SIZE;
            let length = u64::from(partition_sectors(size)) * SECTOR_SIZE;
            let region = PartitionFile::new(file, offset, length);
            format_volume(region, options).map_err(|source| {
                invalid_size(size, format!("cannot format the volume: {source}"))
            })?;
        }
        VolumeLayout::Superfloppy => {
            format_volume(file, options).map_err(|source| {
                invalid_size(size, format!("cannot format the volume: {source}"))
            })?;
        }
    }

    if layout == VolumeLayout::MbrPartitioned {
        patch_hidden_sectors(disk)?;
    }

    let boot = read_boot_metadata(disk)?;
    let fat_type = boot.fat_type.ok_or_else(|| ImageError::Corrupt {
        path: disk.to_path_buf(),
        message: "the created volume has no FAT type".to_owned(),
    })?;
    Ok(VolumeImage {
        layout,
        fat_type,
        disk_size: boot.disk_size,
        geometry: boot.geometry,
        partition_start: boot.partition_start,
        partition_sectors: boot.partition_sectors,
        volume_id,
    })
}

fn validate_size(size: u64) -> Result<VolumeLayout, ImageError> {
    if !size.is_multiple_of(SECTOR_SIZE) {
        return Err(invalid_size(
            size,
            "the size must be a multiple of 512 bytes",
        ));
    }
    if size == FLOPPY_1440_SIZE || size == FLOPPY_2880_SIZE {
        return Ok(VolumeLayout::Superfloppy);
    }
    if (MIN_PARTITIONED_VOLUME_SIZE..=MAX_PARTITIONED_VOLUME_SIZE).contains(&size) {
        return Ok(VolumeLayout::MbrPartitioned);
    }
    Err(invalid_size(
        size,
        format!(
            "the size must be a floppy size or between {MIN_PARTITIONED_VOLUME_SIZE} and {MAX_PARTITIONED_VOLUME_SIZE} bytes"
        ),
    ))
}

fn format_options(layout: VolumeLayout, size: u64, name: &str) -> FormatVolumeOptions {
    let mut label = [b' '; 11];
    label[..VOLUME_LABEL_TEXT.len()].copy_from_slice(VOLUME_LABEL_TEXT.as_bytes());
    let options = FormatVolumeOptions::new()
        .bytes_per_sector(SECTOR_SIZE as u16)
        .fats(FAT_COPIES)
        .volume_id(volume_id(name))
        .volume_label(label);
    match layout {
        VolumeLayout::MbrPartitioned => options
            .fat_type(fatfs::FatType::Fat16)
            .total_sectors(partition_sectors(size))
            .bytes_per_cluster(cluster_bytes(size))
            .max_root_dir_entries(512)
            .media(MEDIA_FIXED)
            .sectors_per_track(SECTORS_PER_TRACK)
            .heads(HEADS)
            .drive_num(0x80),
        VolumeLayout::Superfloppy if size == FLOPPY_1440_SIZE => options
            .fat_type(fatfs::FatType::Fat12)
            .total_sectors(2880)
            .bytes_per_cluster(512)
            .max_root_dir_entries(224)
            .media(MEDIA_FLOPPY)
            .sectors_per_track(18)
            .heads(2)
            .drive_num(0x00),
        VolumeLayout::Superfloppy => options
            .fat_type(fatfs::FatType::Fat16)
            .total_sectors(5760)
            .bytes_per_cluster(512)
            .max_root_dir_entries(224)
            .media(MEDIA_FLOPPY)
            .sectors_per_track(36)
            .heads(2)
            .drive_num(0x00),
    }
}

fn partition_sectors(size: u64) -> u32 {
    (size / SECTOR_SIZE) as u32 - PARTITION_START
}

fn cluster_bytes(size: u64) -> u32 {
    (size / 32_768).next_power_of_two().clamp(512, 32_768) as u32
}

fn volume_id(name: &str) -> u32 {
    let digest = Sha256::digest(name.as_bytes());
    u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]])
}

fn write_mbr(disk: &Path, size: u64) -> Result<(), ImageError> {
    let sectors = partition_sectors(size);
    let mut sector = [0u8; SECTOR_BYTES];
    let entry = PARTITION_TABLE_OFFSET;
    sector[entry] = 0x00;
    sector[entry + 1..entry + 4].copy_from_slice(&chs(PARTITION_START));
    sector[entry + 4] = PARTITION_TYPE_FAT16;
    sector[entry + 5..entry + 8].copy_from_slice(&chs(PARTITION_START + sectors - 1));
    sector[entry + 8..entry + 12].copy_from_slice(&PARTITION_START.to_le_bytes());
    sector[entry + 12..entry + 16].copy_from_slice(&sectors.to_le_bytes());
    sector[BOOT_SIGNATURE_OFFSET] = 0x55;
    sector[BOOT_SIGNATURE_OFFSET + 1] = 0xAA;

    let mut file = OpenOptions::new()
        .write(true)
        .open(disk)
        .map_err(|source| image_io(disk, source))?;
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.write_all(&sector))
        .and_then(|_| file.flush())
        .map_err(|source| image_io(disk, source))
}

fn patch_hidden_sectors(disk: &Path) -> Result<(), ImageError> {
    let offset = u64::from(PARTITION_START) * SECTOR_SIZE + HIDDEN_SECTORS_OFFSET;
    let mut file = OpenOptions::new()
        .write(true)
        .open(disk)
        .map_err(|source| image_io(disk, source))?;
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.write_all(&PARTITION_START.to_le_bytes()))
        .and_then(|_| file.flush())
        .map_err(|source| image_io(disk, source))
}

fn chs(lba: u32) -> [u8; 3] {
    let lba = lba.min(CHS_MAX_LBA);
    let head = (lba / CHS_SECTORS_PER_TRACK) % CHS_HEADS;
    let sector = (lba % CHS_SECTORS_PER_TRACK) + 1;
    let cylinder = lba / (CHS_HEADS * CHS_SECTORS_PER_TRACK);
    [
        head as u8,
        (sector | ((cylinder >> 2) & 0xC0)) as u8,
        (cylinder & 0xFF) as u8,
    ]
}

fn invalid_size(size: u64, reason: impl Into<String>) -> ImageError {
    ImageError::InvalidVolumeSize {
        size,
        reason: reason.into(),
    }
}

fn image_io(path: &Path, source: std::io::Error) -> ImageError {
    ImageError::Io {
        path: path.to_path_buf(),
        source,
    }
}
