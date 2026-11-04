//! Filesystem fixtures for the runtime tests.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use dosker_core::ContainerId;
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions, format_volume};

const DISK_SIZE: u64 = 16 * 1024 * 1024;
const SECTOR_SIZE: u64 = 512;
const AUTOEXEC: &[u8] = b"@ECHO OFF\r\n";

/// Creates a container disk with a FAT16 superfloppy and an `AUTOEXEC.BAT`.
pub fn make_test_disk(root: &Path, id: &ContainerId) -> PathBuf {
    let directory = root.join("containers").join(id.as_str());
    fs::create_dir_all(&directory).expect("create the container directory");
    let path = directory.join("disk.img");

    let file = File::create(&path).expect("create the test disk");
    file.set_len(DISK_SIZE).expect("size the test disk");
    let options = FormatVolumeOptions::new()
        .fat_type(FatType::Fat16)
        .total_sectors((DISK_SIZE / SECTOR_SIZE) as u32);
    format_volume(file, options).expect("format the test disk as FAT16");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open the test disk");
    let filesystem = FileSystem::new(file, FsOptions::new()).expect("mount the test disk");
    {
        let root_dir = filesystem.root_dir();
        let mut entry = root_dir
            .create_file("AUTOEXEC.BAT")
            .expect("create AUTOEXEC.BAT");
        entry.truncate().expect("truncate AUTOEXEC.BAT");
        entry.write_all(AUTOEXEC).expect("write AUTOEXEC.BAT");
    }
    filesystem.unmount().expect("unmount the test disk");

    path
}

/// Reads the agent configuration file from a container disk.
pub fn read_config(disk: &Path) -> Vec<u8> {
    dosker_image::read_file(disk, dosker_image::CONFIG_PATH).expect("read the agent configuration")
}
