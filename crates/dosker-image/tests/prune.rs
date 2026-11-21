mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use dosker_image::{
    Attributes, CacheRecord, CacheStore, DosPath, ImageReference, Layer, LayerDelta, OciStore,
    TreeFile, sha256_digest,
};
use support::{sample_boot, sample_config};

fn store(dir: &Path) -> OciStore {
    OciStore::new(dir.to_path_buf())
}

fn cache(dir: &Path) -> CacheStore {
    CacheStore::new(dir.to_path_buf())
}

fn file(path: &str, bytes: &[u8]) -> TreeFile {
    TreeFile {
        path: DosPath::parse(path).unwrap(),
        attributes: Attributes {
            archive: true,
            ..Attributes::default()
        },
        timestamp: dosker_image::fixed_timestamp(),
        bytes: bytes.to_vec(),
    }
}

fn layer(content: &[u8]) -> Layer {
    Layer::from_delta(
        &LayerDelta {
            whiteouts: Vec::new(),
            files: vec![file("A.TXT", content)],
            dirs: Vec::new(),
        },
        "test",
        None,
        None,
    )
    .unwrap()
}

fn commit(store: &OciStore, tag: Option<&str>, content: &[u8]) -> String {
    let descriptor = store.put_layer(&layer(content)).unwrap();
    let reference = tag.map(|value| ImageReference::parse(value).unwrap());
    store
        .commit_image(
            reference.as_ref(),
            &sample_config(sample_boot()),
            vec![descriptor],
            BTreeMap::new(),
        )
        .unwrap()
        .manifest_digest
}

fn record(key: &str, layer_digest: &str) -> CacheRecord {
    CacheRecord {
        schema_version: dosker_image::cache::CACHE_SCHEMA_VERSION,
        key: key.to_owned(),
        layer_digest: layer_digest.to_owned(),
        instruction: "COPY A.TXT A.TXT".to_owned(),
        created_at: Utc.timestamp_opt(1, 0).unwrap(),
    }
}

fn missing_digest() -> String {
    sha256_digest(b"missing layer")
}

fn blob_path(store: &OciStore, digest: &str) -> PathBuf {
    store.blob_path(digest).unwrap()
}

#[test]
fn untagged_manifests_lists_an_untagged_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let digest = commit(&store, None, b"one");
    assert_eq!(
        store.untagged_manifest_digests(&BTreeSet::new()).unwrap(),
        vec![digest]
    );
}

#[test]
fn untagged_manifests_skips_a_tagged_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let digest = commit(&store, Some("demo:latest"), b"one");
    let untagged = store.untagged_manifest_digests(&BTreeSet::new()).unwrap();
    assert!(!untagged.contains(&digest));
}

#[test]
fn untagged_manifests_skips_an_in_use_digest() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let digest = commit(&store, None, b"one");
    let in_use = BTreeSet::from([digest]);
    assert!(store.untagged_manifest_digests(&in_use).unwrap().is_empty());
}

#[test]
fn untagged_manifests_ignores_a_layer_blob() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    store.put_layer(&layer(b"one")).unwrap();
    assert!(
        store
            .untagged_manifest_digests(&BTreeSet::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn untagged_manifests_is_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let first = commit(&store, None, b"one");
    let second = commit(&store, None, b"two");
    let third = commit(&store, None, b"three");
    let mut expected = vec![first, second, third];
    expected.sort();
    assert_eq!(
        store.untagged_manifest_digests(&BTreeSet::new()).unwrap(),
        expected
    );
}

#[test]
fn untagged_manifests_returns_an_empty_list_for_a_missing_blobs_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    assert!(
        store
            .untagged_manifest_digests(&BTreeSet::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cache_prune_removes_a_record_with_a_missing_layer() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let cache = cache(dir.path());
    cache.store(&record("a", &missing_digest())).unwrap();
    assert_eq!(cache.prune(&store).unwrap(), 1);
    assert!(cache.load("a").unwrap().is_none());
}

#[test]
fn cache_prune_keeps_a_record_with_a_stored_layer() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let blob = store.put_blob(b"layer").unwrap();
    let cache = cache(dir.path());
    cache.store(&record("a", &blob.digest)).unwrap();
    assert_eq!(cache.prune(&store).unwrap(), 0);
    assert!(cache.load("a").unwrap().is_some());
}

#[test]
fn cache_prune_removes_a_corrupt_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let cache = cache(dir.path());
    let path = cache.entry_path("a");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{not json").unwrap();
    assert_eq!(cache.prune(&store).unwrap(), 1);
    assert!(!path.exists());
}

#[test]
fn cache_prune_removes_a_record_with_an_unsupported_schema() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let cache = cache(dir.path());
    let mut saved = record("a", "");
    saved.schema_version = 2;
    cache.store(&saved).unwrap();
    assert_eq!(cache.prune(&store).unwrap(), 1);
    assert!(!cache.entry_path("a").exists());
}

#[test]
fn cache_prune_removes_a_record_with_a_corrupt_layer() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let blob = store.put_blob(b"layer").unwrap();
    let path = blob_path(&store, &blob.digest);
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"corrupt").unwrap();
    let cache = cache(dir.path());
    cache.store(&record("a", &blob.digest)).unwrap();
    assert_eq!(cache.prune(&store).unwrap(), 1);
    assert!(!cache.entry_path("a").exists());
}

#[test]
fn cache_prune_keeps_a_config_only_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let cache = cache(dir.path());
    cache.store(&record("a", "")).unwrap();
    assert_eq!(cache.prune(&store).unwrap(), 0);
    assert!(cache.load("a").unwrap().is_some());
}

#[test]
fn cache_prune_returns_zero_for_a_missing_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    assert_eq!(cache(dir.path()).prune(&store).unwrap(), 0);
}
