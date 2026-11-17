//! Builds the deterministic FreeDOS base disk image.

use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

use base64::Engine;
use fatfs::{Dir, FileSystem, FormatVolumeOptions, format_volume};

use crate::boot::{BlobRef, BootLayout, BootMetadata, FatType, Geometry};
use crate::error::ImageError;
use crate::fat::{PartitionFile, fixed_fs_options, read_boot_metadata_from, read_tree_from};
use crate::freedos::AGENT_VERSION;
use crate::freedos::inputs::{BOOT_SECTOR_TEMPLATE, FreedosBinaries, FreedosVersion, MBR_TEMPLATE};
use crate::tree::Tree;

/// The size of the base disk in bytes.
pub const DISK_SIZE: u64 = 33_554_432;
/// The first sector of the FAT partition.
pub const PARTITION_START: u32 = 2048;
/// The length of the FAT partition in sectors.
pub const PARTITION_SECTORS: u32 = 63_488;
/// The number of heads in the disk geometry.
pub const HEADS: u16 = 16;
/// The number of sectors per track in the disk geometry.
pub const SECTORS_PER_TRACK: u16 = 32;
/// The bytes in one sector.
pub const BYTES_PER_SECTOR: u16 = 512;
/// The bytes in one cluster.
pub const BYTES_PER_CLUSTER: u32 = 512;
/// The root-directory entry capacity.
pub const ROOT_DIR_ENTRIES: u16 = 512;
/// The FAT16 media descriptor.
pub const MEDIA_DESCRIPTOR: u8 = 0xF8;
/// The volume identifier.
pub const VOLUME_ID: u32 = 0x0045_105C;
/// The volume label, padded to eleven bytes.
pub const VOLUME_LABEL: [u8; 11] = *b"DOSKER     ";

/// The base `CONFIG.SYS` contents.
pub const CONFIG_SYS_BYTES: &[u8] = b"FILES=20\r\nBUFFERS=10\r\nLASTDRIVE=Z\r\n";
/// The base `AUTOEXEC.BAT` contents.
pub const BASE_AUTOEXEC_BYTES: &[u8] =
    b"@ECHO OFF\r\nSET COMSPEC=C:\\COMMAND.COM\r\nC:\\DOSKER\\DOSKER.EXE\r\n";

const SECTOR_BYTES: usize = BYTES_PER_SECTOR as usize;
const BASE_IMAGE_PATH: &str = "freedos-base.img";
const PARTITION_TABLE_OFFSET: usize = 446;
const BOOT_SIGNATURE_OFFSET: usize = 510;
const BPB_START: usize = 0x0B;
const BPB_END: usize = 0x3E;
const HIDDEN_SECTORS_OFFSET: usize = 28;
const FAT_COPIES: u8 = 2;
const DRIVE_NUMBER: u8 = 0x80;

/// A built FreeDOS base image.
#[derive(Clone, Debug)]
pub struct BaseImage {
    pub version: FreedosVersion,
    pub agent_version: String,
    pub raw: Vec<u8>,
    pub tree: Tree,
    pub boot: BootMetadata,
}

/// Builds the deterministic 32 MiB FreeDOS base disk and its tree.
pub fn build_base_image(
    binaries: &FreedosBinaries,
    version: &FreedosVersion,
) -> Result<BaseImage, ImageError> {
    validate_mbr(&binaries.mbr)?;

    let mut raw = vec![0u8; DISK_SIZE as usize];
    raw[..SECTOR_BYTES].copy_from_slice(&binaries.mbr);

    let partition_offset = u64::from(PARTITION_START) * u64::from(BYTES_PER_SECTOR);
    let partition_length = u64::from(PARTITION_SECTORS) * u64::from(BYTES_PER_SECTOR);

    {
        let disk = PartitionFile::new(
            Cursor::new(raw.as_mut_slice()),
            partition_offset,
            partition_length,
        );
        let options = FormatVolumeOptions::new()
            .fat_type(fatfs::FatType::Fat16)
            .bytes_per_sector(BYTES_PER_SECTOR)
            .total_sectors(PARTITION_SECTORS)
            .bytes_per_cluster(BYTES_PER_CLUSTER)
            .fats(FAT_COPIES)
            .max_root_dir_entries(ROOT_DIR_ENTRIES)
            .media(MEDIA_DESCRIPTOR)
            .sectors_per_track(SECTORS_PER_TRACK)
            .heads(HEADS)
            .drive_num(DRIVE_NUMBER)
            .volume_id(VOLUME_ID)
            .volume_label(VOLUME_LABEL);
        format_volume(disk, options)
            .map_err(|source| corrupt_base(format!("cannot format the partition: {source}")))?;
    }

    {
        let disk = PartitionFile::new(
            Cursor::new(raw.as_mut_slice()),
            partition_offset,
            partition_length,
        );
        let fs = FileSystem::new(disk, fixed_fs_options())
            .map_err(|source| corrupt_base(format!("cannot mount the partition: {source}")))?;
        {
            let root = fs.root_dir();
            write_file(&root, "KERNEL.SYS", &binaries.kernel)?;
            write_file(&root, "COMMAND.COM", &binaries.command)?;
            write_file(&root, "CONFIG.SYS", CONFIG_SYS_BYTES)?;
            write_file(&root, "AUTOEXEC.BAT", BASE_AUTOEXEC_BYTES)?;
            let dosker = root
                .create_dir("DOSKER")
                .map_err(|source| corrupt_base(format!("cannot create DOSKER: {source}")))?;
            write_file(&dosker, "DOSKER.EXE", &binaries.agent)?;
        }
        fs.unmount()
            .map_err(|source| corrupt_base(format!("cannot unmount the partition: {source}")))?;
    }

    patch_boot_sector(&mut raw, binaries)?;

    let tree = read_tree_from(Cursor::new(raw.as_mut_slice()), Path::new(BASE_IMAGE_PATH))?;
    let boot =
        read_boot_metadata_from(Cursor::new(raw.as_mut_slice()), Path::new(BASE_IMAGE_PATH))?;

    Ok(BaseImage {
        version: version.clone(),
        agent_version: AGENT_VERSION.to_owned(),
        raw,
        tree,
        boot,
    })
}

fn write_file<T: Read + Write + Seek>(
    dir: &Dir<'_, T>,
    name: &str,
    bytes: &[u8],
) -> Result<(), ImageError> {
    let mut file = dir
        .create_file(name)
        .map_err(|source| corrupt_base(format!("cannot create {name}: {source}")))?;
    file.truncate()
        .map_err(|source| corrupt_base(format!("cannot truncate {name}: {source}")))?;
    file.write_all(bytes)
        .map_err(|source| corrupt_base(format!("cannot write {name}: {source}")))?;
    Ok(())
}

fn patch_boot_sector(raw: &mut [u8], binaries: &FreedosBinaries) -> Result<(), ImageError> {
    validate_boot_sector(&binaries.boot_sector)?;

    let partition_offset = PARTITION_START as usize * SECTOR_BYTES;
    let formatted = &raw[partition_offset..partition_offset + SECTOR_BYTES];
    let mut sector = binaries.boot_sector.clone();
    sector[BPB_START..BPB_END].copy_from_slice(&formatted[BPB_START..BPB_END]);
    sector[HIDDEN_SECTORS_OFFSET..HIDDEN_SECTORS_OFFSET + 4]
        .copy_from_slice(&PARTITION_START.to_le_bytes());
    raw[partition_offset..partition_offset + SECTOR_BYTES].copy_from_slice(&sector);
    Ok(())
}

fn validate_mbr(mbr: &[u8]) -> Result<(), ImageError> {
    if mbr.len() != SECTOR_BYTES {
        return Err(ImageError::FreedosTemplate {
            reason: format!(
                "the MBR template is {} bytes, expected {SECTOR_BYTES}",
                mbr.len()
            ),
        });
    }
    if mbr[BOOT_SIGNATURE_OFFSET..SECTOR_BYTES] != [0x55, 0xAA] {
        return Err(ImageError::FreedosTemplate {
            reason: "the MBR template has no boot signature".to_owned(),
        });
    }

    let entry = &mbr[PARTITION_TABLE_OFFSET..PARTITION_TABLE_OFFSET + 16];
    let start = u32::from_le_bytes(entry[8..12].try_into().expect("four bytes"));
    let sectors = u32::from_le_bytes(entry[12..16].try_into().expect("four bytes"));
    if entry[0] != 0x80
        || entry[4] != 0x06
        || start != PARTITION_START
        || sectors != PARTITION_SECTORS
    {
        return Err(ImageError::FreedosTemplate {
            reason: format!(
                "the MBR partition entry is not the pinned bootable FAT16 partition at LBA {PARTITION_START} with {PARTITION_SECTORS} sectors"
            ),
        });
    }
    Ok(())
}

fn validate_boot_sector(sector: &[u8]) -> Result<(), ImageError> {
    if sector.len() != SECTOR_BYTES {
        return Err(ImageError::FreedosTemplate {
            reason: format!(
                "the boot-sector template is {} bytes, expected {SECTOR_BYTES}",
                sector.len()
            ),
        });
    }
    if sector[BOOT_SIGNATURE_OFFSET..SECTOR_BYTES] != [0x55, 0xAA] {
        return Err(ImageError::FreedosTemplate {
            reason: "the boot-sector template has no boot signature".to_owned(),
        });
    }
    Ok(())
}

/// Validates that a stored base still matches the pinned layout and templates.
pub(crate) fn validate_base_boot(boot: &BootMetadata) -> Result<(), ImageError> {
    if boot.layout != BootLayout::MbrPartitioned {
        return Err(template_error(
            "the base boot is not an MBR-partitioned disk",
        ));
    }
    if boot.disk_size != DISK_SIZE {
        return Err(template_error(format!(
            "the base disk is {} bytes, expected {DISK_SIZE}",
            boot.disk_size
        )));
    }
    if boot.geometry
        != (Geometry {
            cylinders: 124,
            heads: u32::from(HEADS),
            sectors: u32::from(SECTORS_PER_TRACK),
            sector_size: u32::from(BYTES_PER_SECTOR),
        })
    {
        return Err(template_error(
            "the base disk geometry is not the pinned value",
        ));
    }
    if boot.partition_start != Some(PARTITION_START)
        || boot.partition_sectors != Some(PARTITION_SECTORS)
    {
        return Err(template_error(
            "the base partition is not the pinned bootable FAT16 partition",
        ));
    }
    if boot.fat_type != Some(FatType::Fat16) {
        return Err(template_error("the base volume is not FAT16"));
    }
    if boot.volume_label.as_deref() != Some("DOSKER") {
        return Err(template_error("the base volume label is not DOSKER"));
    }
    if boot.media_descriptor != Some(MEDIA_DESCRIPTOR) {
        return Err(template_error("the base media descriptor is not 0xF8"));
    }

    let mbr = decode_inline(boot.mbr.as_ref(), "MBR")?;
    if mbr.as_slice() != MBR_TEMPLATE {
        return Err(template_error(
            "the base MBR does not match the pinned template",
        ));
    }
    if !boot.mbr.as_ref().expect("the MBR was decoded").verify(&mbr) {
        return Err(template_error("the base MBR does not match its digest"));
    }

    let sector = decode_inline(boot.boot_sector.as_ref(), "boot sector")?;
    if sector.len() != SECTOR_BYTES {
        return Err(template_error(format!(
            "the base boot sector is {} bytes, expected {SECTOR_BYTES}",
            sector.len()
        )));
    }
    if sector[..BPB_START] != BOOT_SECTOR_TEMPLATE[..BPB_START]
        || sector[BPB_END..] != BOOT_SECTOR_TEMPLATE[BPB_END..]
    {
        return Err(template_error(
            "the base boot sector does not keep the pinned FreeDOS loader",
        ));
    }
    if sector[BOOT_SIGNATURE_OFFSET..SECTOR_BYTES] != [0x55, 0xAA] {
        return Err(template_error("the base boot sector has no boot signature"));
    }
    if le_u32(&sector[28..32]) != PARTITION_START {
        return Err(template_error(
            "the base boot sector has the wrong hidden sectors",
        ));
    }
    if u32::from(le_u16(&sector[19..21])) != PARTITION_SECTORS {
        return Err(template_error(
            "the base boot sector has the wrong sector count",
        ));
    }
    if sector[13] != 1 {
        return Err(template_error(
            "the base boot sector has the wrong cluster size",
        ));
    }
    if le_u16(&sector[14..16]) != 1 {
        return Err(template_error(
            "the base boot sector has the wrong reserved sectors",
        ));
    }
    if sector[16] != FAT_COPIES {
        return Err(template_error(
            "the base boot sector has the wrong FAT count",
        ));
    }
    if le_u16(&sector[17..19]) != ROOT_DIR_ENTRIES {
        return Err(template_error(
            "the base boot sector has the wrong root entries",
        ));
    }
    if le_u16(&sector[22..24]) == 0 {
        return Err(template_error(
            "the base boot sector has no sectors per FAT",
        ));
    }
    if sector[21] != MEDIA_DESCRIPTOR {
        return Err(template_error(
            "the base boot sector has the wrong media descriptor",
        ));
    }
    if le_u16(&sector[26..28]) != HEADS {
        return Err(template_error(
            "the base boot sector has the wrong head count",
        ));
    }
    if le_u16(&sector[24..26]) != SECTORS_PER_TRACK {
        return Err(template_error(
            "the base boot sector has the wrong sectors per track",
        ));
    }
    if sector[36] != DRIVE_NUMBER {
        return Err(template_error(
            "the base boot sector has the wrong drive number",
        ));
    }
    if sector[38] != 0x29 {
        return Err(template_error(
            "the base boot sector has no extended boot signature",
        ));
    }
    if le_u32(&sector[39..43]) != VOLUME_ID {
        return Err(template_error(
            "the base boot sector has the wrong volume id",
        ));
    }
    if sector[43..54] != VOLUME_LABEL {
        return Err(template_error(
            "the base boot sector has the wrong volume label",
        ));
    }
    if &sector[54..62] != b"FAT16   " {
        return Err(template_error(
            "the base boot sector has the wrong file-system type",
        ));
    }
    Ok(())
}

fn decode_inline(blob: Option<&BlobRef>, name: &str) -> Result<Vec<u8>, ImageError> {
    let blob = blob.ok_or_else(|| template_error(format!("the base boot has no {name}")))?;
    let inline = blob
        .bytes_inline
        .as_deref()
        .ok_or_else(|| template_error(format!("the base boot has no inline {name} bytes")))?;
    base64::engine::general_purpose::STANDARD
        .decode(inline)
        .map_err(|error| template_error(format!("cannot decode the {name} bytes: {error}")))
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn template_error(reason: impl Into<String>) -> ImageError {
    ImageError::FreedosTemplate {
        reason: reason.into(),
    }
}

fn corrupt_base(message: String) -> ImageError {
    ImageError::Corrupt {
        path: std::path::PathBuf::from(BASE_IMAGE_PATH),
        message,
    }
}
