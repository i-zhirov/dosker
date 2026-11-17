//! Projects the canonical tree and boot metadata onto a raw disk image.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use base64::Engine;
use fatfs::{FileSystem, FormatVolumeOptions, format_volume};

use crate::boot::{BlobRef, BootLayout, BootMetadata};
use crate::error::ImageError;
use crate::fat::{
    FatRegion, PartitionFile, fat_path, fat_region, fixed_fs_options, read_boot_metadata,
};
use crate::tree::Tree;

/// The bytes in one sector of a projected volume.
pub const PROJECTION_SECTOR_BYTES: usize = 512;

const FAT12_MAX_CLUSTERS: u32 = 4085;
const FAT16_MAX_CLUSTERS: u32 = 65525;
const DIR_ENTRY_BYTES: u32 = 32;
const BYTES_PER_SECTOR_OFFSET: usize = 11;
const SECTORS_PER_CLUSTER_OFFSET: usize = 13;
const RESERVED_SECTORS_OFFSET: usize = 14;
const FATS_OFFSET: usize = 16;
const ROOT_ENTRIES_OFFSET: usize = 17;
const TOTAL_SECTORS_16_OFFSET: usize = 19;
const MEDIA_OFFSET: usize = 21;
const SECTORS_PER_FAT_16_OFFSET: usize = 22;
const SECTORS_PER_TRACK_OFFSET: usize = 24;
const HEADS_OFFSET: usize = 26;
const TOTAL_SECTORS_32_OFFSET: usize = 32;
const DRIVE_NUM_OFFSET: usize = 36;
const EXTENDED_SIGNATURE_OFFSET: usize = 38;
const VOLUME_ID_OFFSET: usize = 39;
const VOLUME_LABEL_OFFSET: usize = 43;
const EXTENDED_BOOT_SIGNATURE: u8 = 0x29;

/// Writes the effective MBR and boot sector over a private disk.
pub fn apply_boot_metadata(disk: &Path, boot: &BootMetadata) -> Result<(), ImageError> {
    if boot.layout == BootLayout::Opaque {
        return Err(projection("an opaque image has no bootable FAT volume"));
    }

    let disk_size = fs::metadata(disk)
        .map_err(|source| ImageError::Io {
            path: disk.to_path_buf(),
            source,
        })?
        .len();
    if disk_size != boot.disk_size {
        return Err(projection(format!(
            "the disk is {disk_size} bytes, expected {}",
            boot.disk_size
        )));
    }

    match boot.layout {
        BootLayout::MbrPartitioned => {
            let mbr = decode_inline(boot.mbr.as_ref(), "MBR")?;
            require_sector_size(&mbr, "MBR")?;
            write_sector(disk, 0, &mbr)?;
        }
        BootLayout::Superfloppy => {
            if boot.mbr.is_some() {
                return Err(projection("a superfloppy image has an MBR"));
            }
        }
        BootLayout::Opaque => unreachable!("opaque is rejected above"),
    }

    let region = fat_region(disk).map_err(|error| match error {
        ImageError::NotBootable(_) | ImageError::UnsupportedLayout(_) => {
            projection("the effective MBR has no readable FAT partition")
        }
        other => other,
    })?;

    match boot.layout {
        BootLayout::MbrPartitioned => {
            let start = (region.offset / PROJECTION_SECTOR_BYTES as u64) as u32;
            let sectors = (region.length / PROJECTION_SECTOR_BYTES as u64) as u32;
            if boot.partition_start != Some(start) || boot.partition_sectors != Some(sectors) {
                return Err(projection(
                    "the effective MBR and partition bounds disagree",
                ));
            }
        }
        BootLayout::Superfloppy => {
            if region.offset != 0 {
                return Err(projection(
                    "the superfloppy volume does not start at the beginning of the disk",
                ));
            }
        }
        BootLayout::Opaque => unreachable!("opaque is rejected above"),
    }

    let boot_sector = decode_inline(boot.boot_sector.as_ref(), "boot sector")?;
    require_sector_size(&boot_sector, "boot sector")?;
    write_sector(disk, region.offset, &boot_sector)?;
    Ok(())
}

/// Formats the disk's FAT volume and writes the tree over it.
pub fn project_tree(disk: &Path, tree: &Tree) -> Result<BootMetadata, ImageError> {
    let region = fat_region(disk)?;
    let source_boot_sector = read_boot_sector(disk, region.offset)?;
    let source = Bpb::parse(&source_boot_sector)?;
    let fat_type = source.fat_type()?;

    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk)
            .map_err(|source| ImageError::Io {
                path: disk.to_path_buf(),
                source,
            })?;
        let volume = PartitionFile::new(file, region.offset, region.length);
        format_volume(volume, source.format_options(fat_type))
            .map_err(|source| projection(format!("cannot format the volume: {source}")))?;
    }

    let formatted_boot_sector = read_boot_sector(disk, region.offset)?;
    let formatted = Bpb::parse(&formatted_boot_sector)?;
    if !source.layout_matches(&formatted) {
        return Err(projection("the volume BPB cannot be preserved"));
    }

    write_sector(disk, region.offset, &source_boot_sector)?;
    write_tree(disk, region, tree)?;
    read_boot_metadata(disk)
}

/// The data-layout fields of a FAT12/FAT16 BIOS parameter block.
struct Bpb {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    fats: u8,
    root_entries: u16,
    total_sectors: u32,
    media: u8,
    sectors_per_fat: u16,
    sectors_per_track: u16,
    heads: u16,
    drive_num: u8,
    extended_signature: u8,
    volume_id: u32,
    volume_label: [u8; 11],
}

impl Bpb {
    fn parse(sector: &[u8; PROJECTION_SECTOR_BYTES]) -> Result<Self, ImageError> {
        let bytes_per_sector = le_u16(sector, BYTES_PER_SECTOR_OFFSET);
        if usize::from(bytes_per_sector) != PROJECTION_SECTOR_BYTES {
            return Err(projection(format!(
                "the volume uses {bytes_per_sector}-byte sectors, expected {PROJECTION_SECTOR_BYTES}"
            )));
        }

        let sectors_per_cluster = sector[SECTORS_PER_CLUSTER_OFFSET];
        if sectors_per_cluster == 0 || !sectors_per_cluster.is_power_of_two() {
            return Err(projection(
                "the volume has an invalid sectors-per-cluster value",
            ));
        }

        let reserved_sectors = le_u16(sector, RESERVED_SECTORS_OFFSET);
        if reserved_sectors != 1 {
            return Err(projection(format!(
                "the volume has {reserved_sectors} reserved sectors, expected 1"
            )));
        }

        let fats = sector[FATS_OFFSET];
        if !(1..=2).contains(&fats) {
            return Err(projection(format!(
                "the volume has {fats} FATs, expected 1 or 2"
            )));
        }

        let root_entries = le_u16(sector, ROOT_ENTRIES_OFFSET);
        let sectors_per_fat = le_u16(sector, SECTORS_PER_FAT_16_OFFSET);
        if root_entries == 0 || sectors_per_fat == 0 {
            return Err(projection("a FAT32 volume is not supported"));
        }

        let total_sectors_16 = le_u16(sector, TOTAL_SECTORS_16_OFFSET);
        let total_sectors = if total_sectors_16 == 0 {
            le_u32(sector, TOTAL_SECTORS_32_OFFSET)
        } else {
            u32::from(total_sectors_16)
        };
        if total_sectors == 0 {
            return Err(projection("the volume has no total sector count"));
        }

        let extended_signature = sector[EXTENDED_SIGNATURE_OFFSET];
        let (volume_id, volume_label) = if extended_signature == EXTENDED_BOOT_SIGNATURE {
            let mut volume_label = [0u8; 11];
            volume_label.copy_from_slice(&sector[VOLUME_LABEL_OFFSET..VOLUME_LABEL_OFFSET + 11]);
            (le_u32(sector, VOLUME_ID_OFFSET), volume_label)
        } else {
            (0, [0u8; 11])
        };

        Ok(Self {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            fats,
            root_entries,
            total_sectors,
            media: sector[MEDIA_OFFSET],
            sectors_per_fat,
            sectors_per_track: le_u16(sector, SECTORS_PER_TRACK_OFFSET),
            heads: le_u16(sector, HEADS_OFFSET),
            drive_num: sector[DRIVE_NUM_OFFSET],
            extended_signature,
            volume_id,
            volume_label,
        })
    }

    fn fat_type(&self) -> Result<fatfs::FatType, ImageError> {
        let root_dir_sectors = (u32::from(self.root_entries) * DIR_ENTRY_BYTES)
            .div_ceil(u32::from(self.bytes_per_sector));
        let first_data_sector = u32::from(self.reserved_sectors)
            + u32::from(self.fats) * u32::from(self.sectors_per_fat)
            + root_dir_sectors;
        if self.total_sectors <= first_data_sector {
            return Err(projection("the volume BPB describes no data region"));
        }

        let clusters =
            (self.total_sectors - first_data_sector) / u32::from(self.sectors_per_cluster);
        if clusters < FAT12_MAX_CLUSTERS {
            Ok(fatfs::FatType::Fat12)
        } else if clusters < FAT16_MAX_CLUSTERS {
            Ok(fatfs::FatType::Fat16)
        } else {
            Err(projection(format!(
                "a volume with {clusters} clusters is too large for FAT16"
            )))
        }
    }

    fn format_options(&self, fat_type: fatfs::FatType) -> FormatVolumeOptions {
        let mut options = FormatVolumeOptions::new()
            .fat_type(fat_type)
            .bytes_per_sector(self.bytes_per_sector)
            .total_sectors(self.total_sectors)
            .bytes_per_cluster(
                u32::from(self.sectors_per_cluster) * u32::from(self.bytes_per_sector),
            )
            .fats(self.fats)
            .max_root_dir_entries(self.root_entries)
            .media(self.media)
            .sectors_per_track(self.sectors_per_track)
            .heads(self.heads)
            .drive_num(self.drive_num);
        if self.extended_signature == EXTENDED_BOOT_SIGNATURE {
            options = options.volume_id(self.volume_id);
            if self.volume_label != [0u8; 11] {
                options = options.volume_label(self.volume_label);
            }
        }
        options
    }

    fn layout_matches(&self, other: &Self) -> bool {
        self.bytes_per_sector == other.bytes_per_sector
            && self.sectors_per_cluster == other.sectors_per_cluster
            && self.reserved_sectors == other.reserved_sectors
            && self.fats == other.fats
            && self.root_entries == other.root_entries
            && self.total_sectors == other.total_sectors
            && self.media == other.media
            && self.sectors_per_fat == other.sectors_per_fat
    }
}

fn read_boot_sector(disk: &Path, offset: u64) -> Result<[u8; PROJECTION_SECTOR_BYTES], ImageError> {
    let mut file = File::open(disk).map_err(|source| ImageError::Io {
        path: disk.to_path_buf(),
        source,
    })?;
    let mut sector = [0u8; PROJECTION_SECTOR_BYTES];
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut sector))
        .map_err(|source| ImageError::Io {
            path: disk.to_path_buf(),
            source,
        })?;
    Ok(sector)
}

fn write_sector(disk: &Path, offset: u64, bytes: &[u8]) -> Result<(), ImageError> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(disk)
        .map_err(|source| ImageError::Io {
            path: disk.to_path_buf(),
            source,
        })?;
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.write_all(bytes))
        .and_then(|_| file.flush())
        .map_err(|source| ImageError::Io {
            path: disk.to_path_buf(),
            source,
        })
}

fn decode_inline(blob: Option<&BlobRef>, name: &str) -> Result<Vec<u8>, ImageError> {
    let blob = blob.ok_or_else(|| projection(format!("the {name} is missing")))?;
    let inline = blob
        .bytes_inline
        .as_deref()
        .ok_or_else(|| projection(format!("the {name} has no inline bytes")))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(inline)
        .map_err(|source| {
            projection(format!("the {name} inline bytes are not base64: {source}"))
        })?;
    if !blob.verify(&bytes) {
        return Err(projection(format!(
            "the {name} digest does not match its bytes"
        )));
    }
    Ok(bytes)
}

fn require_sector_size(bytes: &[u8], name: &str) -> Result<(), ImageError> {
    if bytes.len() != PROJECTION_SECTOR_BYTES {
        return Err(projection(format!(
            "the {name} is {} bytes, expected {PROJECTION_SECTOR_BYTES}",
            bytes.len()
        )));
    }
    Ok(())
}

fn write_tree(disk: &Path, region: FatRegion, tree: &Tree) -> Result<(), ImageError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(disk)
        .map_err(|source| ImageError::Io {
            path: disk.to_path_buf(),
            source,
        })?;
    let volume = PartitionFile::new(file, region.offset, region.length);
    let fs = FileSystem::new(volume, fixed_fs_options())
        .map_err(|source| corrupt(disk, format!("cannot mount the FAT volume: {source}")))?;

    {
        let root = fs.root_dir();
        for dir in tree.dirs() {
            let dos_path = dir.path.to_dos_string();
            root.create_dir(&fat_path(&dos_path))
                .map_err(|source| corrupt(disk, format!("cannot create {dos_path}: {source}")))?;
        }
        for entry in tree.files() {
            let dos_path = entry.path.to_dos_string();
            let mut file = root
                .create_file(&fat_path(&dos_path))
                .map_err(|source| corrupt(disk, format!("cannot create {dos_path}: {source}")))?;
            file.truncate()
                .map_err(|source| corrupt(disk, format!("cannot truncate {dos_path}: {source}")))?;
            file.write_all(&entry.bytes)
                .map_err(|source| corrupt(disk, format!("cannot write {dos_path}: {source}")))?;
        }
    }

    fs.unmount()
        .map_err(|source| corrupt(disk, format!("cannot unmount the FAT volume: {source}")))?;
    Ok(())
}

fn le_u16(sector: &[u8; PROJECTION_SECTOR_BYTES], offset: usize) -> u16 {
    u16::from_le_bytes([sector[offset], sector[offset + 1]])
}

fn le_u32(sector: &[u8; PROJECTION_SECTOR_BYTES], offset: usize) -> u32 {
    u32::from_le_bytes([
        sector[offset],
        sector[offset + 1],
        sector[offset + 2],
        sector[offset + 3],
    ])
}

fn projection(reason: impl Into<String>) -> ImageError {
    ImageError::Projection {
        reason: reason.into(),
    }
}

fn corrupt(disk: &Path, message: String) -> ImageError {
    ImageError::Corrupt {
        path: disk.to_path_buf(),
        message,
    }
}
