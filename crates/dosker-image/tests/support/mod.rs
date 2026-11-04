#![allow(dead_code)]

//! FAT image fixtures for the image tests.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions, format_volume};

const SECTOR_SIZE: u64 = 512;
const PARTITION_TABLE_OFFSET: usize = 446;
const PARTITION_TYPE_FAT16: u8 = 0x06;

/// Creates a FAT16 image, optionally with a DOS MBR in front of the volume.
pub fn make_fat16_image(
    path: &Path,
    partition_offset: u64,
    partition_length: u64,
    total_size: u64,
) {
    let file = File::create(path).expect("create image");
    file.set_len(total_size).expect("size image");

    if partition_offset != 0 {
        write_mbr(path, partition_offset, partition_length);
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open image");
    let region = Region::new(file, partition_offset, partition_length);
    let options = FormatVolumeOptions::new()
        .fat_type(FatType::Fat16)
        .total_sectors((partition_length / SECTOR_SIZE) as u32);
    format_volume(region, options).expect("format FAT16");
}

/// Writes a file into the FAT volume at `partition_offset`.
pub fn write_fat_file(path: &Path, partition_offset: u64, name: &str, data: &[u8]) {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open image");
    let file_size = file.metadata().expect("image metadata").len();
    let region = Region::new(file, partition_offset, file_size - partition_offset);
    let fs = FileSystem::new(region, FsOptions::new()).expect("mount FAT");

    {
        let root = fs.root_dir();
        let mut dir = root;
        let parts: Vec<&str> = name.split('/').filter(|part| !part.is_empty()).collect();
        let (file_name, parent_parts) = parts.split_last().expect("file name");
        for part in parent_parts {
            dir = dir.create_dir(part).expect("create directory");
        }
        let mut entry = dir.create_file(file_name).expect("create file");
        entry.truncate().expect("truncate file");
        entry.write_all(data).expect("write file");
    }

    fs.unmount().expect("unmount FAT");
}

fn write_mbr(path: &Path, partition_offset: u64, partition_length: u64) {
    let mut sector = [0u8; SECTOR_SIZE as usize];
    let entry = PARTITION_TABLE_OFFSET;
    sector[entry] = 0x80;
    sector[entry + 4] = PARTITION_TYPE_FAT16;
    let start_lba = (partition_offset / SECTOR_SIZE) as u32;
    let sectors = (partition_length / SECTOR_SIZE) as u32;
    sector[entry + 8..entry + 12].copy_from_slice(&start_lba.to_le_bytes());
    sector[entry + 12..entry + 16].copy_from_slice(&sectors.to_le_bytes());
    sector[510] = 0x55;
    sector[511] = 0xAA;

    let mut file = OpenOptions::new().write(true).open(path).expect("open MBR");
    file.seek(SeekFrom::Start(0)).expect("seek MBR");
    file.write_all(&sector).expect("write MBR");
}

/// Returns the on-disk size of a test image.
pub fn image_size(path: &Path) -> u64 {
    fs::metadata(path).expect("image metadata").len()
}

/// A read/write/seek view over one region of a file.
struct Region {
    file: File,
    base: u64,
    length: u64,
    position: u64,
}

impl Region {
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

impl Read for Region {
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

impl Write for Region {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let limit = self.remaining().min(buf.len() as u64) as usize;
        if limit == 0 && !buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "write past the end of the region",
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

impl Seek for Region {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::End(offset) => self.length as i128 + i128::from(offset),
            SeekFrom::Current(offset) => self.position as i128 + i128::from(offset),
        };
        self.position = target.clamp(0, self.length as i128) as u64;
        Ok(self.position)
    }
}
