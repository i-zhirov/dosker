mod support;

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use dosker_image::{
    Attributes, BootMetadata, DosPath, ImageError, LayerDelta, Tree, TreeDir, TreeFile,
    apply_boot_metadata, fat_region, fixed_timestamp, project_tree, read_boot_metadata, read_tree,
};
use support::{make_fat16_image, make_fat16_image_with_label, write_fat_dir, write_fat_file};

const MIB: u64 = 1024 * 1024;
const SECTOR_BYTES: usize = 512;
const PARTITION_OFFSET: u64 = 1_048_576;
const PARTITION_LENGTH: u64 = 8 * 1024 * 1024;
const DISK_SIZE: u64 = 16 * 1024 * 1024;

fn mbr_disk(path: &Path) {
    make_fat16_image(path, PARTITION_OFFSET, PARTITION_LENGTH, DISK_SIZE);
}

fn superfloppy_disk(path: &Path) {
    make_fat16_image(path, 0, PARTITION_LENGTH, PARTITION_LENGTH);
}

fn tree(files: &[(&str, &[u8])], dirs: &[&str]) -> Tree {
    let mut delta = LayerDelta::new();
    for dir in dirs {
        delta.add_dir(TreeDir {
            path: DosPath::parse(dir).unwrap(),
            attributes: Attributes::default(),
            timestamp: fixed_timestamp(),
        });
    }
    for (path, bytes) in files {
        delta.add_file(TreeFile {
            path: DosPath::parse(path).unwrap(),
            attributes: Attributes::default(),
            timestamp: fixed_timestamp(),
            bytes: bytes.to_vec(),
        });
    }
    let mut tree = Tree::new();
    tree.apply_delta(&delta).unwrap();
    tree
}

fn read_at(path: &Path, offset: u64, length: usize) -> Vec<u8> {
    let mut file = File::open(path).unwrap();
    let mut bytes = vec![0u8; length];
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.read_exact(&mut bytes).unwrap();
    bytes
}

fn write_at(path: &Path, offset: u64, bytes: &[u8]) {
    let mut file = OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(bytes).unwrap();
    file.flush().unwrap();
}

fn fat32_boot_sector() -> [u8; SECTOR_BYTES] {
    let mut sector = [0u8; SECTOR_BYTES];
    sector[0] = 0xEB;
    sector[1] = 0x58;
    sector[2] = 0x90;
    sector[11..13].copy_from_slice(&512u16.to_le_bytes());
    sector[13] = 1;
    sector[14..16].copy_from_slice(&32u16.to_le_bytes());
    sector[16] = 2;
    sector[21] = 0xF8;
    sector[24..26].copy_from_slice(&32u16.to_le_bytes());
    sector[26..28].copy_from_slice(&2u16.to_le_bytes());
    sector[32..36].copy_from_slice(&2048u32.to_le_bytes());
    sector[36..40].copy_from_slice(&1u32.to_le_bytes());
    sector[66] = 0x29;
    sector[510] = 0x55;
    sector[511] = 0xAA;
    sector
}

/// Builds a disk whose current MBR points at one partition and whose effective
/// MBR points at a second, fully formatted partition further into the file.
fn moved_partition_disk(dir: &Path) -> (PathBuf, BootMetadata) {
    let source = dir.join("source.img");
    make_fat16_image(&source, 2 * MIB, PARTITION_LENGTH, DISK_SIZE);
    let boot = read_boot_metadata(&source).unwrap();

    let target = dir.join("disk.img");
    mbr_disk(&target);

    let moved = read_at(&source, 2 * MIB, (DISK_SIZE - 2 * MIB) as usize);
    write_at(&target, 2 * MIB, &moved);
    (target, boot)
}

#[test]
fn projection_round_trips_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);

    let tree = tree(
        &[
            ("APP/HELLO.TXT", b"hello\n"),
            ("AUTOEXEC.BAT", b"@ECHO OFF\r\n"),
        ],
        &["APP", "EMPTY"],
    );
    project_tree(&path, &tree).unwrap();

    assert_eq!(read_tree(&path).unwrap(), tree);
}

#[test]
fn projection_writes_an_empty_volume() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);
    write_fat_dir(&path, PARTITION_OFFSET, "OLD");
    write_fat_file(&path, PARTITION_OFFSET, "OLD.TXT", b"old");
    write_fat_file(&path, PARTITION_OFFSET, "OLD/INNER.TXT", b"inner");

    project_tree(&path, &tree(&[], &[])).unwrap();

    assert!(read_tree(&path).unwrap().is_empty());
}

#[test]
fn projection_replaces_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);
    write_fat_file(&path, PARTITION_OFFSET, "APP/HELLO.TXT", b"before");

    let tree = tree(&[("APP/HELLO.TXT", b"after")], &["APP"]);
    project_tree(&path, &tree).unwrap();

    assert_eq!(read_tree(&path).unwrap(), tree);
}

#[test]
fn projection_removes_a_file_absent_from_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);
    write_fat_file(&path, PARTITION_OFFSET, "DROP.TXT", b"drop");
    write_fat_file(&path, PARTITION_OFFSET, "KEEP.TXT", b"keep");

    let tree = tree(&[("KEEP.TXT", b"keep")], &[]);
    project_tree(&path, &tree).unwrap();

    assert_eq!(read_tree(&path).unwrap(), tree);
}

#[test]
fn projection_removes_a_directory_absent_from_the_tree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);
    write_fat_dir(&path, PARTITION_OFFSET, "OLD");
    write_fat_file(&path, PARTITION_OFFSET, "OLD/INNER.TXT", b"inner");
    write_fat_file(&path, PARTITION_OFFSET, "KEEP.TXT", b"keep");

    let tree = tree(&[("KEEP.TXT", b"keep")], &[]);
    project_tree(&path, &tree).unwrap();

    assert_eq!(read_tree(&path).unwrap(), tree);
}

#[test]
fn projection_preserves_the_mbr_and_boot_sector() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);
    let before = read_boot_metadata(&path).unwrap();
    let mbr = read_at(&path, 0, SECTOR_BYTES);
    let boot_sector = read_at(&path, PARTITION_OFFSET, SECTOR_BYTES);

    project_tree(&path, &tree(&[("HELLO.TXT", b"hello")], &[])).unwrap();

    assert_eq!(read_at(&path, 0, SECTOR_BYTES), mbr);
    assert_eq!(read_at(&path, PARTITION_OFFSET, SECTOR_BYTES), boot_sector);
    assert_eq!(read_boot_metadata(&path).unwrap(), before);
}

#[test]
fn projection_preserves_a_superfloppy_boot_sector() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    superfloppy_disk(&path);
    let before = read_boot_metadata(&path).unwrap();
    let boot_sector = read_at(&path, 0, SECTOR_BYTES);

    project_tree(&path, &tree(&[("HELLO.TXT", b"hello")], &[])).unwrap();

    assert_eq!(read_at(&path, 0, SECTOR_BYTES), boot_sector);
    assert_eq!(read_boot_metadata(&path).unwrap(), before);
}

#[test]
fn projection_preserves_disk_size_label_media_and_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    make_fat16_image_with_label(
        &path,
        PARTITION_OFFSET,
        PARTITION_LENGTH,
        DISK_SIZE,
        Some(*b"TESTVOL    "),
    );
    let before = read_boot_metadata(&path).unwrap();

    project_tree(&path, &tree(&[("HELLO.TXT", b"hello")], &[])).unwrap();

    let after = read_boot_metadata(&path).unwrap();
    assert_eq!(after.disk_size, before.disk_size);
    assert_eq!(after.geometry, before.geometry);
    assert_eq!(after.volume_label.as_deref(), Some("TESTVOL"));
    assert_eq!(after.media_descriptor, before.media_descriptor);
}

#[test]
fn projection_rejects_a_non_bootable_image() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blank.img");
    fs::write(&path, vec![0u8; MIB as usize]).unwrap();

    match project_tree(&path, &tree(&[], &[])) {
        Err(ImageError::NotBootable(reported)) => assert_eq!(reported, path),
        other => panic!("expected NotBootable, got {other:?}"),
    }
}

#[test]
fn projection_rejects_a_fat32_volume() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    fs::write(&path, vec![0u8; MIB as usize]).unwrap();
    write_at(&path, 0, &fat32_boot_sector());

    match project_tree(&path, &tree(&[], &[])) {
        Err(ImageError::Projection { .. }) => {}
        other => panic!("expected Projection, got {other:?}"),
    }
}

#[test]
fn projection_rejects_a_bpb_it_cannot_preserve() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);
    write_at(&path, PARTITION_OFFSET + 14, &2u16.to_le_bytes());

    match project_tree(&path, &tree(&[], &[])) {
        Err(ImageError::Projection { .. }) => {}
        other => panic!("expected Projection, got {other:?}"),
    }
}

#[test]
fn projection_rejects_a_non_512_byte_sector() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);
    write_at(&path, PARTITION_OFFSET + 11, &1024u16.to_le_bytes());

    match project_tree(&path, &tree(&[], &[])) {
        Err(ImageError::Projection { .. }) => {}
        other => panic!("expected Projection, got {other:?}"),
    }
}

#[test]
fn apply_boot_metadata_writes_the_mbr_and_boot_sector() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);
    let before = read_boot_metadata(&path).unwrap();
    let mbr = read_at(&path, 0, SECTOR_BYTES);
    let boot_sector = read_at(&path, PARTITION_OFFSET, SECTOR_BYTES);

    write_at(&path, 0, &[0u8; SECTOR_BYTES]);
    write_at(&path, PARTITION_OFFSET, &[0u8; SECTOR_BYTES]);
    apply_boot_metadata(&path, &before).unwrap();

    assert_eq!(read_at(&path, 0, SECTOR_BYTES), mbr);
    assert_eq!(read_at(&path, PARTITION_OFFSET, SECTOR_BYTES), boot_sector);
    assert_eq!(read_boot_metadata(&path).unwrap(), before);
}

#[test]
fn apply_boot_metadata_uses_the_override_mbr_to_locate_the_partition() {
    let dir = tempfile::tempdir().unwrap();
    let (path, boot) = moved_partition_disk(dir.path());
    assert_eq!(fat_region(&path).unwrap().offset, PARTITION_OFFSET);

    apply_boot_metadata(&path, &boot).unwrap();

    let region = fat_region(&path).unwrap();
    assert_eq!(region.offset, 2 * MIB);
    assert_eq!(region.length, PARTITION_LENGTH);
    let metadata = read_boot_metadata(&path).unwrap();
    assert_eq!(metadata.partition_start, Some((2 * MIB / 512) as u32));
    assert_eq!(
        metadata.partition_sectors,
        Some((PARTITION_LENGTH / 512) as u32)
    );
}

#[test]
fn projection_round_trips_a_moved_partition() {
    let dir = tempfile::tempdir().unwrap();
    let (path, boot) = moved_partition_disk(dir.path());
    write_fat_file(&path, PARTITION_OFFSET, "OLD.TXT", b"old");
    let old_boot_sector = read_at(&path, PARTITION_OFFSET, SECTOR_BYTES);
    apply_boot_metadata(&path, &boot).unwrap();

    let tree = tree(&[("NEW.TXT", b"new")], &[]);
    project_tree(&path, &tree).unwrap();

    assert_eq!(read_tree(&path).unwrap(), tree);
    assert_eq!(
        read_at(&path, PARTITION_OFFSET, SECTOR_BYTES),
        old_boot_sector
    );
}

#[test]
fn apply_boot_metadata_rejects_missing_inline_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);
    let mut boot = read_boot_metadata(&path).unwrap();
    boot.mbr.as_mut().unwrap().bytes_inline = None;

    match apply_boot_metadata(&path, &boot) {
        Err(ImageError::Projection { reason }) => assert!(reason.contains("MBR")),
        other => panic!("expected Projection, got {other:?}"),
    }
}

#[test]
fn apply_boot_metadata_rejects_an_opaque_layout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disk.img");
    mbr_disk(&path);

    match apply_boot_metadata(&path, &BootMetadata::opaque(DISK_SIZE)) {
        Err(ImageError::Projection { reason }) => {
            assert_eq!(reason, "an opaque image has no bootable FAT volume");
        }
        other => panic!("expected Projection, got {other:?}"),
    }
}

#[test]
fn apply_boot_metadata_rejects_a_mismatched_partition() {
    let dir = tempfile::tempdir().unwrap();
    let (path, mut boot) = moved_partition_disk(dir.path());
    boot.partition_start = Some((PARTITION_OFFSET / 512) as u32);

    match apply_boot_metadata(&path, &boot) {
        Err(ImageError::Projection { reason }) => assert!(reason.contains("partition bounds")),
        other => panic!("expected Projection, got {other:?}"),
    }
}
