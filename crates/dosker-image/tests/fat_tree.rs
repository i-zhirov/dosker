mod support;

use std::fs;

use dosker_image::{BootLayout, FatType, read_boot_metadata, read_tree};
use support::{make_fat16_image, make_fat16_image_with_label, write_fat_dir, write_fat_file};

const MIB: u64 = 1024 * 1024;

fn mbr_image(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("disk.img");
    make_fat16_image(&path, MIB, 8 * MIB, 16 * MIB);
    path
}

fn superfloppy_image(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("disk.img");
    make_fat16_image(&path, 0, 4 * MIB, 4 * MIB);
    path
}

#[test]
fn read_tree_reads_files_directories_attributes_and_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());
    write_fat_dir(&path, MIB, "APP");
    write_fat_file(&path, MIB, "APP/FOO.TXT", b"hello");
    let tree = read_tree(&path).unwrap();
    assert!(
        tree.get(&dosker_image::DosPath::parse("APP").unwrap())
            .is_some()
    );
    let file = tree
        .get(&dosker_image::DosPath::parse("APP/FOO.TXT").unwrap())
        .unwrap();
    assert!(matches!(file, dosker_image::TreeNode::File(_)));
}

#[test]
fn read_tree_reads_file_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());
    write_fat_file(&path, MIB, "A.TXT", b"hello");
    let tree = read_tree(&path).unwrap();
    assert_eq!(tree.files().next().unwrap().bytes, b"hello");
}

#[test]
fn read_tree_rejects_a_long_file_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());
    write_fat_file(&path, MIB, "LONGFILENAME.TXT", b"hello");
    assert!(read_tree(&path).is_err());
}

#[test]
fn read_tree_skips_the_volume_label() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    make_fat16_image_with_label(&path, MIB, 8 * MIB, 16 * MIB, Some(*b"TESTVOL    "));
    let tree = read_tree(&path).unwrap();
    assert!(tree.is_empty());
}

#[test]
fn read_tree_preserves_an_empty_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());
    write_fat_dir(&path, MIB, "EMPTY");
    let tree = read_tree(&path).unwrap();
    assert!(
        tree.get(&dosker_image::DosPath::parse("EMPTY").unwrap())
            .is_some()
    );
}

#[test]
fn read_boot_metadata_reads_the_mbr_partition_and_boot_sector() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());
    let metadata = read_boot_metadata(&path).unwrap();
    assert_eq!(metadata.layout, BootLayout::MbrPartitioned);
    assert_eq!(metadata.disk_size, 16 * MIB);
    assert_eq!(metadata.partition_start, Some((MIB / 512) as u32));
    assert_eq!(metadata.partition_sectors, Some((8 * MIB / 512) as u32));
    assert_eq!(metadata.fat_type, Some(FatType::Fat16));
    assert!(metadata.mbr.is_some());
    assert!(metadata.boot_sector.is_some());
}

#[test]
fn read_boot_metadata_hashes_the_mbr_and_boot_sector() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());
    let bytes = fs::read(&path).unwrap();
    let metadata = read_boot_metadata(&path).unwrap();
    assert!(metadata.mbr.as_ref().unwrap().verify(&bytes[..512]));
    assert!(
        metadata
            .boot_sector
            .as_ref()
            .unwrap()
            .verify(&bytes[MIB as usize..MIB as usize + 512])
    );
}

#[test]
fn read_boot_metadata_reads_a_superfloppy() {
    let dir = tempfile::tempdir().unwrap();
    let path = superfloppy_image(dir.path());
    let metadata = read_boot_metadata(&path).unwrap();
    assert_eq!(metadata.layout, BootLayout::Superfloppy);
    assert!(metadata.mbr.is_none());
    assert!(metadata.boot_sector.is_some());
}

#[test]
fn read_boot_metadata_rejects_a_non_bootable_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blank.img");
    fs::write(&path, vec![0u8; 4096]).unwrap();
    assert!(read_boot_metadata(&path).is_err());
}
