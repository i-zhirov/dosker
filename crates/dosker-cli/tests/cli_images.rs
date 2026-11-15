mod common;

use common::{CliRoot, stderr_text, stdout_text};
use dosker_image::{ImageRef, ImageReference, OciStore};

#[test]
fn images_lists_imported_references() {
    let root = CliRoot::new();
    root.import_image_as("zeta:latest");
    root.import_image_as("alpha:v2");
    root.import_image_as("alpha:v1");
    let output = root.run(&["images"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "REFERENCE DIGEST SIZE CREATED");
    assert!(lines[1].starts_with("alpha:v1 "));
    assert!(lines[2].starts_with("alpha:v2 "));
    assert!(lines[3].starts_with("zeta:latest "));
}

#[test]
fn images_lists_two_tags_for_the_same_digest() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let tag = root.run(&["tag", "demo:latest", "copy:latest"]);
    assert_eq!(tag.status.code(), Some(0), "stderr: {}", stderr_text(&tag));
    let output = root.run(&["images"]);
    let text = stdout_text(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.iter().any(|line| line.starts_with("demo:latest ")));
    assert!(lines.iter().any(|line| line.starts_with("copy:latest ")));
    let digests: std::collections::BTreeSet<&str> = lines
        .iter()
        .skip(1)
        .map(|line| line.split_whitespace().nth(1).unwrap())
        .collect();
    assert_eq!(digests.len(), 1);
}

#[test]
fn rmi_untags_an_image_with_another_tag() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    root.run(&["tag", "demo:latest", "copy:latest"]);
    let output = root.run(&["rmi", "demo:latest"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("untagged demo:latest"));
    assert!(root.run(&["images"]).status.success());
    assert!(stdout_text(&root.run(&["images"])).contains("copy:latest"));
}

#[test]
fn rmi_untags_the_last_tag_and_keeps_the_manifest_addressable() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let store = OciStore::new(root.path().to_path_buf());
    let image = store
        .load(&ImageRef::Tag(
            ImageReference::parse("demo:latest").unwrap(),
        ))
        .unwrap();
    let output = root.run(&["rmi", "demo:latest"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("untagged demo:latest"));
    assert!(
        store
            .load(&ImageRef::Digest(
                dosker_image::ImageDigest::parse(&image.manifest_digest).unwrap()
            ))
            .is_ok()
    );
}

#[test]
fn rmi_by_digest_removes_the_manifest_and_blobs() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let store = OciStore::new(root.path().to_path_buf());
    let image = store
        .load(&ImageRef::Tag(
            ImageReference::parse("demo:latest").unwrap(),
        ))
        .unwrap();
    assert!(root.run(&["rmi", "demo:latest"]).status.success());
    let output = root.run(&["rmi", &image.manifest_digest]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("removed "));
    assert!(!store.blob_path(&image.manifest_digest).unwrap().exists());
}

#[test]
fn rmi_rejects_an_unknown_reference() {
    let root = CliRoot::new();
    let output = root.run(&["rmi", "missing:latest"]);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn rmi_rejects_an_invalid_reference() {
    let root = CliRoot::new();
    let output = root.run(&["rmi", "Bad"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn tag_adds_a_reference_and_create_uses_it() {
    let root = CliRoot::new();
    root.import_image_as("demo:latest");
    let output = root.run(&["tag", "demo:latest", "copy:latest"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("tagged copy:latest"));
    let create = root.run(&["create", "copy:latest", "--name", "copy"]);
    assert_eq!(
        create.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&create)
    );
}

#[test]
fn tag_rejects_an_unknown_source() {
    let root = CliRoot::new();
    let output = root.run(&["tag", "missing:latest", "copy:latest"]);
    assert_eq!(output.status.code(), Some(1));
}
