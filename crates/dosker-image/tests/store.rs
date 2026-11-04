mod support;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use dosker_image::image::IMAGE_SCHEMA_VERSION;
use dosker_image::{
    IMAGE_RECORD_FILE, ImageError, ImageMedia, ImageRecord, ImageReference, ImageStore,
};
use dosker_platform::CopyOutcome;
use sha2::{Digest, Sha256};
use support::make_fat16_image;

const MIB: u64 = 1024 * 1024;

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn reference(text: &str) -> ImageReference {
    ImageReference::parse(text).unwrap()
}

fn store() -> (tempfile::TempDir, ImageStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = ImageStore::new(dir.path().to_path_buf());
    (dir, store)
}

fn disk_image(dir: &Path) -> PathBuf {
    let path = dir.join("source.img");
    make_fat16_image(&path, MIB, 8 * MIB, 16 * MIB);
    path
}

fn opaque_blob(path: &Path, len: usize) {
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.push((state & 0xff) as u8);
    }
    fs::write(path, bytes).unwrap();
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

#[test]
fn import_copies_the_source_and_records_the_digest() {
    let (dir, store) = store();
    let source = disk_image(dir.path());
    let expected = fs::read(&source).unwrap();

    let record = store
        .import(&source, &reference("freedos:test"), ImageMedia::Disk, at(0))
        .unwrap();

    assert_eq!(record.schema_version, IMAGE_SCHEMA_VERSION);
    assert_eq!(record.name, "freedos");
    assert_eq!(record.tag, "test");
    assert_eq!(record.media, ImageMedia::Disk);
    assert_eq!(record.size, expected.len() as u64);
    assert_eq!(record.digest.len(), "sha256:".len() + 64);
    assert!(record.digest.starts_with("sha256:"));
    let digest = record.digest.strip_prefix("sha256:").unwrap();
    assert!(
        digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    let expected_digest = format!("sha256:{}", hex(&Sha256::digest(&expected)));
    assert_eq!(record.digest, expected_digest);
    assert_eq!(record.imported_at, at(0));
    assert_eq!(
        fs::read(store.disk_path(&reference("freedos:test"))).unwrap(),
        expected
    );
}

#[test]
fn import_leaves_the_source_unchanged() {
    let (dir, store) = store();
    let source = disk_image(dir.path());
    let before = fs::read(&source).unwrap();

    store
        .import(&source, &reference("freedos:test"), ImageMedia::Disk, at(0))
        .unwrap();

    assert_eq!(fs::read(&source).unwrap(), before);
}

#[test]
fn import_rejects_a_duplicate_reference() {
    let (dir, store) = store();
    let source = disk_image(dir.path());
    let image = reference("freedos:test");
    store
        .import(&source, &image, ImageMedia::Disk, at(0))
        .unwrap();

    let error = store
        .import(&source, &image, ImageMedia::Disk, at(1))
        .unwrap_err();

    assert!(matches!(error, ImageError::AlreadyExists(value) if value == image));
}

#[test]
fn import_booter_accepts_an_opaque_blob() {
    let (dir, store) = store();
    let source = dir.path().join("booter.img");
    opaque_blob(&source, (MIB as usize) - 1);

    let record = store
        .import(
            &source,
            &reference("booter:test"),
            ImageMedia::Booter,
            at(0),
        )
        .unwrap();

    assert_eq!(record.media, ImageMedia::Booter);
    assert!(record.is_booter());
    assert!(!record.expects_agent());
    assert_eq!(record.size, MIB - 1);
    assert_eq!(
        fs::read(store.disk_path(&reference("booter:test"))).unwrap(),
        fs::read(&source).unwrap()
    );
}

#[test]
fn import_rejects_a_non_bootable_disk() {
    let (dir, store) = store();
    let source = dir.path().join("blank.img");
    fs::write(&source, vec![0u8; 4096]).unwrap();

    let error = store
        .import(&source, &reference("freedos:test"), ImageMedia::Disk, at(0))
        .unwrap_err();

    assert!(matches!(error, ImageError::NotBootable(path) if path == source));
    assert!(matches!(
        store.load(&reference("freedos:test")),
        Err(ImageError::NotFound(_))
    ));
}

#[test]
fn import_rejects_a_superfloppy_disk() {
    let (dir, store) = store();
    let source = dir.path().join("superfloppy.img");
    make_fat16_image(&source, 0, 4 * MIB, 4 * MIB);

    let error = store
        .import(&source, &reference("freedos:test"), ImageMedia::Disk, at(0))
        .unwrap_err();

    assert!(matches!(error, ImageError::UnsupportedLayout(path) if path == source));
    assert!(
        !store
            .image_dir(&reference("freedos:test"))
            .join(IMAGE_RECORD_FILE)
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn imported_disk_is_read_only() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, store) = store();
    let source = disk_image(dir.path());
    let image = reference("freedos:test");
    store
        .import(&source, &image, ImageMedia::Disk, at(0))
        .unwrap();

    let mode = fs::metadata(store.disk_path(&image))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o444);
}

#[test]
fn load_rejects_an_unsupported_schema() {
    let (dir, store) = store();
    let source = disk_image(dir.path());
    let image = reference("freedos:test");
    let mut record = store
        .import(&source, &image, ImageMedia::Disk, at(0))
        .unwrap();
    record.schema_version = 2;
    fs::write(
        store.image_dir(&image).join(IMAGE_RECORD_FILE),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();

    let error = store.load(&image).unwrap_err();

    assert!(matches!(error, ImageError::Corrupt { .. }));
}

#[test]
fn load_reports_a_missing_reference() {
    let (_dir, store) = store();
    let image = reference("freedos:test");

    let error = store.load(&image).unwrap_err();

    assert!(matches!(error, ImageError::NotFound(value) if value == image));
}

#[test]
fn list_returns_images_sorted_by_name_and_tag() {
    let (dir, store) = store();
    let source = disk_image(dir.path());
    for image in ["zeta:latest", "alpha:v2", "alpha:v1"] {
        store
            .import(&source, &reference(image), ImageMedia::Disk, at(0))
            .unwrap();
    }

    let listed = store.list().unwrap();

    let labels: Vec<String> = listed
        .iter()
        .map(|record| format!("{}:{}", record.name, record.tag))
        .collect();
    assert_eq!(labels, ["alpha:v1", "alpha:v2", "zeta:latest"]);
}

#[test]
fn copy_disk_to_returns_the_same_bytes() {
    let (dir, store) = store();
    let source = disk_image(dir.path());
    let image = reference("freedos:test");
    store
        .import(&source, &image, ImageMedia::Disk, at(0))
        .unwrap();
    let destination = dir.path().join("container/disk.img");

    let outcome = store.copy_disk_to(&image, &destination).unwrap();

    assert!(matches!(
        outcome,
        CopyOutcome::Reflink | CopyOutcome::FullCopy
    ));
    assert_eq!(fs::read(&destination).unwrap(), fs::read(&source).unwrap());
}

#[test]
fn copy_disk_to_does_not_modify_the_stored_image() {
    let (dir, store) = store();
    let source = disk_image(dir.path());
    let image = reference("freedos:test");
    store
        .import(&source, &image, ImageMedia::Disk, at(0))
        .unwrap();
    let stored = store.disk_path(&image);
    let before = fs::read(&stored).unwrap();
    let destination = dir.path().join("container/disk.img");

    store.copy_disk_to(&image, &destination).unwrap();
    let mut copy = OpenOptions::new().write(true).open(&destination).unwrap();
    copy.write_all(&[0xff; 16]).unwrap();
    copy.sync_all().unwrap();

    assert_eq!(fs::read(&stored).unwrap(), before);
}

#[test]
fn image_record_round_trips_through_json() {
    let record = ImageRecord {
        schema_version: IMAGE_SCHEMA_VERSION,
        name: "freedos".to_owned(),
        tag: "test".to_owned(),
        media: ImageMedia::Disk,
        digest: format!("sha256:{}", "a".repeat(64)),
        size: 16 * MIB,
        source: "/images/freedos-test.img".to_owned(),
        agent_version: Some("1.0.0".to_owned()),
        cmd: Some("ECHO HELLO".to_owned()),
        imported_at: at(7),
    };

    let json = serde_json::to_vec(&record).unwrap();
    let parsed: ImageRecord = serde_json::from_slice(&json).unwrap();

    assert_eq!(parsed, record);
    assert_eq!(
        serde_json::to_value(record.media).unwrap(),
        serde_json::json!("disk")
    );
    assert_eq!(
        serde_json::to_value(ImageMedia::Booter).unwrap(),
        serde_json::json!("booter")
    );
}
