//! Access to the FAT volume inside a raw DOS disk image.

use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::{Duration, TimeZone, Utc};
use fatfs::{Dir, FileAttributes, FileSystem, FsOptions};

use crate::boot::{BlobRef, BootLayout, BootMetadata, FatType, Geometry};
use crate::error::ImageError;
use crate::path::{DosName, DosPath};
use crate::tree::{Attributes, LayerDelta, Tree, TreeDir, TreeFile};

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
    fat_region_from(&mut file, image)
}

/// Detects the FAT region from the first sector and the reader's end offset.
fn fat_region_from<R: Read + Seek>(source: &mut R, path: &Path) -> Result<FatRegion, ImageError> {
    let file_size = source
        .seek(SeekFrom::End(0))
        .map_err(|source| image_io(path, source))?;
    source
        .seek(SeekFrom::Start(0))
        .map_err(|source| image_io(path, source))?;

    let mut sector = [0u8; BOOT_SECTOR_BYTES];
    match source.read_exact(&mut sector) {
        Ok(()) => {}
        Err(source) if source.kind() == ErrorKind::UnexpectedEof => {
            return Err(ImageError::NotBootable(path.to_path_buf()));
        }
        Err(source) => return Err(image_io(path, source)),
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

    Err(ImageError::NotBootable(path.to_path_buf()))
}

/// The fixed filesystem time used for generated image entries.
#[derive(Debug)]
pub(crate) struct FatFixedTime;

impl fatfs::TimeProvider for FatFixedTime {
    fn get_current_date(&self) -> fatfs::Date {
        fatfs::Date {
            year: 1980,
            month: 1,
            day: 1,
        }
    }

    fn get_current_date_time(&self) -> fatfs::DateTime {
        fatfs::DateTime {
            date: self.get_current_date(),
            time: fatfs::Time {
                hour: 0,
                min: 0,
                sec: 0,
                millis: 0,
            },
        }
    }
}

/// The single fixed-time provider for generated FAT volumes.
pub(crate) static FAT_FIXED_TIME: FatFixedTime = FatFixedTime;

/// Mount options that stamp every generated entry with the fixed time.
pub(crate) fn fixed_fs_options() -> FsOptions {
    FsOptions::new().time_provider(&FAT_FIXED_TIME)
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

/// Reads the FAT tree rooted at `C:\`.
pub fn read_tree(image: &Path) -> Result<Tree, ImageError> {
    let file = File::open(image).map_err(|source| ImageError::Io {
        path: image.to_path_buf(),
        source,
    })?;
    read_tree_from(file, image)
}

/// Reads the FAT tree rooted at `C:\` from any seekable reader.
pub(crate) fn read_tree_from<R: Read + Write + Seek>(
    mut source: R,
    path: &Path,
) -> Result<Tree, ImageError> {
    let region = fat_region_from(&mut source, path)?;
    let disk = PartitionFile::new(source, region.offset, region.length);
    let fs = FileSystem::new(disk, FsOptions::new())
        .map_err(|source| corrupt(path, format!("cannot mount FAT volume: {source}")))?;
    let mut delta = LayerDelta::new();
    walk_dir(&fs.root_dir(), &DosPath::parse("C:")?, &mut delta)?;
    let mut tree = Tree::new();
    tree.apply_delta(&delta)?;
    Ok(tree)
}

/// Reads the boot metadata of a raw DOS image.
pub fn read_boot_metadata(image: &Path) -> Result<BootMetadata, ImageError> {
    let file = File::open(image).map_err(|source| ImageError::Io {
        path: image.to_path_buf(),
        source,
    })?;
    read_boot_metadata_from(file, image)
}

/// Reads the boot metadata of a raw DOS image from any seekable reader.
pub(crate) fn read_boot_metadata_from<R: Read + Write + Seek>(
    mut source: R,
    path: &Path,
) -> Result<BootMetadata, ImageError> {
    let region = fat_region_from(&mut source, path)?;
    let disk_size = source
        .seek(SeekFrom::End(0))
        .map_err(|source| image_io(path, source))?;

    let mut boot_sector = [0u8; BOOT_SECTOR_BYTES];
    source
        .seek(SeekFrom::Start(region.offset))
        .and_then(|_| source.read_exact(&mut boot_sector))
        .map_err(|source| image_io(path, source))?;
    let mbr = match region.layout {
        FatLayout::MbrPartitioned => {
            let mut sector = [0u8; BOOT_SECTOR_BYTES];
            source
                .seek(SeekFrom::Start(0))
                .and_then(|_| source.read_exact(&mut sector))
                .map_err(|source| image_io(path, source))?;
            Some(BlobRef::from_bytes(&sector))
        }
        FatLayout::Superfloppy => None,
    };

    let disk = PartitionFile::new(source, region.offset, region.length);
    let fs = FileSystem::new(disk, FsOptions::new())
        .map_err(|source| corrupt(path, format!("cannot mount FAT volume: {source}")))?;
    let fat_type = match fs.fat_type() {
        fatfs::FatType::Fat12 => FatType::Fat12,
        fatfs::FatType::Fat16 => FatType::Fat16,
        fatfs::FatType::Fat32 => FatType::Fat32,
    };
    let label = fs.volume_label().trim_end().to_owned();
    let volume_label = if label.is_empty() { None } else { Some(label) };
    let (sector_size, heads, sectors) = read_bpb_geometry(&boot_sector);
    let total_sectors = if sector_size == 0 {
        0
    } else {
        region.length / u64::from(sector_size)
    };
    let cylinders = if heads == 0 || sectors == 0 {
        0
    } else {
        (total_sectors / (u64::from(heads) * u64::from(sectors))) as u32
    };
    let geometry = Geometry {
        cylinders,
        heads,
        sectors,
        sector_size,
    };
    let layout = match region.layout {
        FatLayout::MbrPartitioned => BootLayout::MbrPartitioned,
        FatLayout::Superfloppy => BootLayout::Superfloppy,
    };
    let (partition_start, partition_sectors) = match layout {
        BootLayout::MbrPartitioned => (
            Some((region.offset / SECTOR_SIZE) as u32),
            Some((region.length / SECTOR_SIZE) as u32),
        ),
        _ => (None, None),
    };
    Ok(BootMetadata {
        layout,
        disk_size,
        geometry,
        partition_start,
        partition_sectors,
        mbr,
        boot_sector: Some(BlobRef::from_bytes(&boot_sector)),
        fat_type: Some(fat_type),
        volume_label,
        media_descriptor: Some(boot_sector[21]),
    })
}

fn walk_dir<R: Read + Write + Seek>(
    dir: &Dir<'_, PartitionFile<R>>,
    prefix: &DosPath,
    delta: &mut LayerDelta,
) -> Result<(), ImageError> {
    for entry in dir.iter() {
        let entry = entry.map_err(|source| ImageError::Corrupt {
            path: PathBuf::from("fat"),
            message: format!("cannot read directory entry: {source}"),
        })?;
        let attributes = entry.attributes();
        if attributes.contains(FileAttributes::VOLUME_ID) {
            continue;
        }
        let short = entry.short_file_name();
        let long = entry.file_name();
        if long == "." || long == ".." {
            continue;
        }
        if !long.eq_ignore_ascii_case(&short) {
            return Err(ImageError::InvalidDosPath {
                path: long,
                reason: "long file names are not supported".to_owned(),
            });
        }
        let name = DosName::normalize(&short)?;
        let path = prefix.join(&name);
        let timestamp = fat_timestamp(entry.modified());
        let attributes = Attributes {
            read_only: attributes.contains(FileAttributes::READ_ONLY),
            hidden: attributes.contains(FileAttributes::HIDDEN),
            system: attributes.contains(FileAttributes::SYSTEM),
            archive: attributes.contains(FileAttributes::ARCHIVE),
        };
        if entry.is_dir() {
            delta.add_dir(TreeDir {
                path: path.clone(),
                attributes,
                timestamp,
            });
            let child = entry.to_dir();
            walk_dir(&child, &path, delta)?;
        } else if entry.is_file() {
            let mut file = entry.to_file();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|source| ImageError::Corrupt {
                    path: PathBuf::from("fat"),
                    message: format!("cannot read {}: {source}", path),
                })?;
            delta.add_file(TreeFile {
                path,
                attributes,
                timestamp,
                bytes,
            });
        } else {
            return Err(ImageError::InvalidDosPath {
                path: short,
                reason: "not a regular file or directory".to_owned(),
            });
        }
    }
    Ok(())
}

fn fat_timestamp(value: fatfs::DateTime) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(
        i32::from(value.date.year),
        u32::from(value.date.month),
        u32::from(value.date.day),
        u32::from(value.time.hour),
        u32::from(value.time.min),
        u32::from(value.time.sec),
    )
    .single()
    .map(|time| time + Duration::milliseconds(i64::from(value.time.millis)))
    .unwrap_or_else(|| {
        Utc.timestamp_opt(315_532_800, 0)
            .single()
            .expect("fixed timestamp")
    })
}

fn read_bpb_geometry(sector: &[u8; BOOT_SECTOR_BYTES]) -> (u32, u32, u32) {
    (
        u32::from(le_u16(&sector[11..13])),
        u32::from(le_u16(&sector[26..28])),
        u32::from(le_u16(&sector[24..26])),
    )
}

/// Reads a file by DOS path, returning `None` only when the file is absent.
pub(crate) fn read_file_if_present(
    image: &Path,
    dos_path: &str,
) -> Result<Option<Vec<u8>>, ImageError> {
    let region = fat_region(image)?;
    let file = File::open(image).map_err(|source| ImageError::Io {
        path: image.to_path_buf(),
        source,
    })?;
    let disk = PartitionFile::new(file, region.offset, region.length);
    let fs = FileSystem::new(disk, FsOptions::new())
        .map_err(|source| corrupt(image, format!("cannot mount FAT volume: {source}")))?;
    let root = fs.root_dir();
    let mut entry = match root.open_file(&fat_path(dos_path)) {
        Ok(entry) => entry,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(corrupt(
                image,
                format!("file {dos_path} is unreadable: {source}"),
            ));
        }
    };

    let mut data = Vec::new();
    entry
        .read_to_end(&mut data)
        .map_err(|source| corrupt(image, format!("cannot read {dos_path}: {source}")))?;
    Ok(Some(data))
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

fn image_io(path: &Path, source: io::Error) -> ImageError {
    ImageError::Io {
        path: path.to_path_buf(),
        source,
    }
}

pub(crate) fn fat_path(dos_path: &str) -> String {
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

/// A read/write/seek view limited to one byte range of a readable source.
pub(crate) struct PartitionFile<R> {
    inner: R,
    base: u64,
    length: u64,
    position: u64,
}

impl<R: Read + Write + Seek> PartitionFile<R> {
    pub(crate) fn new(inner: R, base: u64, length: u64) -> Self {
        Self {
            inner,
            base,
            length,
            position: 0,
        }
    }

    fn remaining(&self) -> u64 {
        self.length.saturating_sub(self.position)
    }
}

impl<R: Read + Write + Seek> Read for PartitionFile<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let limit = self.remaining().min(buf.len() as u64) as usize;
        if limit == 0 {
            return Ok(0);
        }
        self.inner
            .seek(SeekFrom::Start(self.base + self.position))?;
        let read = self.inner.read(&mut buf[..limit])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl<R: Read + Write + Seek> Write for PartitionFile<R> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let limit = self.remaining().min(buf.len() as u64) as usize;
        if limit == 0 && !buf.is_empty() {
            return Err(io::Error::new(
                ErrorKind::WriteZero,
                "write past the end of the partition",
            ));
        }
        self.inner
            .seek(SeekFrom::Start(self.base + self.position))?;
        let written = self.inner.write(&buf[..limit])?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<R: Read + Write + Seek> Seek for PartitionFile<R> {
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
