use std::fs;
use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use dosker_core::ContainerId;
use dosker_image::{DEFAULT_VOLUME_SIZE, FatType, VolumeLayout};
use dosker_runtime::{VolumeError, VolumeHolder, VolumeStore};

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn store() -> (tempfile::TempDir, VolumeStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = VolumeStore::new(dir.path().to_path_buf());
    (dir, store)
}

fn holder(id: &str, name: &str) -> VolumeHolder {
    VolumeHolder {
        container_id: ContainerId::parse(id).unwrap(),
        container_name: name.to_owned(),
        supervisor_pid: std::process::id(),
    }
}

#[test]
fn create_writes_an_image_and_a_meta_file() {
    let (dir, store) = store();
    let volume = store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap();

    assert_eq!(volume.meta.schema_version, 1);
    assert_eq!(volume.meta.name, "data");
    assert_eq!(volume.meta.size, DEFAULT_VOLUME_SIZE);
    assert_eq!(volume.meta.layout, VolumeLayout::MbrPartitioned);
    assert_eq!(volume.meta.fat_type, FatType::Fat16);
    assert_eq!(volume.meta.created_at, at());
    assert!(!volume.meta.dirty);

    let volume_dir = dir.path().join("volumes").join("data");
    let image = volume_dir.join("volume.img");
    let meta = volume_dir.join("meta.json");
    assert!(image.is_file());
    assert!(meta.is_file());
    assert_eq!(fs::metadata(&image).unwrap().len(), DEFAULT_VOLUME_SIZE);

    let loaded = store.load("data").unwrap();
    assert_eq!(loaded.meta, volume.meta);
    assert!(loaded.attached.is_none());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(dir.path().join("volumes"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }
}

#[test]
fn create_rejects_a_duplicate_name_without_touching_the_volume() {
    let (dir, store) = store();
    store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap();

    let volume_dir = dir.path().join("volumes").join("data");
    let image_before = fs::read(volume_dir.join("volume.img")).unwrap();
    let meta_before = fs::read(volume_dir.join("meta.json")).unwrap();

    let error = store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap_err();
    assert!(matches!(error, VolumeError::AlreadyExists(ref name) if name == "data"));

    assert_eq!(
        fs::read(volume_dir.join("volume.img")).unwrap(),
        image_before
    );
    assert_eq!(fs::read(volume_dir.join("meta.json")).unwrap(), meta_before);
}

#[test]
fn load_reports_a_missing_volume() {
    let (_dir, store) = store();

    let error = store.load("data").unwrap_err();
    assert!(matches!(error, VolumeError::NotFound(ref name) if name == "data"));
}

#[test]
fn list_of_a_fresh_root_is_empty() {
    let (dir, store) = store();

    assert!(store.list().unwrap().is_empty());
    assert!(!dir.path().join("volumes").exists());
}

#[test]
fn list_returns_volumes_sorted_by_name() {
    let (_dir, store) = store();
    store.create("beta", DEFAULT_VOLUME_SIZE, at()).unwrap();
    store.create("alpha", DEFAULT_VOLUME_SIZE, at()).unwrap();

    let names: Vec<String> = store
        .list()
        .unwrap()
        .into_iter()
        .map(|volume| volume.meta.name)
        .collect();
    assert_eq!(names, ["alpha", "beta"]);
}

#[test]
fn a_second_attach_fails_and_names_the_holder() {
    let (_dir, store) = store();
    store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap();
    let _lock = store
        .acquire("data", holder("000000000001", "other"))
        .unwrap();

    let error = match store.acquire("data", holder("000000000002", "mine")) {
        Ok(_) => panic!("the second attach should fail"),
        Err(error) => error,
    };
    let message = error.to_string();
    match error {
        VolumeError::InUse { name, holder } => {
            assert_eq!(name, "data");
            assert_eq!(holder.container_name, "other");
            assert!(message.contains("other"));
        }
        other => panic!("expected InUse, got {other}"),
    }
}

#[test]
fn dropping_the_lock_releases_the_volume() {
    let (_dir, store) = store();
    store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap();

    let lock = store
        .acquire("data", holder("000000000001", "other"))
        .unwrap();
    drop(lock);

    let lock = store
        .acquire("data", holder("000000000002", "mine"))
        .unwrap();
    drop(lock);

    assert!(store.load("data").unwrap().attached.is_none());
}

#[test]
fn remove_rejects_a_volume_that_is_attached() {
    let (dir, store) = store();
    store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap();
    let _lock = store
        .acquire("data", holder("000000000001", "other"))
        .unwrap();

    let error = store.remove("data").unwrap_err();
    assert!(matches!(error, VolumeError::InUse { ref name, .. } if name == "data"));
    assert!(dir.path().join("volumes/data/volume.img").is_file());
}

#[test]
fn remove_deletes_the_volume_directory() {
    let (dir, store) = store();
    store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap();
    let lock = store
        .acquire("data", holder("000000000001", "other"))
        .unwrap();
    drop(lock);

    let volume_dir = dir.path().join("volumes").join("data");
    assert!(volume_dir.join("volume.img").is_file());
    assert!(volume_dir.join("meta.json").is_file());
    assert!(volume_dir.join(".lock").is_file());

    store.remove("data").unwrap();

    assert!(!volume_dir.join("volume.img").exists());
    assert!(!volume_dir.join("meta.json").exists());
    assert!(!volume_dir.join(".lock").exists());
    assert!(!volume_dir.exists());
}

#[test]
fn a_stale_holder_without_a_lock_is_not_reported() {
    let (dir, store) = store();
    store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap();

    let mut bytes = serde_json::to_vec(&holder("000000000001", "stale")).unwrap();
    bytes.push(b'\n');
    fs::write(dir.path().join("volumes/data/.lock"), bytes).unwrap();

    assert!(store.load("data").unwrap().attached.is_none());

    let lock = store
        .acquire("data", holder("000000000002", "mine"))
        .unwrap();
    drop(lock);
}

#[test]
fn a_meta_file_with_another_schema_version_is_rejected() {
    let (dir, store) = store();
    store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap();

    let meta_path = dir.path().join("volumes/data/meta.json");
    let mut meta: serde_json::Value =
        serde_json::from_slice(&fs::read(&meta_path).unwrap()).unwrap();
    meta["schema_version"] = serde_json::json!(2);
    fs::write(&meta_path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();

    let error = store.load("data").unwrap_err();
    assert!(matches!(error, VolumeError::InvalidMeta { .. }));
}

#[test]
fn set_dirty_updates_the_meta_file() {
    let (dir, store) = store();
    store.create("data", DEFAULT_VOLUME_SIZE, at()).unwrap();

    store.set_dirty("data", true).unwrap();
    assert!(store.load("data").unwrap().meta.dirty);

    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("volumes/data/meta.json")).unwrap())
            .unwrap();
    assert_eq!(meta["dirty"], serde_json::json!(true));

    store.set_dirty("data", false).unwrap();
    assert!(!store.load("data").unwrap().meta.dirty);
}

#[test]
fn load_remove_acquire_and_set_dirty_reject_path_like_names() {
    let (dir, store) = store();

    let parent_target = dir.path().join("..").join("x");
    let absolute_target = Path::new("/tmp/x");
    let parent_before = parent_target.exists();
    let absolute_before = absolute_target.exists();

    for name in ["../x", "/tmp/x", "a/b"] {
        assert!(matches!(
            store.load(name).unwrap_err(),
            VolumeError::InvalidName(_)
        ));
        assert!(matches!(
            store.remove(name).unwrap_err(),
            VolumeError::InvalidName(_)
        ));
        let error = match store.acquire(name, holder("000000000001", "other")) {
            Ok(_) => panic!("{name} should be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, VolumeError::InvalidName(_)));
        assert!(matches!(
            store.set_dirty(name, true).unwrap_err(),
            VolumeError::InvalidName(_)
        ));
    }

    assert_eq!(parent_target.exists(), parent_before);
    assert_eq!(absolute_target.exists(), absolute_before);
    assert!(!dir.path().join("volumes").exists());
    assert!(!dir.path().join("x").exists());
}
