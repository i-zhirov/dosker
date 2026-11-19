use std::fs;
use std::path::{Path, PathBuf};

use dosker_image::{
    DEFAULT_VOLUME_SIZE, FLOPPY_1440_SIZE, FLOPPY_2880_SIZE, FatType, ImageError, VolumeLayout,
    create_volume, read_tree, sha256_digest,
};

const PARTITION_TABLE_OFFSET: usize = 446;

fn create(dir: &Path, size: u64, name: &str) -> (PathBuf, dosker_image::VolumeImage) {
    let path = dir.join("volume.img");
    let image = create_volume(&path, size, name).expect("create volume");
    (path, image)
}

fn read_sector(path: &Path, offset: u64) -> [u8; 512] {
    let bytes = fs::read(path).expect("read image");
    let start = offset as usize;
    let mut sector = [0u8; 512];
    sector.copy_from_slice(&bytes[start..start + 512]);
    sector
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn decode_chs(bytes: &[u8]) -> u32 {
    let head = u32::from(bytes[0]);
    let sector = u32::from(bytes[1] & 0x3F);
    let cylinder = (u32::from(bytes[1] & 0xC0) << 2) | u32::from(bytes[2]);
    (cylinder * 16 + head) * 32 + (sector - 1)
}

fn expected_volume_id(name: &str) -> u32 {
    let digest = sha256_digest(name.as_bytes());
    let hex = digest.strip_prefix("sha256:").expect("digest prefix");
    let mut bytes = [0u8; 4];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("hex byte");
    }
    u32::from_le_bytes(bytes)
}

fn expected_cluster_bytes(size: u64) -> u64 {
    (size / 32_768).next_power_of_two().clamp(512, 32_768)
}

#[test]
fn the_default_size_creates_a_partitioned_fat16_volume() {
    let dir = tempfile::tempdir().unwrap();
    let (path, image) = create(dir.path(), DEFAULT_VOLUME_SIZE, "alpha");

    assert_eq!(image.layout, VolumeLayout::MbrPartitioned);
    assert_eq!(image.fat_type, FatType::Fat16);
    assert_eq!(image.partition_start, Some(2048));
    assert_eq!(image.disk_size, fs::metadata(&path).unwrap().len());
    assert_eq!(image.disk_size, DEFAULT_VOLUME_SIZE);
    assert_eq!(image.volume_id, expected_volume_id("alpha"));
}

#[test]
fn a_partitioned_volume_has_no_files() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = create(dir.path(), DEFAULT_VOLUME_SIZE, "alpha");

    assert!(read_tree(&path).unwrap().is_empty());
}

#[test]
fn a_floppy_size_creates_a_superfloppy_volume() {
    for size in [FLOPPY_1440_SIZE, FLOPPY_2880_SIZE] {
        let dir = tempfile::tempdir().unwrap();
        let (_, image) = create(dir.path(), size, "alpha");

        assert_eq!(image.layout, VolumeLayout::Superfloppy);
        assert_eq!(image.partition_start, None);
        assert_eq!(image.partition_sectors, None);
        assert_eq!(image.disk_size, size);
    }
}

#[test]
fn the_1440_floppy_is_fat12_and_the_2880_floppy_is_fat16() {
    let dir = tempfile::tempdir().unwrap();
    let (_, floppy) = create(dir.path(), FLOPPY_1440_SIZE, "alpha");
    assert_eq!(floppy.fat_type, FatType::Fat12);

    let dir = tempfile::tempdir().unwrap();
    let (_, floppy) = create(dir.path(), FLOPPY_2880_SIZE, "alpha");
    assert_eq!(floppy.fat_type, FatType::Fat16);
}

#[test]
fn the_partitioned_bpb_has_the_pinned_fields() {
    let dir = tempfile::tempdir().unwrap();
    let (path, image) = create(dir.path(), DEFAULT_VOLUME_SIZE, "alpha");
    let sector = read_sector(&path, 2048 * 512);
    let cluster_bytes = expected_cluster_bytes(DEFAULT_VOLUME_SIZE);

    assert_eq!(sector[0], 0xEB);
    assert_eq!(le_u16(&sector[11..13]), 512);
    assert_eq!(u64::from(sector[13]), cluster_bytes / 512);
    assert_eq!(le_u16(&sector[14..16]), 1);
    assert_eq!(sector[16], 2);
    assert_eq!(le_u16(&sector[17..19]), 512);
    assert_eq!(le_u32(&sector[28..32]), 2048);
    assert_eq!(sector[21], 0xF8);
    assert_eq!(le_u16(&sector[24..26]), 32);
    assert_eq!(le_u16(&sector[26..28]), 16);
    assert_eq!(sector[36], 0x80);
    assert_eq!(sector[38], 0x29);
    assert_eq!(le_u32(&sector[39..43]), image.volume_id);
    assert_eq!(&sector[43..54], b"DOSKER     ");
    assert_eq!(&sector[54..62], b"FAT16   ");
    assert_eq!(&sector[510..512], &[0x55, 0xAA]);
}

#[test]
fn the_mbr_describes_one_bootable_free_partition() {
    let dir = tempfile::tempdir().unwrap();
    let size = DEFAULT_VOLUME_SIZE;
    let (path, _) = create(dir.path(), size, "alpha");
    let sector = read_sector(&path, 0);
    let entry = &sector[PARTITION_TABLE_OFFSET..PARTITION_TABLE_OFFSET + 16];
    let sectors = (size / 512) as u32 - 2048;

    assert_eq!(entry[0], 0x00);
    assert_eq!(entry[4], 0x06);
    assert_eq!(le_u32(&entry[8..12]), 2048);
    assert_eq!(le_u32(&entry[12..16]), sectors);
    assert_eq!(&sector[510..512], &[0x55, 0xAA]);

    assert_eq!(decode_chs(&entry[1..4]), 2048);
    assert_eq!(entry[1], 0);
    assert_eq!(entry[2] & 0x3F, 1);
    assert_eq!((u32::from(entry[2] & 0xC0) << 2) | u32::from(entry[3]), 4);
    assert_eq!(decode_chs(&entry[5..8]), 2048 + sectors - 1);
}

#[test]
fn the_mbr_chs_round_trips_within_the_chs_range() {
    for size in [16 * 1024 * 1024, 64 * 1024 * 1024] {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = create(dir.path(), size, "alpha");
        let sector = read_sector(&path, 0);
        let entry = &sector[PARTITION_TABLE_OFFSET..PARTITION_TABLE_OFFSET + 16];
        let sectors = (size / 512) as u32 - 2048;
        let end_lba = 2048 + sectors - 1;

        assert!(end_lba <= 524_287);
        assert_eq!(decode_chs(&entry[1..4]), 2048);
        assert_eq!(decode_chs(&entry[5..8]), end_lba);
    }
}

#[test]
fn the_mbr_chs_caps_an_end_lba_above_the_chs_range() {
    let dir = tempfile::tempdir().unwrap();
    let size = 1024 * 1024 * 1024;
    let (path, _) = create(dir.path(), size, "alpha");
    let sector = read_sector(&path, 0);
    let entry = &sector[PARTITION_TABLE_OFFSET..PARTITION_TABLE_OFFSET + 16];

    assert_eq!(decode_chs(&entry[1..4]), 2048);
    assert_eq!(&entry[5..8], &[0x0F, 0xE0, 0xFF]);
    assert_eq!(le_u32(&entry[8..12]), 2048);
    assert_eq!(le_u32(&entry[12..16]), (size / 512) as u32 - 2048);
}

#[test]
fn the_superfloppy_bpb_has_the_pinned_fields() {
    let cases: [(u64, u16, &[u8]); 2] = [
        (FLOPPY_1440_SIZE, 18, b"FAT12   "),
        (FLOPPY_2880_SIZE, 36, b"FAT16   "),
    ];

    for (size, sectors_per_track, fs_type) in cases {
        let dir = tempfile::tempdir().unwrap();
        let (path, image) = create(dir.path(), size, "alpha");
        let sector = read_sector(&path, 0);

        assert_eq!(sector[0], 0xEB);
        assert_eq!(le_u16(&sector[11..13]), 512);
        assert_eq!(sector[13], 1);
        assert_eq!(le_u16(&sector[14..16]), 1);
        assert_eq!(sector[16], 2);
        assert_eq!(le_u16(&sector[17..19]), 224);
        assert_eq!(le_u32(&sector[28..32]), 0);
        assert_eq!(sector[21], 0xF0);
        assert_eq!(le_u16(&sector[24..26]), sectors_per_track);
        assert_eq!(le_u16(&sector[26..28]), 2);
        assert_eq!(sector[36], 0x00);
        assert_eq!(sector[38], 0x29);
        assert_eq!(le_u32(&sector[39..43]), image.volume_id);
        assert_eq!(&sector[43..54], b"DOSKER     ");
        assert_eq!(&sector[54..62], fs_type);
        assert_eq!(&sector[510..512], &[0x55, 0xAA]);
    }
}

#[test]
fn partitioned_cluster_sizes_stay_within_fat16() {
    for size in [16 * 1024 * 1024, 64 * 1024 * 1024, 300 * 1024 * 1024] {
        let dir = tempfile::tempdir().unwrap();
        let (path, image) = create(dir.path(), size, "alpha");

        assert_eq!(image.fat_type, FatType::Fat16);
        assert!(read_tree(&path).is_ok());
    }
}

#[test]
fn a_size_that_is_not_a_multiple_of_512_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("volume.img");

    let error = create_volume(&path, 1_474_561, "alpha").unwrap_err();

    assert!(matches!(error, ImageError::InvalidVolumeSize { .. }));
}

#[test]
fn a_size_below_the_partitioned_minimum_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("volume.img");

    let error = create_volume(&path, 1_048_576, "alpha").unwrap_err();

    assert!(matches!(error, ImageError::InvalidVolumeSize { .. }));
}

#[test]
fn a_size_above_the_maximum_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("volume.img");

    let error = create_volume(&path, 2_147_483_648, "alpha").unwrap_err();

    assert!(matches!(error, ImageError::InvalidVolumeSize { .. }));
}

#[test]
fn the_volume_id_depends_on_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let first = create_volume(&dir.path().join("first.img"), DEFAULT_VOLUME_SIZE, "alpha").unwrap();
    let second =
        create_volume(&dir.path().join("second.img"), DEFAULT_VOLUME_SIZE, "beta").unwrap();

    assert_ne!(first.volume_id, second.volume_id);
    assert_eq!(first.volume_id, expected_volume_id("alpha"));
}
