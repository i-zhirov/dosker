//! Access to the FAT volume inside a raw DOS disk image.

use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::Path;

use fatfs::{FileSystem, FsOptions};

use crate::error::ImageError;

/// The directory of the host-managed configuration file.
pub const CONFIG_DIR: &str = "DOSKER";
/// The name of the host-managed configuration file.
pub const CONFIG_FILE: &str = "CONFIG.INI";
/// The DOS path of the host-managed configuration file.
pub const CONFIG_PATH: &str = r"C:\DOSKER\CONFIG.INI";
/// The control port the agent listens on.
pub const DEFAULT_CONTROL_PORT: u8 = 2;
/// The serial line speed the agent uses.
pub const DEFAULT_BAUD: u32 = 115200;

const SECTOR_SIZE: u64 = 512;
const BOOT_SECTOR_BYTES: usize = SECTOR_SIZE as usize;
const SIGNATURE_OFFSET: usize = 510;
const PARTITION_TABLE_OFFSET: usize = 446;
const PARTITION_TYPE_FAT16: u8 = 0x06;
const PARTITION_TYPE_FAT16_LBA: u8 = 0x0E;

/// How a raw image maps its FAT volume onto the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FatLayout {
    /// The FAT volume starts at an MBR partition offset.
    MbrPartitioned,
    /// The FAT volume starts at the beginning of the file.
    Superfloppy,
}

/// The byte range of the FAT volume inside an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FatRegion {
    /// The first byte of the volume.
    pub offset: u64,
    /// The length of the volume in bytes.
    pub length: u64,
    /// How the volume is placed in the file.
    pub layout: FatLayout,
}

/// Locates the FAT volume inside a raw image.
pub fn fat_region(image: &Path) -> Result<FatRegion, ImageError> {
    let mut file = File::open(image).map_err(|source| ImageError::Io {
        path: image.to_path_buf(),
        source,
    })?;
    let file_size = file
        .metadata()
        .map_err(|source| ImageError::Io {
            path: image.to_path_buf(),
            source,
        })?
        .len();

    let mut sector = [0u8; BOOT_SECTOR_BYTES];
    match file.read_exact(&mut sector) {
        Ok(()) => {}
        Err(source) if source.kind() == ErrorKind::UnexpectedEof => {
            return Err(ImageError::NotBootable(image.to_path_buf()));
        }
        Err(source) => {
            return Err(ImageError::Io {
                path: image.to_path_buf(),
                source,
            });
        }
    }

    if has_signature(&sector)
        && let Some(region) = mbr_region(&sector, file_size)
    {
        return Ok(region);
    }

    if is_fat_boot_sector(&sector) {
        return Ok(FatRegion {
            offset: 0,
            length: file_size,
            layout: FatLayout::Superfloppy,
        });
    }

    Err(ImageError::NotBootable(image.to_path_buf()))
}

/// Writes the agent configuration file into the image's FAT volume.
pub fn write_config(image: &Path, control_port: u8, baud: u32) -> Result<(), ImageError> {
    let region = fat_region(image)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(image)
        .map_err(|source| ImageError::Io {
            path: image.to_path_buf(),
            source,
        })?;
    let disk = PartitionFile::new(file, region.offset, region.length);
    let fs = FileSystem::new(disk, FsOptions::new())
        .map_err(|source| corrupt(image, format!("cannot mount FAT volume: {source}")))?;

    {
        let root = fs.root_dir();
        let dir = root
            .open_dir(CONFIG_DIR)
            .or_else(|_| root.create_dir(CONFIG_DIR))
            .map_err(|source| corrupt(image, format!("cannot open {CONFIG_DIR}: {source}")))?;
        let mut entry = dir
            .create_file(CONFIG_FILE)
            .map_err(|source| corrupt(image, format!("cannot create {CONFIG_FILE}: {source}")))?;
        entry
            .truncate()
            .map_err(|source| corrupt(image, format!("cannot truncate {CONFIG_FILE}: {source}")))?;
        let payload = format!("CONTROL_PORT={control_port}\r\nBAUD={baud}\r\n");
        entry
            .write_all(payload.as_bytes())
            .map_err(|source| corrupt(image, format!("cannot write {CONFIG_FILE}: {source}")))?;
    }

    fs.unmount()
        .map_err(|source| corrupt(image, format!("cannot unmount FAT volume: {source}")))?;
    Ok(())
}

/// Reads a file from the image's FAT volume by DOS path.
pub fn read_file(image: &Path, dos_path: &str) -> Result<Vec<u8>, ImageError> {
    let region = fat_region(image)?;
    let file = File::open(image).map_err(|source| ImageError::Io {
        path: image.to_path_buf(),
        source,
    })?;
    let disk = PartitionFile::new(file, region.offset, region.length);
    let fs = FileSystem::new(disk, FsOptions::new())
        .map_err(|source| corrupt(image, format!("cannot mount FAT volume: {source}")))?;
    let root = fs.root_dir();
    let mut entry = root.open_file(&fat_path(dos_path)).map_err(|source| {
        corrupt(
            image,
            format!("file {dos_path} is missing or unreadable: {source}"),
        )
    })?;

    let mut data = Vec::new();
    entry
        .read_to_end(&mut data)
        .map_err(|source| corrupt(image, format!("cannot read {dos_path}: {source}")))?;
    Ok(data)
}

fn corrupt(image: &Path, message: String) -> ImageError {
    ImageError::Corrupt {
        path: image.to_path_buf(),
        message,
    }
}

fn fat_path(dos_path: &str) -> String {
    let mut path = dos_path;
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        path = &path[2..];
    }
    let path = path.trim_start_matches(['/', '\\']);
    path.replace('\\', "/")
}

fn has_signature(sector: &[u8; BOOT_SECTOR_BYTES]) -> bool {
    sector[SIGNATURE_OFFSET] == 0x55 && sector[SIGNATURE_OFFSET + 1] == 0xAA
}

fn mbr_region(sector: &[u8; BOOT_SECTOR_BYTES], file_size: u64) -> Option<FatRegion> {
    let entry = PARTITION_TABLE_OFFSET;
    let partition_type = sector[entry + 4];
    if partition_type != PARTITION_TYPE_FAT16 && partition_type != PARTITION_TYPE_FAT16_LBA {
        return None;
    }

    let start_lba = u64::from(le_u32(&sector[entry + 8..entry + 12]));
    let sectors = u64::from(le_u32(&sector[entry + 12..entry + 16]));
    let offset = start_lba.checked_mul(SECTOR_SIZE)?;
    let length = sectors.checked_mul(SECTOR_SIZE)?;

    if offset == 0 || length == 0 {
        return None;
    }

    let end = offset.checked_add(length)?;
    if end > file_size {
        return None;
    }

    Some(FatRegion {
        offset,
        length,
        layout: FatLayout::MbrPartitioned,
    })
}

fn is_fat_boot_sector(sector: &[u8; BOOT_SECTOR_BYTES]) -> bool {
    if sector[0] != 0xEB && sector[0] != 0xE9 {
        return false;
    }

    let bytes_per_sector = le_u16(&sector[11..13]);
    if !(512..=4096).contains(&bytes_per_sector) || !bytes_per_sector.is_power_of_two() {
        return false;
    }

    let sectors_per_cluster = sector[13];
    if sectors_per_cluster == 0 || !sectors_per_cluster.is_power_of_two() {
        return false;
    }

    if le_u16(&sector[14..16]) == 0 {
        return false;
    }

    let fats = sector[16];
    if !(1..=2).contains(&fats) {
        return false;
    }

    let root_entries = le_u16(&sector[17..19]);
    let total_sectors_16 = le_u16(&sector[19..21]);
    let total_sectors_32 = le_u32(&sector[32..36]);
    if total_sectors_16 == 0 && total_sectors_32 == 0 {
        return false;
    }

    if sector[21] < 0xF0 {
        return false;
    }

    let sectors_per_fat_16 = le_u16(&sector[22..24]);
    if sectors_per_fat_16 != 0 {
        return true;
    }

    root_entries == 0 && le_u32(&sector[36..40]) != 0
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// A read/write/seek view limited to one byte range of a file.
struct PartitionFile {
    file: File,
    base: u64,
    length: u64,
    position: u64,
}

impl PartitionFile {
    fn new(file: File, base: u64, length: u64) -> Self {
        Self {
            file,
            base,
            length,
            position: 0,
        }
    }

    fn remaining(&self) -> u64 {
        self.length.saturating_sub(self.position)
    }
}

impl Read for PartitionFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let limit = self.remaining().min(buf.len() as u64) as usize;
        if limit == 0 {
            return Ok(0);
        }
        self.file.seek(SeekFrom::Start(self.base + self.position))?;
        let read = self.file.read(&mut buf[..limit])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Write for PartitionFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let limit = self.remaining().min(buf.len() as u64) as usize;
        if limit == 0 && !buf.is_empty() {
            return Err(io::Error::new(
                ErrorKind::WriteZero,
                "write past the end of the partition",
            ));
        }
        self.file.seek(SeekFrom::Start(self.base + self.position))?;
        let written = self.file.write(&buf[..limit])?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for PartitionFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.length) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };
        self.position = target.clamp(0, i128::from(self.length)) as u64;
        Ok(self.position)
    }
}
