use dosker_image::{BlobRef, BootLayout, BootMetadata, FatType, Geometry};

#[test]
fn opaque_boot_has_the_opaque_layout() {
    let metadata = BootMetadata::opaque(1024);
    assert_eq!(metadata.layout, BootLayout::Opaque);
    assert_eq!(metadata.disk_size, 1024);
    assert!(metadata.geometry.sectors == 0);
}

#[test]
fn validate_requires_partition_fields_for_mbr_partitioned() {
    let mut metadata = BootMetadata {
        layout: BootLayout::MbrPartitioned,
        disk_size: 1024,
        geometry: Geometry {
            cylinders: 1,
            heads: 1,
            sectors: 1,
            sector_size: 512,
        },
        partition_start: None,
        partition_sectors: None,
        mbr: None,
        boot_sector: None,
        fat_type: Some(FatType::Fat16),
        volume_label: None,
        media_descriptor: None,
    };
    assert!(metadata.validate().is_err());
    metadata.partition_start = Some(1);
    metadata.partition_sectors = Some(2);
    assert!(metadata.validate().is_err());
}

#[test]
fn validate_allows_the_opaque_layout_without_partition_fields() {
    assert!(BootMetadata::opaque(1024).validate().is_ok());
}

#[test]
fn blob_ref_round_trips_bytes_inline() {
    let reference = BlobRef::from_bytes(b"boot");
    assert_eq!(reference.sha256, dosker_image::sha256_digest(b"boot"));
    assert!(reference.verify(b"boot"));
    assert!(!reference.verify(b"other"));
}

#[test]
fn blob_ref_verify_rejects_different_bytes() {
    assert!(!BlobRef::from_bytes(b"a").verify(b"b"));
}

#[test]
fn boot_metadata_round_trips_through_json() {
    let metadata = BootMetadata::opaque(1024);
    let json = serde_json::to_string(&metadata).unwrap();
    assert_eq!(
        serde_json::from_str::<BootMetadata>(&json).unwrap(),
        metadata
    );
}

#[test]
fn volume_label_is_limited_to_eleven_characters() {
    let mut metadata = BootMetadata::opaque(1024);
    metadata.volume_label = Some("012345678901".to_owned());
    assert!(metadata.validate().is_err());
}

fn mbr_boot(sector_byte: u8) -> BootMetadata {
    BootMetadata {
        layout: BootLayout::MbrPartitioned,
        disk_size: 16 * 1024 * 1024,
        geometry: Geometry {
            cylinders: 16,
            heads: 16,
            sectors: 32,
            sector_size: 512,
        },
        partition_start: Some(2048),
        partition_sectors: Some(16_384),
        mbr: Some(BlobRef::from_bytes(&[0x11; 512])),
        boot_sector: Some(BlobRef::from_bytes(&[sector_byte; 512])),
        fat_type: Some(FatType::Fat16),
        volume_label: Some("DOSKER".to_owned()),
        media_descriptor: Some(0xF8),
    }
}

#[test]
fn boot_artifacts_ignore_inline_bytes() {
    let with_inline = mbr_boot(0xAA);
    let mut without_inline = with_inline.clone();
    without_inline.mbr.as_mut().unwrap().bytes_inline = None;
    without_inline.boot_sector.as_mut().unwrap().bytes_inline = None;

    assert!(with_inline.boot_artifacts_equal(&without_inline));
    assert!(without_inline.boot_artifacts_equal(&with_inline));
}

#[test]
fn boot_artifacts_detect_a_changed_boot_sector() {
    let before = mbr_boot(0xAA);
    let after = mbr_boot(0xAB);

    assert!(!before.boot_artifacts_equal(&after));
    assert!(!after.boot_artifacts_equal(&before));
}

#[test]
fn boot_artifacts_detect_a_changed_mbr() {
    let before = mbr_boot(0xAA);
    let mut after = before.clone();
    after.mbr = Some(BlobRef::from_bytes(&[0x22; 512]));

    assert!(!before.boot_artifacts_equal(&after));
    assert!(!after.boot_artifacts_equal(&before));
}

#[test]
fn boot_artifacts_detect_changed_partition_bounds() {
    let base = mbr_boot(0xAA);

    let mut moved_start = base.clone();
    moved_start.partition_start = Some(4096);
    assert!(!base.boot_artifacts_equal(&moved_start));

    let mut resized = base.clone();
    resized.partition_sectors = Some(8192);
    assert!(!base.boot_artifacts_equal(&resized));

    let mut resized_disk = base.clone();
    resized_disk.disk_size += 512;
    assert!(!base.boot_artifacts_equal(&resized_disk));
}

#[test]
fn boot_artifacts_detect_a_changed_geometry() {
    let base = mbr_boot(0xAA);

    let mut changed_heads = base.clone();
    changed_heads.geometry.heads = 8;
    assert!(!base.boot_artifacts_equal(&changed_heads));

    let mut changed_sectors = base.clone();
    changed_sectors.geometry.sectors = 63;
    assert!(!base.boot_artifacts_equal(&changed_sectors));
}
