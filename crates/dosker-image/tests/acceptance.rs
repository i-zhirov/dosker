use std::fs;

use chrono::{TimeZone, Utc};
use dosker_image::ImageError;
use dosker_image::acceptance::{ACCEPTANCE_SCHEMA_VERSION, AcceptanceStore, AcceptedImage};

fn digest(seed: char) -> String {
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn record() -> AcceptedImage {
    AcceptedImage {
        schema_version: ACCEPTANCE_SCHEMA_VERSION,
        digest: digest('a'),
        config_digest: digest('b'),
        image: "i-zhirov/old-game".to_owned(),
        license_summary: "abandonware".to_owned(),
        accepted_at: Utc.timestamp_opt(1_763_900_000, 0).unwrap(),
    }
}

#[test]
fn an_acceptance_round_trips() {
    let directory = tempfile::tempdir().unwrap();
    let store = AcceptanceStore::new(directory.path().to_path_buf());

    assert!(!store.is_accepted(&digest('a')).unwrap());
    store.record(&record()).unwrap();

    assert!(store.is_accepted(&digest('a')).unwrap());
    let records = store.list().unwrap();
    assert_eq!(records, vec![record()]);
}

#[test]
fn a_config_digest_is_recorded() {
    let directory = tempfile::tempdir().unwrap();
    let store = AcceptanceStore::new(directory.path().to_path_buf());
    store.record(&record()).unwrap();

    let path = store.path(&digest('a')).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

    assert_eq!(value["config_digest"], digest('b'));
    assert_eq!(value["license_summary"], "abandonware");
}

#[test]
fn a_corrupt_record_is_reported() {
    let directory = tempfile::tempdir().unwrap();
    let store = AcceptanceStore::new(directory.path().to_path_buf());
    let path = store.path(&digest('a')).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"not json").unwrap();

    assert!(matches!(
        store.is_accepted(&digest('a')),
        Err(ImageError::Corrupt { .. })
    ));
}

#[test]
fn an_unsafe_digest_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let store = AcceptanceStore::new(directory.path().to_path_buf());

    assert!(matches!(
        store.path("../escape"),
        Err(ImageError::InvalidReference(_))
    ));
    assert!(matches!(
        store.is_accepted("sha256:not-hex"),
        Err(ImageError::InvalidReference(_))
    ));
}
