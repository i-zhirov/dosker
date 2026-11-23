use std::fs;
use std::path::{Path, PathBuf};

use dosker_build::{
    RegistryError, parse_metadata, parse_namespaces, registry_root, scan_tags, validate_metadata,
    validate_namespaces,
};

fn write(root: &Path, relative: &str, text: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, text).unwrap();
    path
}

fn metadata(version: &str, tags: &[&str], description: &str) -> String {
    let tags = tags
        .iter()
        .map(|tag| format!("\"{tag}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "schema = 1\nversion = \"{version}\"\ntags = [{tags}]\ndescription = \"{description}\"\n"
    )
}

fn namespaces(owner: &str, github: &str, proof: &str, note: Option<&str>) -> String {
    let note = note
        .map(|note| format!("note = \"{note}\"\n"))
        .unwrap_or_default();
    format!("schema = 1\n\n[owners.{owner}]\ngithub = \"{github}\"\nproof = \"{proof}\"\n{note}")
}

#[test]
fn a_valid_registry_toml_parses() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "registry.toml",
        &metadata("1.0.0", &["latest", "1.0.0"], "A DOS game"),
    );

    let parsed = parse_metadata(&path).unwrap();

    assert_eq!(parsed.schema, 1);
    assert_eq!(parsed.version, "1.0.0");
    assert_eq!(parsed.tags, vec!["latest".to_owned(), "1.0.0".to_owned()]);
    assert_eq!(parsed.description, "A DOS game");
    validate_metadata(&parsed, &path).unwrap();
}

#[test]
fn registry_toml_rejects_an_unknown_field() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "registry.toml",
        &format!(
            "{}extra = 1\n",
            metadata("1.0.0", &["latest"], "A DOS game")
        ),
    );

    assert!(matches!(
        parse_metadata(&path),
        Err(RegistryError::Metadata { .. })
    ));
}

#[test]
fn registry_toml_rejects_a_bad_version_or_tag() {
    let dir = tempfile::tempdir().unwrap();
    let bad_version = write(
        dir.path(),
        "bad-version/registry.toml",
        &metadata("not a tag", &["latest"], "A DOS game"),
    );
    let parsed = parse_metadata(&bad_version).unwrap();
    assert!(validate_metadata(&parsed, &bad_version).is_err());

    let bad_tag = write(
        dir.path(),
        "bad-tag/registry.toml",
        &metadata("1.0.0", &["not a tag"], "A DOS game"),
    );
    let parsed = parse_metadata(&bad_tag).unwrap();
    assert!(validate_metadata(&parsed, &bad_tag).is_err());
}

#[test]
fn registry_toml_rejects_an_empty_description() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "registry.toml",
        &metadata("1.0.0", &["latest"], ""),
    );
    let parsed = parse_metadata(&path).unwrap();

    assert!(validate_metadata(&parsed, &path).is_err());
}

#[test]
fn registry_toml_rejects_more_than_32_tags() {
    let dir = tempfile::tempdir().unwrap();
    let tags = (0..33)
        .map(|index| format!("tag{index}"))
        .collect::<Vec<_>>();
    let tag_refs = tags.iter().map(String::as_str).collect::<Vec<_>>();
    let path = write(
        dir.path(),
        "registry.toml",
        &metadata("1.0.0", &tag_refs, "A DOS game"),
    );
    let parsed = parse_metadata(&path).unwrap();

    assert!(validate_metadata(&parsed, &path).is_err());
}

#[test]
fn namespaces_require_a_matching_github_and_proof() {
    let dir = tempfile::tempdir().unwrap();
    let valid = write(
        dir.path(),
        "valid.toml",
        &namespaces("i-zhirov", "i-zhirov", "https://github.com/i-zhirov", None),
    );
    let parsed = parse_namespaces(&valid).unwrap();
    validate_namespaces(&parsed, &valid, "i-zhirov").unwrap();

    let mismatch = write(
        dir.path(),
        "mismatch.toml",
        &namespaces("i-zhirov", "other", "https://github.com/i-zhirov", None),
    );
    let parsed = parse_namespaces(&mismatch).unwrap();
    assert!(matches!(
        validate_namespaces(&parsed, &mismatch, "i-zhirov"),
        Err(RegistryError::Namespaces { .. })
    ));

    let unproven = write(
        dir.path(),
        "unproven.toml",
        &namespaces("i-zhirov", "i-zhirov", "", None),
    );
    let parsed = parse_namespaces(&unproven).unwrap();
    assert!(matches!(
        validate_namespaces(&parsed, &unproven, "i-zhirov"),
        Err(RegistryError::UnprovenOwner { .. })
    ));

    let unknown = write(
        dir.path(),
        "unknown.toml",
        &namespaces("i-zhirov", "i-zhirov", "https://github.com/i-zhirov", None),
    );
    let parsed = parse_namespaces(&unknown).unwrap();
    assert!(matches!(
        validate_namespaces(&parsed, &unknown, "other"),
        Err(RegistryError::UnknownOwner { .. })
    ));
}

#[test]
fn namespaces_reject_an_invalid_key_or_a_long_note() {
    let dir = tempfile::tempdir().unwrap();
    let bad_key = write(
        dir.path(),
        "bad-key.toml",
        &namespaces("Bad_Key", "Bad_Key", "https://github.com/i-zhirov", None),
    );
    let parsed = parse_namespaces(&bad_key).unwrap();
    assert!(matches!(
        validate_namespaces(&parsed, &bad_key, "Bad_Key"),
        Err(RegistryError::Namespaces { .. })
    ));

    let note = "x".repeat(281);
    let long_note = write(
        dir.path(),
        "long-note.toml",
        &namespaces(
            "i-zhirov",
            "i-zhirov",
            "https://github.com/i-zhirov",
            Some(&note),
        ),
    );
    let parsed = parse_namespaces(&long_note).unwrap();
    assert!(matches!(
        validate_namespaces(&parsed, &long_note, "i-zhirov"),
        Err(RegistryError::Namespaces { .. })
    ));
}

#[test]
fn a_duplicate_tag_names_both_images() {
    let dir = tempfile::tempdir().unwrap();
    let first = write(
        dir.path(),
        "images/owner/one/registry.toml",
        &metadata("1.0.0", &["latest"], "One"),
    );
    let second = write(
        dir.path(),
        "images/owner/two/registry.toml",
        &metadata("2.0.0", &["latest"], "Two"),
    );

    let error = scan_tags(dir.path()).unwrap_err();
    match error {
        RegistryError::DuplicateTag {
            tag,
            first: found_first,
            second: found_second,
        } => {
            assert_eq!(tag, "latest");
            let mut found = [found_first, found_second];
            found.sort();
            let mut expected = [first, second];
            expected.sort();
            assert_eq!(found, expected);
        }
        other => panic!("expected a duplicate tag, got {other:?}"),
    }
}

#[test]
fn registry_root_walks_up_to_the_images_directory() {
    let dir = tempfile::tempdir().unwrap();
    let image = write(
        dir.path(),
        "images/owner/name/registry.toml",
        &metadata("1.0.0", &["latest"], "A DOS game"),
    );

    let root = registry_root(image.parent().unwrap(), None).unwrap();

    assert_eq!(root, dir.path().to_path_buf());
}

#[test]
fn registry_root_honours_an_explicit_path() {
    let dir = tempfile::tempdir().unwrap();
    let image = write(
        dir.path(),
        "images/owner/name/registry.toml",
        &metadata("1.0.0", &["latest"], "A DOS game"),
    );
    let explicit = tempfile::tempdir().unwrap();

    let root = registry_root(image.parent().unwrap(), Some(explicit.path())).unwrap();

    assert_eq!(root, explicit.path().to_path_buf());
}
