mod support;

use std::fs;
use std::path::{Path, PathBuf};

use dosker_build::{
    RegistryError, resolve_target, validate_final, validate_registry, validate_target,
};
use dosker_core::{LicenseBasis, LicenseEntry, LicenseStatus, LicenseSummary};
use dosker_image::{
    Attributes, DosPath, ImageRef, LayerDelta, OciStore, Tree, TreeDir, TreeFile, fixed_timestamp,
};
use support::{
    empty_tree, sample_license, store_booter_base, store_final_booter, store_final_tree,
    store_licensed_tree_base, store_tree_base,
};

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn metadata(tags: &[&str]) -> String {
    let tags = tags
        .iter()
        .map(|tag| format!("\"{tag}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("schema = 1\nversion = \"1.0.0\"\ntags = [{tags}]\ndescription = \"Demo\"\n")
}

fn namespaces(owner: &str) -> String {
    format!(
        "schema = 1\n\n[owners.{owner}]\ngithub = \"{owner}\"\nproof = \"https://github.com/{owner}\"\n"
    )
}

fn fixture(
    image_owner: &str,
    registered_owner: &str,
    name: &str,
    doskerfile: &str,
) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("images").join(image_owner).join(name);
    write(&image.join("Doskerfile"), doskerfile);
    write(&image.join("registry.toml"), &metadata(&["latest"]));
    write(
        &dir.path().join("namespaces.toml"),
        &namespaces(registered_owner),
    );
    (dir, image)
}

fn store() -> (tempfile::TempDir, OciStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = OciStore::new(dir.path().to_path_buf());
    (dir, store)
}

fn valid_doskerfile() -> &'static str {
    "FROM base:latest\nCOPY APP C:\\APP\nLICENSE NAME=APP SPDX=MIT STATUS=redistributable BASIS=open-source-license COVERS=C:\\APP\\*\n"
}

fn write_app(image: &Path) {
    write(&image.join("APP/FOO.TXT"), "x");
}

#[test]
fn a_valid_tree_image_validates() {
    let (store_dir, store) = store();
    store_tree_base(
        store_dir.path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    let (registry, image) = fixture("i-zhirov", "i-zhirov", "demo", valid_doskerfile());
    write_app(&image);

    let validations = validate_registry(registry.path(), &store).unwrap();

    assert_eq!(validations.len(), 1);
    assert_eq!(validations[0].image, "images/i-zhirov/demo");
    assert_eq!(validations[0].version, "1.0.0");
    assert_eq!(validations[0].tags, vec!["latest".to_owned()]);
    assert_eq!(
        validations[0].license_summary,
        LicenseSummary::Redistributable
    );
}

#[test]
fn an_image_without_a_license_fails() {
    let (store_dir, store) = store();
    store_tree_base(
        store_dir.path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    let (registry, image) = fixture(
        "i-zhirov",
        "i-zhirov",
        "demo",
        "FROM base:latest\nCOPY APP C:\\APP\n",
    );
    write_app(&image);

    let error = validate_registry(registry.path(), &store).unwrap_err();

    assert!(matches!(error, RegistryError::Doskerfile { .. }));
}

#[test]
fn a_contradictory_license_status_fails() {
    let (store_dir, store) = store();
    store_tree_base(
        store_dir.path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    let (registry, image) = fixture(
        "i-zhirov",
        "i-zhirov",
        "demo",
        "FROM base:latest\nCOPY APP C:\\APP\nLICENSE NAME=APP SPDX=MIT STATUS=abandonware BASIS=good-faith EVIDENCE=https://example.org/evidence COVERS=C:\\APP\\*\n",
    );
    write_app(&image);

    let error = validate_registry(registry.path(), &store).unwrap_err();

    assert!(matches!(error, RegistryError::Doskerfile { .. }));
}

#[test]
fn an_unknown_owner_fails_on_a_per_image_target() {
    let (store_dir, store) = store();
    store_tree_base(
        store_dir.path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    let (_registry, image) = fixture("other", "i-zhirov", "demo", valid_doskerfile());
    write_app(&image);

    let target = resolve_target(&image, None).unwrap();
    let error = validate_target(&target, &store).unwrap_err();

    assert!(matches!(error, RegistryError::UnknownOwner { .. }));
}

#[test]
fn an_unknown_owner_fails_on_a_root_target() {
    let (store_dir, store) = store();
    store_tree_base(
        store_dir.path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    let (registry, image) = fixture("other", "i-zhirov", "demo", valid_doskerfile());
    write_app(&image);

    let error = validate_registry(registry.path(), &store).unwrap_err();

    assert!(matches!(error, RegistryError::UnknownOwner { .. }));
}

#[test]
fn a_duplicate_tag_fails_on_a_per_image_target() {
    let (store_dir, store) = store();
    store_tree_base(
        store_dir.path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    let (registry, image) = fixture("i-zhirov", "i-zhirov", "one", valid_doskerfile());
    write_app(&image);
    let second = registry.path().join("images/i-zhirov/two");
    write(&second.join("Doskerfile"), valid_doskerfile());
    write(&second.join("registry.toml"), &metadata(&["latest"]));

    let target = resolve_target(&image, None).unwrap();
    let error = validate_target(&target, &store).unwrap_err();

    assert!(matches!(error, RegistryError::DuplicateTag { .. }));
}

#[test]
fn an_uncovered_copy_directory_fails() {
    let (store_dir, store) = store();
    store_tree_base(
        store_dir.path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    let (registry, image) = fixture(
        "i-zhirov",
        "i-zhirov",
        "demo",
        "FROM base:latest\nCOPY APP C:\\APP\nLICENSE NAME=APP SPDX=MIT STATUS=redistributable BASIS=open-source-license COVERS=C:\\OTHER\\*\n",
    );
    write_app(&image);

    let error = validate_registry(registry.path(), &store).unwrap_err();

    assert!(matches!(error, RegistryError::Doskerfile { .. }));
}

#[test]
fn a_copyleft_image_without_a_notice_fails() {
    let (store_dir, store) = store();
    store_tree_base(
        store_dir.path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    let (registry, image) = fixture(
        "i-zhirov",
        "i-zhirov",
        "demo",
        "FROM base:latest\nCOPY APP C:\\APP\nLICENSE NAME=APP SPDX=GPL-2.0-only STATUS=redistributable BASIS=open-source-license SOURCE_URL=https://example.org/source COVERS=C:\\APP\\*\n",
    );
    write_app(&image);

    let error = validate_registry(registry.path(), &store).unwrap_err();

    assert!(matches!(error, RegistryError::Doskerfile { .. }));
}

#[test]
fn a_booter_image_with_a_wildcard_validates() {
    let (store_dir, store) = store();
    store_booter_base(store_dir.path(), "boot:latest", b"BOOTER");
    let (registry, _image) = fixture(
        "i-zhirov",
        "i-zhirov",
        "demo",
        "FROM boot:latest\nLICENSE NAME=BOOT SPDX=MIT STATUS=redistributable BASIS=open-source-license COVERS=*\n",
    );

    let validations = validate_registry(registry.path(), &store).unwrap();

    assert_eq!(validations.len(), 1);
    assert_eq!(
        validations[0].license_summary,
        LicenseSummary::Redistributable
    );
    assert!(validations[0].checked_paths.is_empty());
}

#[test]
fn a_booter_image_without_a_wildcard_fails() {
    let (store_dir, store) = store();
    store_booter_base(store_dir.path(), "boot:latest", b"BOOTER");
    let (registry, _image) = fixture(
        "i-zhirov",
        "i-zhirov",
        "demo",
        "FROM boot:latest\nLICENSE NAME=BOOT SPDX=MIT STATUS=redistributable BASIS=open-source-license\n",
    );

    let error = validate_registry(registry.path(), &store).unwrap_err();

    assert!(matches!(error, RegistryError::BooterNeedsWildcard));
}

#[test]
fn a_booter_with_a_copyleft_entry_fails() {
    let (store_dir, store) = store();
    store_booter_base(store_dir.path(), "boot:latest", b"BOOTER");
    let (registry, _image) = fixture(
        "i-zhirov",
        "i-zhirov",
        "demo",
        "FROM boot:latest\nLICENSE NAME=BOOT SPDX=GPL-2.0-only STATUS=redistributable BASIS=open-source-license SOURCE_URL=https://example.org/source COVERS=*\n",
    );

    let error = validate_registry(registry.path(), &store).unwrap_err();

    assert!(matches!(
        error,
        RegistryError::BooterNoticeUnverifiable { .. }
    ));
}

#[test]
fn a_namespaced_base_carries_its_entries() {
    let (store_dir, store) = store();
    let mut base_entry = sample_license("BASE");
    base_entry.covers = vec![r"C:\APP\*".to_owned()];
    store_licensed_tree_base(store_dir.path(), "owner/base:tag", vec![base_entry]);
    let (registry, image) = fixture(
        "owner",
        "owner",
        "demo",
        "FROM owner/base:tag\nCOPY APP C:\\APP\n",
    );
    write_app(&image);

    let validations = validate_registry(registry.path(), &store).unwrap();

    assert_eq!(validations.len(), 1);
    assert_eq!(
        validations[0].license_summary,
        LicenseSummary::Redistributable
    );
}

#[test]
fn a_missing_namespaced_base_reports_a_pull_hint() {
    let (_store_dir, store) = store();
    let (registry, _image) = fixture("owner", "owner", "demo", "FROM owner/missing:latest\n");

    let error = validate_registry(registry.path(), &store).unwrap_err();

    match error {
        RegistryError::BaseNotLocal { reference } => {
            assert_eq!(reference, "owner/missing:latest");
        }
        other => panic!("expected a base error, got {other:?}"),
    }
}

fn final_tree(dirs: &[&str], files: &[(&str, &[u8])]) -> Tree {
    let mut delta = LayerDelta::new();
    for dir in dirs {
        delta.add_dir(TreeDir {
            path: DosPath::parse(dir).expect("the directory path parses"),
            attributes: Attributes::default(),
            timestamp: fixed_timestamp(),
        });
    }
    for (path, bytes) in files {
        delta.add_file(TreeFile {
            path: DosPath::parse(path).expect("the file path parses"),
            attributes: Attributes {
                archive: true,
                ..Attributes::default()
            },
            timestamp: fixed_timestamp(),
            bytes: bytes.to_vec(),
        });
    }
    let mut tree = Tree::new();
    tree.apply_delta(&delta).expect("the tree applies");
    tree
}

fn final_license(name: &str, spdx: &str, covers: &[&str]) -> LicenseEntry {
    LicenseEntry {
        name: name.to_owned(),
        spdx: spdx.to_owned(),
        holder: None,
        status: LicenseStatus::Redistributable,
        basis: Some(LicenseBasis::OpenSourceLicense),
        evidence: None,
        license_url: None,
        covers: covers.iter().map(|cover| (*cover).to_owned()).collect(),
        source_url: None,
        notice: None,
    }
}

#[test]
fn a_final_tree_outside_coverage_fails() {
    let (store_dir, store) = store();
    let tree = final_tree(&["APP"], &[("APP/FOO.TXT", b"x")]);
    store_final_tree(
        store_dir.path(),
        "final:latest",
        &tree,
        vec![final_license("APP", "MIT", &[r"C:\OTHER\*"])],
    );
    let reference = ImageRef::parse("final:latest").unwrap();

    let error = validate_final(&reference, &store).unwrap_err();

    match error {
        RegistryError::Final { reason, .. } => {
            assert!(reason.contains("does not cover"), "{reason}");
        }
        other => panic!("expected a coverage error, got {other:?}"),
    }
}

#[test]
fn a_final_tree_with_the_notice_passes() {
    let (store_dir, store) = store();
    let tree = final_tree(
        &["APP", "DOSKER"],
        &[("APP/FOO.TXT", b"x"), ("DOSKER/NOTICE.TXT", b"notice")],
    );
    let mut license = final_license("APP", "GPL-2.0-only", &[r"C:\APP\*"]);
    license.source_url = Some("https://example.org/source".to_owned());
    store_final_tree(store_dir.path(), "final:latest", &tree, vec![license]);
    let reference = ImageRef::parse("final:latest").unwrap();

    let validation = validate_final(&reference, &store).unwrap();

    assert_eq!(validation.image, "final:latest");
    assert_eq!(validation.license_summary, LicenseSummary::Redistributable);
}

#[test]
fn a_final_booter_without_a_wildcard_fails() {
    let (store_dir, store) = store();
    store_final_booter(
        store_dir.path(),
        "boot:latest",
        b"BOOT",
        vec![final_license("BOOT", "MIT", &[])],
    );
    let reference = ImageRef::parse("boot:latest").unwrap();

    let error = validate_final(&reference, &store).unwrap_err();

    assert!(matches!(error, RegistryError::BooterNeedsWildcard));
}
