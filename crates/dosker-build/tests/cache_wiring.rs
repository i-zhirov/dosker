use std::fs;

use chrono::{TimeZone, Utc};
use dosker_build::{BuildCache, CacheLookup};
use dosker_image::cache::CACHE_SCHEMA_VERSION;
use dosker_image::{
    Attributes, CacheRecord, CacheStore, DosPath, Layer, LayerDelta, OciStore, TreeFile,
    fixed_timestamp,
};

fn open(root: &std::path::Path) -> (OciStore, CacheStore) {
    (
        OciStore::new(root.to_path_buf()),
        CacheStore::new(root.to_path_buf()),
    )
}

fn layer(parent: Option<&str>) -> Layer {
    let mut delta = LayerDelta::new();
    delta.add_file(TreeFile {
        path: DosPath::parse(r"C:\APP\HELLO.TXT").expect("the file path parses"),
        attributes: Attributes {
            archive: true,
            ..Attributes::default()
        },
        timestamp: fixed_timestamp(),
        bytes: b"hello".to_vec(),
    });
    Layer::from_delta(
        &delta,
        "COPY HELLO.TXT C:\\APP",
        parent.map(str::to_owned),
        None,
    )
    .expect("the layer builds")
}

fn record(key: &str, layer_digest: &str, instruction: &str) -> CacheRecord {
    CacheRecord {
        schema_version: CACHE_SCHEMA_VERSION,
        key: key.to_owned(),
        layer_digest: layer_digest.to_owned(),
        instruction: instruction.to_owned(),
        created_at: Utc.timestamp_opt(1, 0).unwrap(),
    }
}

fn stored_layer(store: &OciStore, layer: &Layer) -> dosker_image::Descriptor {
    store.put_layer(layer).expect("the layer stores")
}

#[test]
fn a_matching_cache_record_returns_the_layer() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = open(dir.path());
    let build = BuildCache::new(&store, &cache, false);
    let layer = layer(None);
    let descriptor = stored_layer(&store, &layer);
    cache
        .store(&record("key", &descriptor.digest, "COPY A B"))
        .unwrap();

    match build
        .lookup("key", "COPY A B", None, true)
        .expect("the lookup succeeds")
    {
        CacheLookup::HitLayer {
            layer: hit_layer,
            descriptor: hit_descriptor,
        } => {
            assert_eq!(hit_layer, layer);
            assert_eq!(hit_descriptor, descriptor);
        }
        other => panic!("expected a layer hit, got {other:?}"),
    }
}

#[test]
fn a_missing_layer_blob_is_a_miss() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = open(dir.path());
    let build = BuildCache::new(&store, &cache, false);
    let descriptor = stored_layer(&store, &layer(None));
    cache
        .store(&record("key", &descriptor.digest, "COPY A B"))
        .unwrap();
    fs::remove_file(store.blob_path(&descriptor.digest).unwrap()).unwrap();

    assert!(matches!(
        build.lookup("key", "COPY A B", None, true).unwrap(),
        CacheLookup::Miss
    ));
    assert!(cache.load("key").unwrap().is_none());
}

#[test]
fn a_layer_parent_mismatch_is_a_miss() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = open(dir.path());
    let build = BuildCache::new(&store, &cache, false);
    let parent = stored_layer(&store, &layer(None));
    let child = layer(Some(&parent.digest));
    let descriptor = stored_layer(&store, &child);
    cache
        .store(&record("key", &descriptor.digest, "COPY A B"))
        .unwrap();

    assert!(matches!(
        build
            .lookup("key", "COPY A B", Some("sha256:unexpected"), true)
            .unwrap(),
        CacheLookup::Miss
    ));
    assert!(cache.load("key").unwrap().is_none());
}

#[test]
fn a_corrupt_record_is_removed_and_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = open(dir.path());
    let build = BuildCache::new(&store, &cache, false);
    let path = cache.entry_path("key");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"not json").unwrap();

    assert!(matches!(
        build.lookup("key", "COPY A B", None, true).unwrap(),
        CacheLookup::Miss
    ));
    assert!(!path.exists());

    let descriptor = stored_layer(&store, &layer(None));
    build
        .store(
            "key",
            &descriptor.digest,
            "COPY A B",
            Utc.timestamp_opt(2, 0).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        build.lookup("key", "COPY A B", None, true).unwrap(),
        CacheLookup::HitLayer { .. }
    ));
}

#[test]
fn a_different_instruction_is_a_miss() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = open(dir.path());
    let build = BuildCache::new(&store, &cache, false);
    let descriptor = stored_layer(&store, &layer(None));
    cache
        .store(&record("key", &descriptor.digest, "COPY A B"))
        .unwrap();

    assert!(matches!(
        build.lookup("key", "COPY A C", None, true).unwrap(),
        CacheLookup::Miss
    ));
    assert!(cache.load("key").unwrap().is_none());
}

#[test]
fn no_cache_bypasses_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = open(dir.path());
    let build = BuildCache::new(&store, &cache, true);
    let descriptor = stored_layer(&store, &layer(None));
    cache
        .store(&record("key", &descriptor.digest, "COPY A B"))
        .unwrap();

    assert!(matches!(
        build.lookup("key", "COPY A B", None, true).unwrap(),
        CacheLookup::Miss
    ));
    assert!(cache.load("key").unwrap().is_some());
}

#[test]
fn config_only_records_use_the_head_layer() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = open(dir.path());
    let build = BuildCache::new(&store, &cache, false);
    let head = stored_layer(&store, &layer(None));
    cache
        .store(&record("config", &head.digest, "ENV A=B"))
        .unwrap();

    assert!(matches!(
        build.lookup("config", "ENV A=B", None, false).unwrap(),
        CacheLookup::HitConfig
    ));

    fs::remove_file(store.blob_path(&head.digest).unwrap()).unwrap();
    assert!(matches!(
        build.lookup("config", "ENV A=B", None, false).unwrap(),
        CacheLookup::Miss
    ));
    assert!(cache.load("config").unwrap().is_none());

    cache.store(&record("config-empty", "", "ENV C=D")).unwrap();
    assert!(matches!(
        build
            .lookup("config-empty", "ENV C=D", None, false)
            .unwrap(),
        CacheLookup::HitConfig
    ));
}

#[test]
fn storing_a_record_writes_a_valid_entry() {
    let dir = tempfile::tempdir().unwrap();
    let (store, cache) = open(dir.path());
    let build = BuildCache::new(&store, &cache, false);
    let descriptor = stored_layer(&store, &layer(None));
    let now = Utc.timestamp_opt(3, 0).unwrap();

    build
        .store("key", &descriptor.digest, "ENV A=B", now)
        .unwrap();

    let stored = cache.load("key").unwrap().expect("the record loads");
    assert_eq!(stored.schema_version, CACHE_SCHEMA_VERSION);
    assert_eq!(stored.key, "key");
    assert_eq!(stored.layer_digest, descriptor.digest);
    assert_eq!(stored.instruction, "ENV A=B");
    assert_eq!(stored.created_at, now);
}
