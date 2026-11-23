mod common;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::Path;
use std::time::Duration;

use common::{CliRoot, run_captured, stderr_text, stdout_text};
use dosker_image::{
    ANNOTATION_REF_NAME, Descriptor, Index, MEDIA_TYPE_MANIFEST, Manifest, OciLayout, OciStore,
    canonical_json,
};
use dosker_image::{ImageRef, ImageReference};

fn image_digest(root: &CliRoot, tag: &str) -> String {
    let store = OciStore::new(root.path().to_path_buf());
    let reference = ImageReference::parse(tag).unwrap();
    store
        .load(&ImageRef::Tag(reference))
        .unwrap()
        .manifest_digest
}

fn save(root: &CliRoot, archive: &Path) -> std::process::Output {
    root.run(&[
        "save",
        "-o",
        archive.to_str().expect("archive path is utf-8"),
        "demo:latest",
    ])
}

fn blob_member(digest: &str) -> String {
    format!("blobs/sha256/{}", digest.strip_prefix("sha256:").unwrap())
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

#[test]
fn save_writes_a_tar_for_a_tagged_image() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let archive = root.path().join("saved.tar");
    let output = save(&root, &archive);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(archive.is_file());
    assert_eq!(
        stdout_text(&output).trim_end(),
        format!("saved demo:latest to {}", archive.display())
    );
}

#[test]
fn save_defaults_the_output_name_to_name_tag_tar() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let directory = tempfile::tempdir().unwrap();
    let mut command = root.command();
    command.current_dir(directory.path());
    command.args(["save", "demo:latest"]);
    let output = run_captured(&mut command, Duration::from_secs(45));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(directory.path().join("demo-latest.tar").is_file());
}

#[test]
fn save_uses_a_safe_default_name_for_a_namespaced_reference() {
    let root = CliRoot::new();
    root.import_image_as("owner/name:tag");
    let directory = tempfile::tempdir().unwrap();
    let mut command = root.command();
    command.current_dir(directory.path());
    command.args(["save", "owner/name:tag"]);
    let output = run_captured(&mut command, Duration::from_secs(45));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(directory.path().join("owner-name-tag.tar").is_file());
}

#[test]
fn save_rejects_an_unknown_image() {
    let root = CliRoot::new();
    let archive = root.path().join("missing.tar");
    let output = root.run(&[
        "save",
        "-o",
        archive.to_str().expect("archive path is utf-8"),
        "missing:latest",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!archive.exists());
}

#[test]
fn save_rejects_a_digest_reference() {
    let root = CliRoot::new();
    let digest = format!("sha256:{}", "0".repeat(64));
    let output = root.run(&["save", &digest]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr_text(&output));
    assert!(stderr_text(&output).contains("save requires a tag reference"));
}

#[test]
fn load_round_trips_a_tree_image() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let digest = image_digest(&root, "demo:latest");
    let archive = root.path().join("demo.tar");
    assert_eq!(save(&root, &archive).status.code(), Some(0));

    assert_eq!(root.run(&["rmi", "demo:latest"]).status.code(), Some(0));
    assert_eq!(root.run(&["rmi", &digest]).status.code(), Some(0));
    assert_eq!(image_digest_opt(&root, "demo:latest"), None);

    let output = root.run(&["load", archive.to_str().expect("archive path is utf-8")]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output).trim_end(), "loaded demo:latest");
    assert_eq!(image_digest(&root, "demo:latest"), digest);
    assert!(stdout_text(&root.run(&["images"])).contains("demo:latest"));
}

#[test]
fn load_round_trips_a_booter_image() {
    let root = CliRoot::new();
    let source = root.path().join("game.img");
    fs::write(&source, vec![0x5a; 4096]).unwrap();
    let output = root.run(&[
        "image",
        "import",
        "--booter",
        source.to_str().expect("source path is utf-8"),
        "game:latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let digest = image_digest(&root, "game:latest");
    let archive = root.path().join("game.tar");
    let output = root.run(&[
        "save",
        "-o",
        archive.to_str().expect("archive path is utf-8"),
        "game:latest",
    ]);
    assert_eq!(output.status.code(), Some(0));

    assert_eq!(root.run(&["rmi", "game:latest"]).status.code(), Some(0));
    assert_eq!(root.run(&["rmi", &digest]).status.code(), Some(0));
    assert_eq!(
        root.run(&["load", archive.to_str().expect("archive path is utf-8")])
            .status
            .code(),
        Some(0)
    );
    let output = root.run(&["create", "game:latest"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn load_rejects_a_missing_archive() {
    let root = CliRoot::new();
    let missing = root.path().join("absent.tar");
    let output = root.run(&["load", missing.to_str().expect("path is utf-8")]);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn load_rejects_a_tar_without_an_oci_layout() {
    let root = CliRoot::new();
    let archive = root.path().join("empty.tar");
    fs::write(&archive, b"").unwrap();
    let output = root.run(&["load", archive.to_str().expect("path is utf-8")]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_text(&output));
    assert!(stderr_text(&output).contains("invalid image archive"));
}

#[test]
fn load_rejects_a_corrupt_blob() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let store = OciStore::new(root.path().to_path_buf());
    let digest = image_digest(&root, "demo:latest");
    let manifest_bytes = store.read_blob(&digest).unwrap();
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).unwrap();
    let mut config = store.read_blob(&manifest.config.digest).unwrap();
    config.extend_from_slice(b" ");

    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_REF_NAME.to_owned(), "demo:latest".to_owned());
    let mut index = Index::new();
    index.add(Descriptor {
        media_type: MEDIA_TYPE_MANIFEST.to_owned(),
        digest: digest.clone(),
        size: manifest_bytes.len() as u64,
        annotations: Some(annotations),
    });
    let layout = canonical_json(&OciLayout::new()).unwrap();
    let index = canonical_json(&index).unwrap();
    let archive = root.path().join("corrupt.tar");
    write_tar(
        &archive,
        &[
            ("oci-layout", &layout),
            ("index.json", &index),
            (&blob_member(&digest), &manifest_bytes),
            (&blob_member(&manifest.config.digest), &config),
        ],
    );
    let output = root.run(&["load", archive.to_str().expect("path is utf-8")]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_text(&output));
    assert!(stderr_text(&output).contains("invalid image archive"));
    assert!(stderr_text(&output).contains("corrupt blob"));
}

#[test]
fn load_keeps_the_tag_when_the_archive_is_loaded_twice() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let archive = root.path().join("demo.tar");
    assert_eq!(save(&root, &archive).status.code(), Some(0));
    let path = archive.to_str().expect("archive path is utf-8");
    let first = root.run(&["load", path]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&first)
    );
    let second = root.run(&["load", path]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&second)
    );
    let store = OciStore::new(root.path().to_path_buf());
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(image_digest_opt(&root, "demo:latest").is_some());
}

fn image_digest_opt(root: &CliRoot, tag: &str) -> Option<String> {
    let store = OciStore::new(root.path().to_path_buf());
    let reference = ImageReference::parse(tag).unwrap();
    store
        .load(&ImageRef::Tag(reference))
        .ok()
        .map(|image| image.manifest_digest)
}
