use dosker_build::registry::{RegistryError, coverage_misses, notice_present, requires_notice};
use dosker_core::{LicenseBasis, LicenseEntry, LicenseStatus};
use dosker_image::{Attributes, DosPath, LayerDelta, Tree, TreeDir, TreeFile, fixed_timestamp};

fn tree(dirs: &[&str], files: &[(&str, &[u8])]) -> Tree {
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

fn redistributable(name: &str, spdx: &str, covers: &[&str]) -> LicenseEntry {
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

fn abandonware(name: &str) -> LicenseEntry {
    LicenseEntry {
        name: name.to_owned(),
        spdx: "NONE".to_owned(),
        holder: None,
        status: LicenseStatus::Abandonware,
        basis: Some(LicenseBasis::GoodFaith),
        evidence: Some("https://example.org/evidence".to_owned()),
        license_url: None,
        covers: Vec::new(),
        source_url: None,
        notice: None,
    }
}

#[test]
fn a_covered_app_directory_passes() {
    let tree = tree(&["APP"], &[("APP/FOO.TXT", b"x")]);
    let entries = [redistributable("APP", "MIT", &[r"C:\APP\*"])];

    assert!(coverage_misses(&tree, &entries, false).unwrap().is_empty());
}

#[test]
fn an_uncovered_app_directory_is_reported() {
    let tree = tree(&["APP"], &[("APP/FOO.TXT", b"x")]);
    let entries = [redistributable("APP", "MIT", &[r"C:\OTHER\*"])];

    assert_eq!(
        coverage_misses(&tree, &entries, false).unwrap(),
        vec![r"C:\APP".to_owned()]
    );
}

#[test]
fn a_file_glob_does_not_cover_its_directory() {
    let tree = tree(&["APP"], &[("APP/FOO.TXT", b"x")]);
    let entries = [redistributable("APP", "MIT", &[r"C:\APP\FOO.TXT"])];

    assert_eq!(
        coverage_misses(&tree, &entries, false).unwrap(),
        vec![r"C:\APP".to_owned()]
    );
}

#[test]
fn a_root_executable_needs_a_cover() {
    let tree = tree(&[], &[("GAME.EXE", b"x"), ("README.TXT", b"x")]);
    let entries = [redistributable("APP", "MIT", &[])];

    assert_eq!(
        coverage_misses(&tree, &entries, false).unwrap(),
        vec![r"C:\GAME.EXE".to_owned()]
    );
}

#[test]
fn system_files_are_exempt() {
    let tree = tree(
        &["DOS", "DOSKER", "APP"],
        &[
            ("DOS/FOO.TXT", b"x"),
            ("DOSKER/NOTICE.TXT", b"x"),
            ("KERNEL.SYS", b"x"),
            ("AUTOEXEC.BAT", b"x"),
            ("APP/FOO.TXT", b"x"),
        ],
    );
    let entries = [redistributable("APP", "MIT", &[r"C:\APP\*"])];

    assert!(coverage_misses(&tree, &entries, false).unwrap().is_empty());
}

#[test]
fn a_booter_needs_a_wildcard_cover() {
    let entries = [redistributable("BOOT", "MIT", &[])];
    assert!(matches!(
        coverage_misses(&Tree::new(), &entries, true),
        Err(RegistryError::BooterNeedsWildcard)
    ));

    let entries = [redistributable("BOOT", "MIT", &["*"])];
    assert!(
        coverage_misses(&Tree::new(), &entries, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_copyleft_entry_requires_a_notice_file() {
    let tree = tree(&["APP"], &[("APP/FOO.TXT", b"x")]);
    let entries = [redistributable("APP", "GPL-2.0-only", &[r"C:\APP\*"])];

    assert!(requires_notice(&entries));
    assert!(!notice_present(&tree));
}

#[test]
fn an_abandonware_entry_requires_a_notice_file() {
    let tree = tree(&["APP"], &[("APP/FOO.TXT", b"x")]);
    let entries = [abandonware("GAME")];

    assert!(requires_notice(&entries));
    assert!(!notice_present(&tree));
}

#[test]
fn a_redistributable_tree_needs_no_notice() {
    let tree = tree(&["APP"], &[("APP/FOO.TXT", b"x")]);
    let entries = [redistributable("APP", "MIT", &[r"C:\APP\*"])];

    assert!(!requires_notice(&entries));
    assert!(!notice_present(&tree));
}
