mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use dosker_image::inject::NEW_AUTOEXEC_BYTES;
use dosker_image::{
    ANNOTATION_RAW_DISK_DIGEST, Attributes, BootMetadata, Descriptor, DosPath, ImageConfig,
    ImageDigest, ImageError, ImageMedia, ImageRef, ImageReference, Layer, LayerDelta,
    MEDIA_TYPE_BOOTER, MEDIA_TYPE_LAYER, Manifest, OciStore, TreeDir, TreeFile, canonical_json,
    fixed_timestamp, read_tree,
};
use support::{make_fat16_image, sample_boot, sample_config, write_fat_file};

const MIB: u64 = 1024 * 1024;

fn agent() -> Vec<u8> {
    b"MZ synthetic agent".to_vec()
}

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

fn file(path: &str, bytes: &[u8]) -> TreeFile {
    TreeFile {
        path: DosPath::parse(path).unwrap(),
        attributes: Attributes {
            archive: true,
            ..Attributes::default()
        },
        timestamp: fixed_timestamp(),
        bytes: bytes.to_vec(),
    }
}

fn tree_dir(path: &str) -> TreeDir {
    TreeDir {
        path: DosPath::parse(path).unwrap(),
        attributes: Attributes::default(),
        timestamp: fixed_timestamp(),
    }
}

fn layer(files: Vec<TreeFile>, dirs: Vec<TreeDir>, parent: Option<String>) -> Layer {
    Layer::from_delta(
        &LayerDelta {
            whiteouts: Vec::new(),
            files,
            dirs,
        },
        "test",
        parent,
        None,
    )
    .unwrap()
}

fn booter_descriptor(store: &OciStore, bytes: &[u8]) -> Descriptor {
    let blob = store.put_blob(bytes).unwrap();
    Descriptor {
        media_type: MEDIA_TYPE_BOOTER.to_owned(),
        digest: blob.digest,
        size: blob.size,
        annotations: None,
    }
}

#[test]
fn import_writes_the_oci_layout_and_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let image = store(dir.path())
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();
    let store = store(dir.path());
    let manifest: Manifest =
        serde_json::from_slice(&store.read_blob(&image.manifest_digest).unwrap()).unwrap();
    assert_eq!(manifest.raw_disk_digest(), image.raw_disk_digest.as_deref());
    let source_digest = dosker_image::sha256_digest(&fs::read(&source).unwrap());
    assert_eq!(
        manifest.source_disk_digest(),
        Some(source_digest.as_str()),
        "the source annotation names the original file"
    );
    let raw_digest = image.raw_disk_digest.clone().unwrap();
    assert!(
        manifest
            .layers
            .iter()
            .all(|layer| layer.digest != raw_digest && layer.digest != source_digest),
        "neither annotation is a layer digest"
    );
}

#[test]
fn import_disk_reads_the_fat_tree_and_boot_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let store = store(dir.path());
    let image = store
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();
    let config: ImageConfig =
        serde_json::from_slice(&store.read_blob(&image.config_digest).unwrap()).unwrap();
    assert!(!config.booter);
    assert_eq!(
        image.layer_digests.len(),
        2,
        "the source and injection layers"
    );
    assert!(image.raw_disk_digest.is_some());
}

#[test]
fn import_rejects_a_disk_without_an_agent() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    assert!(matches!(
        store(dir.path()).import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            None,
            now(1),
        ),
        Err(ImageError::AgentMissing)
    ));
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
            None,
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
            Some(agent().as_slice()),
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();
    assert!(matches!(
        store.import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(2),
        ),
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
            .import(
                &source,
                &reference("bad:latest"),
                ImageMedia::Disk,
                Some(agent().as_slice()),
                now(1),
            )
            .is_err()
    );
}

#[test]
fn import_leaves_the_source_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let source = disk(dir.path(), "source.img");
    let before = fs::read(&source).unwrap();
    store(dir.path())
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("one:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();
    let second = store
        .import(
            &source,
            &reference("two:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
            .import(
                &source,
                &reference(tag),
                ImageMedia::Disk,
                Some(agent().as_slice()),
                now(1),
            )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &first,
            &reference("first:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();
    let second_image = store
        .import(
            &second,
            &reference("second:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
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
        .import(
            &source,
            &reference("a:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("one:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();
    let second = store
        .import(
            &source,
            &reference("two:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(2),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();
    let destination = dir.path().join("disk.img");
    store
        .copy_disk_to(&ImageRef::Tag(reference("demo:latest")), &destination)
        .unwrap();
    let raw = fs::read(
        store
            .blob_path(image.raw_disk_digest.as_deref().unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(fs::read(&destination).unwrap(), raw);
    assert_ne!(
        raw,
        fs::read(&source).unwrap(),
        "the stored disk carries the injected agent"
    );

    let tree = read_tree(&destination).unwrap();
    let agent_file = tree
        .files()
        .find(|file| file.path.as_str() == "DOSKER/DOSKER.EXE")
        .expect("the injected agent is in the copied disk");
    assert_eq!(agent_file.bytes, agent());
    let autoexec = tree
        .files()
        .find(|file| file.path.as_str() == "AUTOEXEC.BAT")
        .expect("the generated AUTOEXEC.BAT is in the copied disk");
    assert_eq!(autoexec.bytes, NEW_AUTOEXEC_BYTES);
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
            None,
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
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
        .import(
            &source,
            &reference("demo:latest"),
            ImageMedia::Disk,
            Some(agent().as_slice()),
            now(1),
        )
        .unwrap();
    store
        .tag(
            &ImageRef::Tag(reference("demo:latest")),
            &reference("copy:latest"),
        )
        .unwrap();
    assert!(store.load(&ImageRef::Tag(reference("copy:latest"))).is_ok());
}

#[test]
fn put_layer_writes_a_canonical_layer_blob() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let value = layer(
        vec![file("APP/FOO.TXT", b"hello")],
        vec![tree_dir("APP")],
        None,
    );
    let descriptor = store.put_layer(&value).unwrap();
    assert_eq!(descriptor.media_type, MEDIA_TYPE_LAYER);
    assert_eq!(descriptor.digest, value.digest().unwrap());
    assert!(store.blob_path(&descriptor.digest).unwrap().is_file());
}

#[test]
fn load_layer_round_trips_a_layer() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let original = layer(
        vec![file("APP/FOO.TXT", b"hello")],
        vec![tree_dir("APP")],
        None,
    );
    let descriptor = store.put_layer(&original).unwrap();
    let loaded = store.load_layer(&descriptor.digest).unwrap();
    assert_eq!(loaded.meta, original.meta);
    assert_eq!(loaded.to_delta(), original.to_delta());
}

#[test]
fn layer_descriptor_reports_the_blob_size() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let descriptor = store
        .put_layer(&layer(vec![file("A.TXT", b"hello")], Vec::new(), None))
        .unwrap();
    let blob_size = fs::metadata(store.blob_path(&descriptor.digest).unwrap())
        .unwrap()
        .len();
    let reported = store.layer_descriptor(&descriptor.digest).unwrap();
    assert_eq!(reported.media_type, MEDIA_TYPE_LAYER);
    assert_eq!(reported.digest, descriptor.digest);
    assert_eq!(reported.size, blob_size);
}

#[test]
fn load_manifest_resolves_a_tag_and_a_digest() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let descriptor = store
        .put_layer(&layer(vec![file("A.TXT", b"a")], Vec::new(), None))
        .unwrap();
    let committed = store
        .commit_image(
            Some(&reference("demo:latest")),
            &sample_config(sample_boot()),
            vec![descriptor],
            BTreeMap::new(),
        )
        .unwrap();
    let by_tag = store
        .load_manifest(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    let by_digest = store
        .load_manifest(&ImageRef::Digest(
            ImageDigest::parse(&committed.manifest_digest).unwrap(),
        ))
        .unwrap();
    assert_eq!(by_tag, by_digest);
}

#[test]
fn load_config_returns_the_stored_config() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let descriptor = store
        .put_layer(&layer(vec![file("A.TXT", b"a")], Vec::new(), None))
        .unwrap();
    let mut config = sample_config(sample_boot());
    config.env.insert("MODE".to_owned(), "test".to_owned());
    config.cmd = Some("APP.EXE".to_owned());
    store
        .commit_image(
            Some(&reference("demo:latest")),
            &config,
            vec![descriptor],
            BTreeMap::new(),
        )
        .unwrap();
    let loaded = store
        .load_config(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    assert_eq!(loaded, config);
}

#[test]
fn load_tree_applies_layers_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let first = layer(
        vec![file("APP/FOO.TXT", b"one")],
        vec![tree_dir("APP")],
        None,
    );
    let first_descriptor = store.put_layer(&first).unwrap();
    let second = layer(
        vec![file("APP/FOO.TXT", b"two")],
        Vec::new(),
        Some(first_descriptor.digest.clone()),
    );
    let second_descriptor = store.put_layer(&second).unwrap();
    store
        .commit_image(
            Some(&reference("demo:latest")),
            &sample_config(sample_boot()),
            vec![first_descriptor, second_descriptor],
            BTreeMap::new(),
        )
        .unwrap();
    let tree = store
        .load_tree(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    let file = tree
        .files()
        .find(|file| file.path.as_str() == "APP/FOO.TXT")
        .unwrap();
    assert_eq!(file.bytes, b"two");
}

#[test]
fn load_tree_rejects_a_broken_parent_link() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let first = layer(vec![file("A.TXT", b"a")], Vec::new(), None);
    let first_descriptor = store.put_layer(&first).unwrap();
    let second = layer(
        vec![file("B.TXT", b"b")],
        Vec::new(),
        Some("sha256:bad".to_owned()),
    );
    let second_descriptor = store.put_layer(&second).unwrap();
    store
        .commit_image(
            Some(&reference("demo:latest")),
            &sample_config(sample_boot()),
            vec![first_descriptor, second_descriptor],
            BTreeMap::new(),
        )
        .unwrap();
    assert!(matches!(
        store.load_tree(&ImageRef::Tag(reference("demo:latest"))),
        Err(ImageError::LayerCorrupt { .. })
    ));
}

#[test]
fn load_tree_rejects_a_booter_image() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let descriptor = booter_descriptor(&store, &[0x5a; 1024]);
    store
        .commit_image(
            Some(&reference("game:latest")),
            &sample_config(BootMetadata::opaque(1024)),
            vec![descriptor],
            BTreeMap::new(),
        )
        .unwrap();
    assert!(matches!(
        store.load_tree(&ImageRef::Tag(reference("game:latest"))),
        Err(ImageError::NoTreeImage(_))
    ));
}

fn boot_with_label(label: &str) -> BootMetadata {
    let mut boot = sample_boot();
    boot.volume_label = Some(label.to_owned());
    boot
}

#[test]
fn load_effective_boot_uses_the_last_layer_override() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let boot_a = boot_with_label("BOOT-A");
    let boot_b = boot_with_label("BOOT-B");
    let first = store
        .put_layer(&layer(vec![file("A.TXT", b"a")], Vec::new(), None))
        .unwrap();
    let second = store
        .put_layer(
            &Layer::from_delta(
                &LayerDelta::new(),
                "step",
                Some(first.digest.clone()),
                Some(boot_b.clone()),
            )
            .unwrap(),
        )
        .unwrap();
    store
        .commit_image(
            Some(&reference("demo:latest")),
            &sample_config(boot_a),
            vec![first, second],
            BTreeMap::new(),
        )
        .unwrap();
    let effective = store
        .load_effective_boot(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    assert_eq!(effective, boot_b);
}

#[test]
fn load_effective_boot_falls_back_to_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let boot = boot_with_label("CONFIG");
    let descriptor = store
        .put_layer(&layer(vec![file("A.TXT", b"a")], Vec::new(), None))
        .unwrap();
    store
        .commit_image(
            Some(&reference("demo:latest")),
            &sample_config(boot.clone()),
            vec![descriptor],
            BTreeMap::new(),
        )
        .unwrap();
    let effective = store
        .load_effective_boot(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    assert_eq!(effective, boot);
}

#[test]
fn load_effective_boot_returns_the_config_for_a_booter() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let descriptor = booter_descriptor(&store, &[0x5a; 1024]);
    let boot = BootMetadata::opaque(1024);
    store
        .commit_image(
            Some(&reference("game:latest")),
            &sample_config(boot.clone()),
            vec![descriptor],
            BTreeMap::new(),
        )
        .unwrap();
    let effective = store
        .load_effective_boot(&ImageRef::Tag(reference("game:latest")))
        .unwrap();
    assert_eq!(effective, boot);
}

#[test]
fn commit_image_writes_config_manifest_and_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let descriptor = store
        .put_layer(&layer(vec![file("A.TXT", b"a")], Vec::new(), None))
        .unwrap();
    let committed = store
        .commit_image(
            Some(&reference("demo:latest")),
            &sample_config(sample_boot()),
            vec![descriptor],
            BTreeMap::new(),
        )
        .unwrap();
    assert!(
        store
            .blob_path(&committed.manifest_digest)
            .unwrap()
            .is_file()
    );
    assert!(store.blob_path(&committed.config_digest).unwrap().is_file());
    for digest in &committed.layer_digests {
        assert!(store.blob_path(digest).unwrap().is_file());
    }
    assert_eq!(
        store
            .load(&ImageRef::Tag(reference("demo:latest")))
            .unwrap()
            .manifest_digest,
        committed.manifest_digest
    );
}

#[test]
fn commit_image_replaces_an_existing_tag() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let first_layer = store
        .put_layer(&layer(vec![file("A.TXT", b"one")], Vec::new(), None))
        .unwrap();
    let first = store
        .commit_image(
            Some(&reference("demo:latest")),
            &sample_config(sample_boot()),
            vec![first_layer],
            BTreeMap::new(),
        )
        .unwrap();
    let second_layer = store
        .put_layer(&layer(vec![file("A.TXT", b"two")], Vec::new(), None))
        .unwrap();
    let mut second_config = sample_config(sample_boot());
    second_config.cmd = Some("APP.EXE".to_owned());
    let second = store
        .commit_image(
            Some(&reference("demo:latest")),
            &second_config,
            vec![second_layer],
            BTreeMap::new(),
        )
        .unwrap();
    assert_ne!(first.manifest_digest, second.manifest_digest);
    assert_eq!(
        store
            .load(&ImageRef::Tag(reference("demo:latest")))
            .unwrap()
            .manifest_digest,
        second.manifest_digest
    );
}

#[test]
fn commit_image_without_a_tag_leaves_the_index_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let index_path = dir.path().join("images/index.json");
    let before = fs::read(&index_path).ok();
    let descriptor = store
        .put_layer(&layer(vec![file("A.TXT", b"a")], Vec::new(), None))
        .unwrap();
    let committed = store
        .commit_image(
            None,
            &sample_config(sample_boot()),
            vec![descriptor],
            BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(fs::read(&index_path).ok(), before);
    let manifest = store
        .load_manifest(&ImageRef::Digest(
            ImageDigest::parse(&committed.manifest_digest).unwrap(),
        ))
        .unwrap();
    assert_eq!(manifest.config.digest, committed.config_digest);
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn commit_image_keeps_the_raw_disk_annotation() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let descriptor = store
        .put_layer(&layer(vec![file("A.TXT", b"a")], Vec::new(), None))
        .unwrap();
    let raw = store.put_blob(b"raw disk").unwrap();
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_RAW_DISK_DIGEST.to_owned(), raw.digest.clone());
    store
        .commit_image(
            Some(&reference("demo:latest")),
            &sample_config(sample_boot()),
            vec![descriptor],
            annotations,
        )
        .unwrap();
    let manifest = store
        .load_manifest(&ImageRef::Tag(reference("demo:latest")))
        .unwrap();
    assert_eq!(manifest.raw_disk_digest(), Some(raw.digest.as_str()));
    assert!(
        manifest
            .layers
            .iter()
            .all(|layer| layer.digest != raw.digest)
    );
    assert_eq!(manifest.layers.len(), 1);
}

#[test]
fn commit_image_rejects_a_booter_with_tree_layers() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let descriptor = store
        .put_layer(&layer(vec![file("A.TXT", b"a")], Vec::new(), None))
        .unwrap();
    assert!(matches!(
        store.commit_image(
            Some(&reference("game:latest")),
            &sample_config(BootMetadata::opaque(1024)),
            vec![descriptor],
            BTreeMap::new(),
        ),
        Err(ImageError::InvalidManifest { .. })
    ));
}

#[allow(clippy::permissions_set_readonly_false)]
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
