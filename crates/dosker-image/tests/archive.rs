mod support;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::Path;

use chrono::{TimeZone, Utc};
use dosker_image::{
    ANNOTATION_RAW_DISK_DIGEST, ANNOTATION_REF_NAME, Attributes, Descriptor, DosPath, Index, Layer,
    LayerDelta, MEDIA_TYPE_MANIFEST, Manifest, OCI_LAYOUT_VERSION, OciLayout, OciStore, TreeFile,
    canonical_json, load_image, save_image, sha256_digest,
};
use dosker_image::{ImageRef, ImageReference};
use support::{sample_boot, sample_config};

fn reference(value: &str) -> ImageReference {
    ImageReference::parse(value).unwrap()
}

fn open_store(dir: &Path) -> OciStore {
    OciStore::new(dir.to_path_buf())
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

fn layer(files: Vec<TreeFile>) -> Layer {
    Layer::from_delta(
        &LayerDelta {
            whiteouts: Vec::new(),
            files,
            dirs: Vec::new(),
        },
        "test",
        None,
        None,
    )
    .unwrap()
}

fn tree_image(dir: &Path, tag: &str, content: &[u8]) -> (OciStore, String) {
    let store = open_store(dir);
    let descriptor = store
        .put_layer(&layer(vec![file("A.TXT", content)]))
        .unwrap();
    let raw = store.put_blob(b"raw disk").unwrap();
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_RAW_DISK_DIGEST.to_owned(), raw.digest);
    let committed = store
        .commit_image(
            Some(&reference(tag)),
            &sample_config(sample_boot()),
            vec![descriptor],
            annotations,
        )
        .unwrap();
    (store, committed.manifest_digest)
}

fn booter_image(dir: &Path, tag: &str) -> (OciStore, String) {
    let source = dir.join("booter.img");
    fs::write(&source, vec![0x5a; 1024]).unwrap();
    let store = open_store(dir);
    let image = store
        .import(
            &source,
            &reference(tag),
            dosker_image::ImageMedia::Booter,
            None,
            Utc.timestamp_opt(1, 0).unwrap(),
        )
        .unwrap();
    (store, image.manifest_digest)
}

fn members(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut archive = tar::Archive::new(File::open(path).unwrap());
    let mut result = BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry.path().unwrap().to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
        result.insert(name, bytes);
    }
    result
}

fn member_names(path: &Path) -> Vec<String> {
    members(path).into_keys().collect()
}

fn write_tar(path: &Path, entries: &[(&str, &[u8])]) {
    let file = File::create(path).unwrap();
    let mut builder = tar::Builder::new(file);
    for (name, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        builder.append_data(&mut header, name, *bytes).unwrap();
    }
    builder.finish().unwrap();
}

fn blob_member(digest: &str) -> String {
    format!("blobs/sha256/{}", digest.strip_prefix("sha256:").unwrap())
}

fn layout_bytes() -> Vec<u8> {
    canonical_json(&OciLayout::new()).unwrap()
}

fn index_bytes(index: &Index) -> Vec<u8> {
    canonical_json(index).unwrap()
}

fn tagged_index(digest: &str, size: u64, tag: &str) -> Index {
    let mut index = Index::new();
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_REF_NAME.to_owned(), tag.to_owned());
    index.add(Descriptor {
        media_type: MEDIA_TYPE_MANIFEST.to_owned(),
        digest: digest.to_owned(),
        size,
        annotations: Some(annotations),
    });
    index
}

#[test]
fn save_writes_the_layout_index_and_referenced_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let (store, digest) = tree_image(dir.path(), "demo:latest", b"hello");
    let destination = dir.path().join("demo.tar");
    let report = save_image(&store, &reference("demo:latest"), &destination).unwrap();

    let manifest: Manifest = serde_json::from_slice(&store.read_blob(&digest).unwrap()).unwrap();
    let mut expected = vec![
        "oci-layout".to_owned(),
        "index.json".to_owned(),
        blob_member(&digest),
        blob_member(&manifest.config.digest),
    ];
    expected.extend(
        manifest
            .layers
            .iter()
            .map(|layer| blob_member(&layer.digest)),
    );
    expected.push(blob_member(manifest.raw_disk_digest().unwrap()));
    expected.sort();
    assert_eq!(member_names(&destination), expected);
    assert_eq!(report.blobs, manifest.layers.len() + 3);
    assert_eq!(report.reference, "demo:latest");
}

#[test]
fn save_and_load_preserve_the_manifest_digest() {
    let dir = tempfile::tempdir().unwrap();
    let (store, digest) = tree_image(dir.path(), "demo:latest", b"hello");
    let archive = dir.path().join("demo.tar");
    save_image(&store, &reference("demo:latest"), &archive).unwrap();

    let target_store = open_store(&dir.path().join("target"));
    let report = load_image(&target_store, &archive).unwrap();
    assert_eq!(report.references, vec!["demo:latest".to_owned()]);
    let reference = ImageRef::Tag(reference("demo:latest"));
    assert_eq!(
        target_store.load(&reference).unwrap().manifest_digest,
        digest
    );
    assert_eq!(
        target_store.load_tree(&reference).unwrap(),
        store.load_tree(&reference).unwrap()
    );
}

#[test]
fn save_writes_a_booter_as_one_raw_blob() {
    let dir = tempfile::tempdir().unwrap();
    let (store, digest) = booter_image(dir.path(), "game:latest");
    let archive = dir.path().join("game.tar");
    let report = save_image(&store, &reference("game:latest"), &archive).unwrap();

    let manifest: Manifest = serde_json::from_slice(&store.read_blob(&digest).unwrap()).unwrap();
    assert_eq!(manifest.layers.len(), 1);
    assert_eq!(
        manifest.layers[0].media_type,
        dosker_image::MEDIA_TYPE_BOOTER
    );
    assert!(manifest.raw_disk_digest().is_none());

    let mut expected = vec![
        "oci-layout".to_owned(),
        "index.json".to_owned(),
        blob_member(&digest),
        blob_member(&manifest.config.digest),
        blob_member(&manifest.layers[0].digest),
    ];
    expected.sort();
    assert_eq!(member_names(&archive), expected);
    assert_eq!(report.blobs, 3);
}

#[test]
fn save_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = tree_image(dir.path(), "demo:latest", b"hello");
    let first = dir.path().join("first.tar");
    let second = dir.path().join("second.tar");
    let first_report = save_image(&store, &reference("demo:latest"), &first).unwrap();
    let second_report = save_image(&store, &reference("demo:latest"), &second).unwrap();
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    assert_eq!(first_report.bytes, second_report.bytes);
}

#[test]
fn load_ignores_a_dot_prefix_and_a_root_entry() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("prefixed.tar");
    let layout = layout_bytes();
    let index = index_bytes(&Index::new());
    write_tar(
        &archive,
        &[
            ("./", b""),
            ("./oci-layout", &layout),
            ("./index.json", &index),
        ],
    );
    let target = open_store(&dir.path().join("target"));
    let report = load_image(&target, &archive).unwrap();
    assert!(report.references.is_empty());
}

#[test]
fn load_rejects_an_unsupported_layout_version() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("bad.tar");
    let layout = br#"{"imageLayoutVersion":"1.1.0"}"#;
    let index = index_bytes(&Index::new());
    write_tar(&archive, &[("oci-layout", layout), ("index.json", &index)]);
    let target = open_store(&dir.path().join("target"));
    let error = load_image(&target, &archive).unwrap_err();
    assert!(
        error.to_string().contains("unsupported layout version"),
        "{error}"
    );
}

#[test]
fn load_rejects_a_missing_manifest_blob() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("bad.tar");
    let index = tagged_index(&sha256_digest(b"absent"), 6, "demo:latest");
    let layout = layout_bytes();
    let index = index_bytes(&index);
    write_tar(&archive, &[("oci-layout", &layout), ("index.json", &index)]);
    let target = open_store(&dir.path().join("target"));
    let error = load_image(&target, &archive).unwrap_err();
    assert!(
        error.to_string().contains("missing manifest blob"),
        "{error}"
    );
}

#[test]
fn load_rejects_a_corrupt_blob() {
    let dir = tempfile::tempdir().unwrap();
    let (store, digest) = tree_image(dir.path(), "demo:latest", b"hello");
    let manifest_bytes = store.read_blob(&digest).unwrap();
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).unwrap();
    let config = store.read_blob(&manifest.config.digest).unwrap();
    let mut corrupt = config;
    corrupt.extend_from_slice(b" ");

    let layout = layout_bytes();
    let index = index_bytes(&tagged_index(
        &digest,
        manifest_bytes.len() as u64,
        "demo:latest",
    ));
    let archive = dir.path().join("bad.tar");
    write_tar(
        &archive,
        &[
            ("oci-layout", &layout),
            ("index.json", &index),
            (&blob_member(&digest), &manifest_bytes),
            (&blob_member(&manifest.config.digest), &corrupt),
        ],
    );
    let target = open_store(&dir.path().join("target"));
    let error = load_image(&target, &archive).unwrap_err();
    assert!(error.to_string().contains("corrupt blob"), "{error}");
}

#[test]
fn load_rejects_an_index_descriptor_without_a_ref() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("bad.tar");
    let mut index = Index::new();
    index.add(Descriptor {
        media_type: MEDIA_TYPE_MANIFEST.to_owned(),
        digest: sha256_digest(b"manifest"),
        size: 8,
        annotations: None,
    });
    let layout = layout_bytes();
    let index = index_bytes(&index);
    write_tar(&archive, &[("oci-layout", &layout), ("index.json", &index)]);
    let target = open_store(&dir.path().join("target"));
    let error = load_image(&target, &archive).unwrap_err();
    assert!(error.to_string().contains("no ref annotation"), "{error}");
}

#[test]
fn load_replaces_an_existing_tag_with_a_new_digest() {
    let dir = tempfile::tempdir().unwrap();
    let first_dir = dir.path().join("first");
    let second_dir = dir.path().join("second");
    fs::create_dir_all(&first_dir).unwrap();
    fs::create_dir_all(&second_dir).unwrap();
    let (first_store, first_digest) = tree_image(&first_dir, "demo:latest", b"one");
    let (second_store, second_digest) = tree_image(&second_dir, "demo:latest", b"two");
    let first = dir.path().join("first.tar");
    let second = dir.path().join("second.tar");
    save_image(&first_store, &reference("demo:latest"), &first).unwrap();
    save_image(&second_store, &reference("demo:latest"), &second).unwrap();

    let target = open_store(&dir.path().join("target"));
    load_image(&target, &first).unwrap();
    load_image(&target, &second).unwrap();
    assert_eq!(
        target
            .load(&ImageRef::Tag(reference("demo:latest")))
            .unwrap()
            .manifest_digest,
        second_digest
    );
    assert_ne!(first_digest, second_digest);
}

#[test]
fn load_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let (store, digest) = tree_image(dir.path(), "demo:latest", b"hello");
    let archive = dir.path().join("demo.tar");
    save_image(&store, &reference("demo:latest"), &archive).unwrap();

    let target = open_store(&dir.path().join("target"));
    let first = load_image(&target, &archive).unwrap();
    let second = load_image(&target, &archive).unwrap();
    assert_eq!(second.references, first.references);
    assert_eq!(
        target
            .load(&ImageRef::Tag(reference("demo:latest")))
            .unwrap()
            .manifest_digest,
        digest
    );
    assert_eq!(target.list().unwrap().len(), 1);
    assert_eq!(second.blobs_written, 0);
}

#[test]
fn archive_constants_are_fixed() {
    assert_eq!(dosker_image::ARCHIVE_MODE, 0o644);
    assert_eq!(dosker_image::ARCHIVE_MTIME_SECONDS, 0);
    assert_eq!(OCI_LAYOUT_VERSION, "1.0.0");
}

#[test]
fn namespaced_tag_round_trips_through_an_archive() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _digest) = tree_image(dir.path(), "owner/name:tag", b"hello");
    let archive = dir.path().join("image.tar");
    save_image(&store, &reference("owner/name:tag"), &archive).unwrap();

    let target_dir = tempfile::tempdir().unwrap();
    let target = open_store(target_dir.path());
    let report = load_image(&target, &archive).unwrap();

    assert_eq!(report.references, vec!["owner/name:tag"]);
}
