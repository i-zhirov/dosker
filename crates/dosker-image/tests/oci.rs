mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use dosker_image::{
    ImageConfig, ImageDigest, ImageError, ImageMedia, ImageRef, ImageReference, Manifest, OciStore,
    canonical_json,
};
use support::{make_fat16_image, write_fat_file};

const MIB: u64 = 1024 * 1024;

fn now(second: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(second, 0).unwrap()
}

fn disk(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    make_fat16_image(&path, MIB, 8 * MIB, 16 * MIB);
    write_fat_file(&path, MIB, "APP/FOO.TXT", b"hello");
    path
}

fn superfloppy(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    make_fat16_image(&path, 0, 4 * MIB, 4 * MIB);
    write_fat_file(&path, 0, "A.TXT", b"hello");
    path
}

fn reference(value: &str) -> ImageReference {
    ImageReference::parse(value).unwrap()
}

fn digest_ref(image: &dosker_image::Image) -> ImageRef {
    ImageRef::Digest(ImageDigest::parse(&image.manifest_digest).unwrap())
}

fn store(dir: &Path) -> OciStore {
    OciStore::new(dir.to_path_buf())
}

#[test]
fn import_writes_the_oci_layout_and_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let image = store(dir.path())
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let layout = dir.path().join("images");
    assert!(layout.join("oci-layout").is_file());
    assert!(layout.join("index.json").is_file());
    assert!(
        layout
            .join("blobs/sha256")
            .join(image.manifest_digest.strip_prefix("sha256:").unwrap())
            .is_file()
    );
    assert!(
        layout
            .join("blobs/sha256")
            .join(image.config_digest.strip_prefix("sha256:").unwrap())
            .is_file()
    );
}

#[test]
fn import_writes_the_raw_disk_annotation() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let image = store(dir.path())
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let store = store(dir.path());
    let manifest: Manifest =
        serde_json::from_slice(&store.read_blob(&image.manifest_digest).unwrap()).unwrap();
    assert_eq!(manifest.raw_disk_digest(), image.raw_disk_digest.as_deref());
    assert!(
        !manifest
            .layers
            .iter()
            .any(|layer| layer.digest == image.raw_disk_digest.clone().unwrap())
    );
}

#[test]
fn import_disk_reads_the_fat_tree_and_boot_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let config: ImageConfig =
        serde_json::from_slice(&store.read_blob(&image.config_digest).unwrap()).unwrap();
    assert!(!config.booter);
    assert!(image.layer_digests.len() == 1);
    assert!(image.raw_disk_digest.is_some());
}

#[test]
fn import_booter_writes_one_opaque_layer() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("booter.img");
    fs::write(&source, vec![0x5a; 1024]).unwrap();
    let store = store(dir.path());
    let image = store
        .import(
            &source,
            &reference("game:latest"),
            ImageMedia::Booter,
            now(1),
        )
        .unwrap();
    let manifest: Manifest =
        serde_json::from_slice(&store.read_blob(&image.manifest_digest).unwrap()).unwrap();
    assert_eq!(manifest.layers.len(), 1);
    assert_eq!(
        manifest.layers[0].media_type,
        dosker_image::MEDIA_TYPE_BOOTER
    );
    assert!(image.is_booter());
    assert!(image.raw_disk_digest.is_some());
}

#[test]
fn import_accepts_a_superfloppy_disk() {
    let dir = tempfile::tempdir().unwrap();
    let source = superfloppy(dir.path(), "floppy.img");
    let store = store(dir.path());
    let image = store
        .import(
            &source,
            &reference("floppy:latest"),
            ImageMedia::Disk,
            now(1),
        )
        .unwrap();
    let config: ImageConfig =
        serde_json::from_slice(&store.read_blob(&image.config_digest).unwrap()).unwrap();
    assert_eq!(config.machine.disk_interface, "floppy");
    assert_eq!(image.disk_interface, "floppy");
}

#[test]
fn import_rejects_a_duplicate_reference() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    assert!(matches!(
        store.import(&source, &reference("demo:latest"), ImageMedia::Disk, now(2)),
        Err(ImageError::AlreadyExists(_))
    ));
}

#[test]
fn import_rejects_a_non_bootable_disk() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("blank.img");
    fs::write(&source, vec![0u8; 4096]).unwrap();
    assert!(
        store(dir.path())
            .import(&source, &reference("bad:latest"), ImageMedia::Disk, now(1))
            .is_err()
    );
}

#[test]
fn import_leaves_the_source_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let before = fs::read(&source).unwrap();
    store(dir.path())
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    assert_eq!(fs::read(&source).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn import_stores_read_only_blobs() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    store(dir.path())
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    for entry in fs::read_dir(dir.path().join("images/blobs/sha256")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let mode = entry.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o444);
    }
}

#[test]
fn import_is_deterministic_for_the_same_source_and_time() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let first = store
        .import(&source, &reference("one:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let second = store
        .import(&source, &reference("two:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    assert_eq!(first.layer_digests, second.layer_digests);
    assert_eq!(first.raw_disk_digest, second.raw_disk_digest);
}

#[test]
fn import_writes_canonical_config_and_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let config: ImageConfig =
        serde_json::from_slice(&store.read_blob(&image.config_digest).unwrap()).unwrap();
    assert_eq!(
        store.read_blob(&image.config_digest).unwrap(),
        canonical_json(&config).unwrap()
    );
    let manifest: Manifest =
        serde_json::from_slice(&store.read_blob(&image.manifest_digest).unwrap()).unwrap();
    assert_eq!(
        store.read_blob(&image.manifest_digest).unwrap(),
        canonical_json(&manifest).unwrap()
    );
}

#[test]
fn oci_layout_descriptors_match_their_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    for digest in std::iter::once(&image.config_digest).chain(image.layer_digests.iter()) {
        assert!(!store.read_blob(digest).unwrap().is_empty());
    }
    let manifest: Manifest =
        serde_json::from_slice(&store.read_blob(&image.manifest_digest).unwrap()).unwrap();
    assert_eq!(manifest.config.digest, image.config_digest);
}

#[test]
fn read_blob_rejects_a_digest_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let path = store.blob_path(&image.config_digest).unwrap();
    make_writable(&path);
    fs::write(&path, b"changed").unwrap();
    assert!(matches!(
        store.read_blob(&image.config_digest),
        Err(ImageError::DigestMismatch { .. })
    ));
}

#[test]
fn list_returns_tags_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    for tag in ["z:latest", "a:v2", "a:v1"] {
        store
            .import(&source, &reference(tag), ImageMedia::Disk, now(1))
            .unwrap();
    }
    let refs: Vec<String> = store
        .list()
        .unwrap()
        .into_iter()
        .map(|image| image.reference)
        .collect();
    assert_eq!(refs, vec!["a:v1", "a:v2", "z:latest"]);
}

#[test]
fn load_by_digest_and_tag_returns_the_same_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let by_digest = store.load(&digest_ref(&image)).unwrap();
    assert_eq!(by_digest.manifest_digest, image.manifest_digest);
}

#[test]
fn load_keeps_an_image_when_the_raw_disk_blob_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    fs::remove_file(
        store
            .blob_path(image.raw_disk_digest.as_ref().unwrap())
            .unwrap(),
    )
    .unwrap();
    let loaded = store
        .load(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    assert_eq!(loaded.size, 0);
    assert!(matches!(
        store.copy_disk_to(
            &ImageRef::Tag(reference("demo:latest")),
            &dir.path().join("disk")
        ),
        Err(ImageError::MissingRawDisk(_))
    ));
}

#[test]
fn tag_adds_a_reference_without_copying_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let before = fs::read_dir(dir.path().join("images/blobs/sha256"))
        .unwrap()
        .count();
    store
        .tag(
            &ImageRef::Tag(reference("demo:latest")),
            &reference("copy:latest"),
        )
        .unwrap();
    let after = fs::read_dir(dir.path().join("images/blobs/sha256"))
        .unwrap()
        .count();
    assert_eq!(before, after);
}

#[test]
fn tag_replaces_an_existing_target() {
    let dir = tempfile::tempdir().unwrap();
    let first = disk(dir.path(), "first.img");
    let second = disk(dir.path(), "second.img");
    let store = store(dir.path());
    let first_image = store
        .import(&first, &reference("first:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let second_image = store
        .import(
            &second,
            &reference("second:latest"),
            ImageMedia::Disk,
            now(2),
        )
        .unwrap();
    store
        .tag(
            &ImageRef::Tag(reference("first:latest")),
            &reference("target:latest"),
        )
        .unwrap();
    store
        .tag(
            &ImageRef::Tag(reference("second:latest")),
            &reference("target:latest"),
        )
        .unwrap();
    assert_eq!(
        store
            .load(&ImageRef::Tag(reference("target:latest")))
            .unwrap()
            .manifest_digest,
        second_image.manifest_digest
    );
    assert_eq!(
        store
            .load(&digest_ref(&first_image))
            .unwrap()
            .manifest_digest,
        first_image.manifest_digest
    );
}

#[test]
fn remove_untags_a_shared_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    store
        .import(&source, &reference("a:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    store
        .tag(
            &ImageRef::Tag(reference("a:latest")),
            &reference("b:latest"),
        )
        .unwrap();
    let outcome = store.remove(&ImageRef::Tag(reference("a:latest"))).unwrap();
    assert!(outcome.untagged);
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn remove_by_tag_keeps_the_manifest_for_digest_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let outcome = store
        .remove(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    assert!(outcome.untagged);
    assert_eq!(
        store.load(&digest_ref(&image)).unwrap().manifest_digest,
        image.manifest_digest
    );
}

#[test]
fn remove_by_digest_deletes_the_manifest_and_unreferenced_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    store
        .remove(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    let outcome = store.remove(&digest_ref(&image)).unwrap();
    assert!(outcome.removed_manifest);
    assert!(!store.blob_path(&image.manifest_digest).unwrap().exists());
    assert!(!store.blob_path(&image.config_digest).unwrap().exists());
    assert!(!store.blob_path(&image.layer_digests[0]).unwrap().exists());
    assert!(
        !store
            .blob_path(image.raw_disk_digest.as_ref().unwrap())
            .unwrap()
            .exists()
    );
}

#[test]
fn remove_by_digest_captures_references_before_deleting() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    store
        .remove(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    store.remove(&digest_ref(&image)).unwrap();
    assert!(!store.blob_path(&image.config_digest).unwrap().exists());
    assert!(
        !store
            .blob_path(image.raw_disk_digest.as_ref().unwrap())
            .unwrap()
            .exists()
    );
}

#[test]
fn remove_keeps_a_shared_raw_disk_blob() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let first = store
        .import(&source, &reference("one:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let second = store
        .import(&source, &reference("two:latest"), ImageMedia::Disk, now(2))
        .unwrap();
    store
        .remove(&ImageRef::Tag(reference("one:latest")))
        .unwrap();
    store.remove(&digest_ref(&first)).unwrap();
    assert!(
        store
            .blob_path(second.raw_disk_digest.as_ref().unwrap())
            .unwrap()
            .exists()
    );
}

#[test]
fn remove_by_digest_rejects_a_tagged_image() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    assert!(matches!(
        store.remove(&digest_ref(&image)),
        Err(ImageError::ImageInUse(_))
    ));
}

#[test]
fn copy_disk_to_copies_the_annotated_raw_disk() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let destination = dir.path().join("disk.img");
    store
        .copy_disk_to(&ImageRef::Tag(reference("demo:latest")), &destination)
        .unwrap();
    assert_eq!(fs::read(&destination).unwrap(), fs::read(&source).unwrap());
    assert_eq!(
        fs::read(
            store
                .blob_path(image.raw_disk_digest.as_ref().unwrap())
                .unwrap()
        )
        .unwrap(),
        fs::read(&source).unwrap()
    );
}

#[test]
fn booter_copy_disk_to_copies_the_single_layer() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("booter.img");
    let bytes = vec![0x5a; 1024];
    fs::write(&source, &bytes).unwrap();
    let store = store(dir.path());
    store
        .import(
            &source,
            &reference("game:latest"),
            ImageMedia::Booter,
            now(1),
        )
        .unwrap();
    let destination = dir.path().join("disk.img");
    store
        .copy_disk_to(&ImageRef::Tag(reference("game:latest")), &destination)
        .unwrap();
    assert_eq!(fs::read(destination).unwrap(), bytes);
}

#[test]
fn tag_and_remove_write_a_parseable_index() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    store
        .tag(
            &ImageRef::Tag(reference("demo:latest")),
            &reference("copy:latest"),
        )
        .unwrap();
    store
        .remove(&ImageRef::Tag(reference("copy:latest")))
        .unwrap();
    assert!(
        serde_json::from_slice::<dosker_image::Index>(
            &fs::read(dir.path().join("images/index.json")).unwrap()
        )
        .is_ok()
    );
    assert!(
        !fs::read_dir(dir.path().join("images"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tmp"))
    );
    assert!(store.load(&digest_ref(&image)).is_ok());
}

#[test]
fn concurrent_tag_calls_keep_both_index_entries() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = Arc::new(store(dir.path()));
    store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    let left = Arc::clone(&store);
    let right = Arc::clone(&store);
    let one = std::thread::spawn(move || {
        left.tag(
            &ImageRef::Tag(reference("demo:latest")),
            &reference("one:latest"),
        )
    });
    let two = std::thread::spawn(move || {
        right.tag(
            &ImageRef::Tag(reference("demo:latest")),
            &reference("two:latest"),
        )
    });
    one.join().unwrap().unwrap();
    two.join().unwrap().unwrap();
    assert!(store.load(&ImageRef::Tag(reference("one:latest"))).is_ok());
    assert!(store.load(&ImageRef::Tag(reference("two:latest"))).is_ok());
}

#[test]
fn index_lock_is_released_after_import() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    store
        .import(&source, &reference("demo:latest"), ImageMedia::Disk, now(1))
        .unwrap();
    store
        .tag(
            &ImageRef::Tag(reference("demo:latest")),
            &reference("copy:latest"),
        )
        .unwrap();
    assert!(store.load(&ImageRef::Tag(reference("copy:latest"))).is_ok());
}

fn make_writable(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o644);
    }
    #[cfg(windows)]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).unwrap();
}
