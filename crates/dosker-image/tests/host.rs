use std::fs;
use std::path::{Path, PathBuf};

use dosker_image::{
    DosPath, TreeNode, fixed_timestamp, read_host_tree, validate_host_tree, write_host_tree,
};

fn write_file(root: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create the parent directory");
    }
    fs::write(&path, bytes).expect("write the file");
    path
}

fn write_dir(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(&path).expect("create the directory");
    path
}

fn valid_directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "APP/MAIN.EXE", b"main bytes");
    write_file(dir.path(), "README.TXT", b"read me");
    dir
}

fn path(value: &str) -> DosPath {
    DosPath::parse(value).unwrap()
}

#[test]
fn a_valid_host_directory_becomes_a_tree() {
    let dir = valid_directory();
    let tree = read_host_tree(dir.path()).expect("a valid host directory reads");

    let paths: Vec<&str> = tree.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(paths, ["APP", "APP/MAIN.EXE", "README.TXT"]);
    for (_, node) in tree.iter() {
        assert_eq!(node.timestamp(), fixed_timestamp());
    }
    match tree.get(&path("APP/MAIN.EXE")) {
        Some(TreeNode::File(file)) => assert_eq!(file.bytes, b"main bytes"),
        other => panic!("expected APP/MAIN.EXE to be a file, got {other:?}"),
    }
    match tree.get(&path("README.TXT")) {
        Some(TreeNode::File(file)) => assert_eq!(file.bytes, b"read me"),
        other => panic!("expected README.TXT to be a file, got {other:?}"),
    }
    assert!(matches!(tree.get(&path("APP")), Some(TreeNode::Dir(_))));
}

#[test]
fn a_valid_host_directory_passes_validation() {
    let dir = valid_directory();
    validate_host_tree(dir.path()).expect("a valid host directory passes validation");
}

#[test]
fn a_name_longer_than_eight_characters_is_rejected_and_names_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let file = write_file(dir.path(), "TOOLONGNAME.TXT", b"x");
    let error = validate_host_tree(dir.path())
        .expect_err("a long name is rejected")
        .to_string();

    assert!(error.contains(&file.display().to_string()), "{error}");
    assert!(error.contains("TOOLONGNAME.TXT"), "{error}");
}

#[test]
fn an_extension_longer_than_three_characters_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "MAIN.LONG", b"x");
    let error = validate_host_tree(dir.path())
        .expect_err("a long extension is rejected")
        .to_string();

    assert!(error.contains("MAIN.LONG"), "{error}");
}

#[test]
fn a_name_with_an_invalid_character_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "BAD+NAME.TXT", b"x");
    let error = validate_host_tree(dir.path())
        .expect_err("an invalid character is rejected")
        .to_string();

    assert!(error.contains("BAD+NAME.TXT"), "{error}");
}

#[test]
fn a_leading_period_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), ".HIDDEN", b"x");
    let error = validate_host_tree(dir.path())
        .expect_err("a leading period is rejected")
        .to_string();

    assert!(error.contains(".HIDDEN"), "{error}");
}

#[cfg(unix)]
#[test]
fn a_symlinked_file_is_rejected() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = write_file(dir.path(), "README.TXT", b"read me");
    let link = dir.path().join("LINK.TXT");
    symlink(&target, &link).unwrap();
    let error = validate_host_tree(dir.path())
        .expect_err("a symlinked file is rejected")
        .to_string();

    assert!(error.contains(&link.display().to_string()), "{error}");
}

#[cfg(unix)]
#[test]
fn a_symlinked_directory_is_rejected() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = write_dir(dir.path(), "DATA");
    write_file(&target, "FILE.TXT", b"data");
    let link = dir.path().join("LINK");
    symlink(&target, &link).unwrap();
    let error = validate_host_tree(dir.path())
        .expect_err("a symlinked directory is rejected")
        .to_string();

    assert!(error.contains(&link.display().to_string()), "{error}");
}

#[cfg(unix)]
#[test]
fn a_symlinked_root_is_rejected() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = write_dir(dir.path(), "DATA");
    write_file(&target, "FILE.TXT", b"data");
    let link = dir.path().join("LINK");
    symlink(&target, &link).unwrap();
    let error = validate_host_tree(&link)
        .expect_err("a symlinked root is rejected")
        .to_string();

    assert!(error.contains(&link.display().to_string()), "{error}");
}

#[test]
fn a_non_directory_source_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let file = write_file(dir.path(), "PLAIN.TXT", b"x");
    let error = validate_host_tree(&file)
        .expect_err("a regular file is not a host directory")
        .to_string();

    assert!(error.contains(&file.display().to_string()), "{error}");
}

#[test]
fn a_missing_source_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("MISSING");
    let error = validate_host_tree(&missing)
        .expect_err("a missing source is rejected")
        .to_string();

    assert!(error.contains(&missing.display().to_string()), "{error}");
}

#[cfg(target_os = "linux")]
#[test]
fn names_are_compared_after_normalization() {
    let dir = tempfile::tempdir().unwrap();
    let upper = write_file(dir.path(), "FOO.TXT", b"upper");
    let lower = write_file(dir.path(), "foo.txt", b"lower");
    let error = validate_host_tree(dir.path())
        .expect_err("two names that normalize to one DOS path are rejected")
        .to_string();

    assert!(error.contains(&upper.display().to_string()), "{error}");
    assert!(error.contains(&lower.display().to_string()), "{error}");
    assert!(error.contains("duplicate"), "{error}");
}

#[test]
fn write_host_tree_mirrors_the_tree() {
    let source = valid_directory();
    let tree = read_host_tree(source.path()).expect("the source reads");

    let destination = tempfile::tempdir().unwrap();
    write_file(destination.path(), "STALE.TXT", b"stale");
    write_file(destination.path(), "STALEDIR/OLD.TXT", b"old");

    write_host_tree(destination.path(), &tree).expect("the tree is written");

    assert!(!destination.path().join("STALE.TXT").exists());
    assert!(!destination.path().join("STALEDIR").exists());
    assert!(destination.path().join("README.TXT").is_file());
    assert!(destination.path().join("APP").is_dir());
    assert!(destination.path().join("APP/MAIN.EXE").is_file());
}

#[test]
fn write_host_tree_writes_the_exact_bytes() {
    let source = tempfile::tempdir().unwrap();
    write_file(source.path(), "APP/MAIN.EXE", &[0, 1, 2, 255, 254]);
    let tree = read_host_tree(source.path()).expect("the source reads");

    let destination = tempfile::tempdir().unwrap();
    write_host_tree(destination.path(), &tree).expect("the tree is written");

    assert_eq!(
        fs::read(destination.path().join("APP/MAIN.EXE")).unwrap(),
        [0, 1, 2, 255, 254]
    );
}

#[cfg(unix)]
#[test]
fn write_host_tree_replaces_a_stale_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let source = tempfile::tempdir().unwrap();
    write_file(source.path(), "README.TXT", b"tree bytes");
    let tree = read_host_tree(source.path()).expect("the source reads");

    let outside = tempfile::tempdir().unwrap();
    let target = write_file(outside.path(), "TARGET.TXT", b"target bytes");

    let destination = tempfile::tempdir().unwrap();
    symlink(&target, destination.path().join("README.TXT")).unwrap();

    write_host_tree(destination.path(), &tree).expect("the tree is written");

    let written = destination.path().join("README.TXT");
    let metadata = fs::symlink_metadata(&written).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(fs::read(&written).unwrap(), b"tree bytes");
    assert_eq!(fs::read(&target).unwrap(), b"target bytes");
}

#[test]
fn write_host_tree_replaces_a_file_where_the_tree_has_a_directory() {
    let source = tempfile::tempdir().unwrap();
    write_file(source.path(), "APP/MAIN.EXE", b"main bytes");
    let tree = read_host_tree(source.path()).expect("the source reads");

    let destination = tempfile::tempdir().unwrap();
    write_file(destination.path(), "APP", b"not a directory");

    write_host_tree(destination.path(), &tree).expect("the tree is written");

    assert!(
        fs::symlink_metadata(destination.path().join("APP"))
            .unwrap()
            .is_dir()
    );
    assert_eq!(
        fs::read(destination.path().join("APP/MAIN.EXE")).unwrap(),
        b"main bytes"
    );
}
