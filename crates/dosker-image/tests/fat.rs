mod support;

use std::fs::{self, OpenOptions};
use std::path::Path;

use dosker_image::{CONFIG_PATH, FatLayout, ImageError, fat_region, read_file, write_config};
use support::{image_size, make_fat16_image, write_fat_file};

const MIB: u64 = 1024 * 1024;
const CONFIG_PAYLOAD: &[u8] = b"CONTROL_PORT=2\r\nBAUD=115200\r\n";

fn mbr_image(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("disk.img");
    make_fat16_image(&path, MIB, 8 * MIB, 16 * MIB);
    path
}

fn superfloppy_image(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("disk.img");
    make_fat16_image(&path, 0, 4 * MIB, 4 * MIB);
    path
}

#[test]
fn fat_region_reads_an_mbr_partition() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());

    let region = fat_region(&path).unwrap();

    assert_eq!(region.offset, MIB);
    assert_eq!(region.length, 8 * MIB);
    assert_eq!(region.layout, FatLayout::MbrPartitioned);
}

#[test]
fn fat_region_uses_the_partition_length_when_the_file_is_larger() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    make_fat16_image(&path, MIB, 2048 * 512, 8 * MIB);

    let region = fat_region(&path).unwrap();

    assert_eq!(region.offset, MIB);
    assert_eq!(region.length, 2048 * 512);
    assert_eq!(region.layout, FatLayout::MbrPartitioned);
}

#[test]
fn fat_region_accepts_a_superfloppy() {
    let dir = tempfile::tempdir().unwrap();
    let path = superfloppy_image(dir.path());

    let region = fat_region(&path).unwrap();

    assert_eq!(region.offset, 0);
    assert_eq!(region.length, image_size(&path));
    assert_eq!(region.layout, FatLayout::Superfloppy);
}

#[test]
fn fat_region_rejects_a_non_bootable_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    fs::write(&path, vec![0u8; 4096]).unwrap();

    assert!(matches!(fat_region(&path), Err(ImageError::NotBootable(_))));
}

#[test]
fn fat_region_rejects_a_truncated_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    fs::write(&path, vec![0u8; 100]).unwrap();

    assert!(matches!(fat_region(&path), Err(ImageError::NotBootable(_))));
}

#[test]
fn fat_region_rejects_a_partition_past_the_end_of_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    make_fat16_image(&path, MIB, 2 * MIB, 4 * MIB);
    let file = OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(MIB + MIB / 2).unwrap();

    assert!(matches!(fat_region(&path), Err(ImageError::NotBootable(_))));
}

#[test]
fn write_config_creates_the_directory_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());

    write_config(&path, 2, 115200).unwrap();

    assert_eq!(read_file(&path, CONFIG_PATH).unwrap(), CONFIG_PAYLOAD);
}

#[test]
fn write_config_uses_crlf_and_the_pinned_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());

    write_config(&path, 2, 115200).unwrap();

    let payload = read_file(&path, CONFIG_PATH).unwrap();
    assert_eq!(payload, b"CONTROL_PORT=2\r\nBAUD=115200\r\n");
    assert_eq!(payload, CONFIG_PAYLOAD);
}

#[test]
fn write_config_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());

    write_config(&path, 2, 115200).unwrap();
    write_config(&path, 2, 115200).unwrap();

    assert_eq!(read_file(&path, CONFIG_PATH).unwrap(), CONFIG_PAYLOAD);
}

#[test]
fn write_config_preserves_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());
    write_fat_file(&path, MIB, "APP/KEEP.TXT", b"KEEP");

    write_config(&path, 2, 115200).unwrap();

    assert_eq!(read_file(&path, r"C:\APP\KEEP.TXT").unwrap(), b"KEEP");
    assert_eq!(read_file(&path, CONFIG_PATH).unwrap(), CONFIG_PAYLOAD);
}

#[test]
fn read_file_returns_the_config_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());
    write_config(&path, 3, 9600).unwrap();

    assert_eq!(
        read_file(&path, CONFIG_PATH).unwrap(),
        b"CONTROL_PORT=3\r\nBAUD=9600\r\n"
    );
}

#[test]
fn read_file_reports_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = mbr_image(dir.path());

    match read_file(&path, CONFIG_PATH) {
        Err(ImageError::Corrupt { message, .. }) => {
            assert!(message.contains(CONFIG_PATH), "message: {message}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}
