use chrono::{TimeZone, Utc};
use dosker_image::{CacheKeyInput, CacheRecord, CacheStore, cache_key};

fn input<'a>(
    parent: &'a str,
    config: &'a str,
    instruction: &'a str,
    hashes: &'a [String],
) -> CacheKeyInput<'a> {
    CacheKeyInput {
        parent_tree_digest: parent,
        config_digest: config,
        instruction,
        input_hashes: hashes,
    }
}

fn record(key: &str) -> CacheRecord {
    CacheRecord {
        schema_version: 1,
        key: key.to_owned(),
        layer_digest: "sha256:layer".to_owned(),
        instruction: "COPY A".to_owned(),
        created_at: Utc.timestamp_opt(1, 0).unwrap(),
    }
}

#[test]
fn cache_key_matches_the_pinned_concatenation() {
    let hashes = vec!["a".to_owned(), "b".to_owned()];
    let key = cache_key(&input("parent", "config", "RUN X", &hashes));
    let expected = dosker_image::sha256_digest(b"parent\nconfig\nRUN X\na\nb");
    assert_eq!(format!("sha256:{key}"), expected);
}

#[test]
fn cache_key_changes_when_the_parent_tree_digest_changes() {
    let hashes = Vec::new();
    assert_ne!(
        cache_key(&input("a", "c", "i", &hashes)),
        cache_key(&input("b", "c", "i", &hashes))
    );
}

#[test]
fn cache_key_changes_when_the_config_digest_changes() {
    let hashes = Vec::new();
    assert_ne!(
        cache_key(&input("p", "a", "i", &hashes)),
        cache_key(&input("p", "b", "i", &hashes))
    );
}

#[test]
fn cache_key_changes_when_the_instruction_changes() {
    let hashes = Vec::new();
    assert_ne!(
        cache_key(&input("p", "c", "one", &hashes)),
        cache_key(&input("p", "c", "two", &hashes))
    );
}

#[test]
fn cache_key_changes_when_an_input_hash_changes() {
    assert_ne!(
        cache_key(&input("p", "c", "i", &["a".into()])),
        cache_key(&input("p", "c", "i", &["b".into()]))
    );
}

#[test]
fn cache_key_is_stable_for_the_same_input_hash_set() {
    assert_eq!(
        cache_key(&input("p", "c", "i", &["a".into(), "b".into()])),
        cache_key(&input("p", "c", "i", &["b".into(), "a".into()]))
    );
}

#[test]
fn cache_key_is_bare_lowercase_hex() {
    let key = cache_key(&input("p", "c", "i", &[]));
    assert_eq!(key.len(), 64);
    assert!(
        key.bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}

#[test]
fn cache_record_round_trips_through_json() {
    let value = record("a");
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(serde_json::from_str::<CacheRecord>(&json).unwrap(), value);
}

#[test]
fn cache_store_round_trips_a_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = CacheStore::new(dir.path().to_path_buf());
    let value = record("a");
    store.store(&value).unwrap();
    assert_eq!(store.load("a").unwrap(), Some(value));
    store.remove("a").unwrap();
    assert_eq!(store.load("a").unwrap(), None);
}

#[test]
fn cache_store_creates_the_cache_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = CacheStore::new(dir.path().to_path_buf());
    store.store(&record("a")).unwrap();
    assert!(dir.path().join("cache").is_dir());
}

#[test]
fn cache_store_missing_directory_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = CacheStore::new(dir.path().to_path_buf());
    assert_eq!(store.load("a").unwrap(), None);
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn cache_store_rejects_a_corrupt_entry() {
    let dir = tempfile::tempdir().unwrap();
    let store = CacheStore::new(dir.path().to_path_buf());
    store.store(&record("a")).unwrap();
    let path = store.entry_path("a");
    std::fs::write(path, b"not json").unwrap();
    assert!(store.load("a").is_err());
}
