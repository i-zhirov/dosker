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
