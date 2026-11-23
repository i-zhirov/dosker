use std::fs;
use std::path::{Path, PathBuf};

use dosker_build::{BuildContext, BuildError, CopyPlan, plan_copy};
use dosker_image::{
    Attributes, DosPath, LayerDelta, Tree, TreeDir, TreeFile, TreeNode, fixed_timestamp,
};

fn workdir() -> DosPath {
    DosPath::parse("").expect("the root parses")
}

fn write(root: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, bytes).unwrap();
    path
}

fn open(root: &Path) -> BuildContext {
    BuildContext::open(root).expect("the context opens")
}

fn tree_with_dirs(dirs: &[&str]) -> Tree {
    let mut delta = LayerDelta::new();
    for dir in dirs {
        delta.add_dir(TreeDir {
            path: DosPath::parse(dir).expect("the directory path parses"),
            attributes: Attributes::default(),
            timestamp: fixed_timestamp(),
        });
    }
    let mut tree = Tree::new();
    tree.apply_delta(&delta).expect("the tree applies");
    tree
}

fn tree_with_files(files: &[(&str, &[u8])]) -> Tree {
    let mut delta = LayerDelta::new();
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

fn dos(path: &str) -> DosPath {
    DosPath::parse(path).expect("the DOS path parses")
}

fn source(text: &str) -> String {
    text.to_owned()
}

fn plan(
    context: &BuildContext,
    tree: &Tree,
    sources: &[String],
    destination: &str,
) -> Result<CopyPlan, BuildError> {
    plan_copy(context, tree, &workdir(), sources, destination, 7)
}

fn assert_copy_error(error: BuildError, needle: &str) {
    match error {
        BuildError::Instruction {
            line,
            instruction,
            reason,
        } => {
            assert_eq!(line, 7);
            assert_eq!(instruction, "COPY");
            assert!(
                reason.contains(needle),
                "{reason:?} does not name {needle:?}"
            );
        }
        other => panic!("expected a COPY instruction error, got {other:?}"),
    }
}

#[test]
fn copy_file_to_a_file_destination() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "hello.txt", b"hello");
    let context = open(dir.path());
    let plan = plan(
        &context,
        &Tree::new(),
        &[source("hello.txt")],
        r"C:\APP\HELLO.TXT",
    )
    .expect("the copy plans");

    assert_eq!(plan.normalized_sources, vec!["hello.txt".to_owned()]);
    assert_eq!(plan.normalized_destination, dos(r"C:\APP\HELLO.TXT"));
    assert_eq!(
        plan.normalized_instruction,
        r#"COPY hello.txt "C:\\APP\\HELLO.TXT""#
    );
    assert_eq!(plan.delta.files.len(), 1);
    assert_eq!(plan.delta.files[0].path, dos(r"C:\APP\HELLO.TXT"));
    assert_eq!(plan.delta.files[0].bytes, b"hello");
    assert!(plan.delta.dirs.iter().any(|dir| dir.path == dos(r"C:\APP")));
    assert_eq!(
        plan.input_hashes[0],
        format!("context:{}", context.digest())
    );
    assert!(
        plan.input_hashes
            .iter()
            .any(|hash| hash.starts_with("file:hello.txt:"))
    );
}

#[test]
fn copy_accepts_the_notice_path() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notice.txt", b"notice");
    let plan = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("notice.txt")],
        r"C:\DOSKER\NOTICE.TXT",
    )
    .expect("the notice copy plans");

    assert_eq!(plan.delta.files.len(), 1);
    assert_eq!(plan.delta.files[0].path, dos(r"C:\DOSKER\NOTICE.TXT"));
    assert!(
        plan.delta
            .dirs
            .iter()
            .any(|dir| dir.path == dos(r"C:\DOSKER"))
    );
}

#[test]
fn copy_rejects_a_dosker_directory_destination() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notice.txt", b"notice");
    let error = match plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("notice.txt")],
        r"C:\DOSKER\",
    ) {
        Ok(_) => panic!("the copy should be rejected"),
        Err(error) => error,
    };

    assert_copy_error(error, "reserved DOSKER");
}

#[test]
fn copy_rejects_a_dosker_multi_source_destination() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", b"a");
    write(dir.path(), "b.txt", b"b");
    let error = match plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("a.txt"), source("b.txt")],
        r"C:\DOSKER\",
    ) {
        Ok(_) => panic!("the copy should be rejected"),
        Err(error) => error,
    };

    assert_copy_error(error, "reserved DOSKER");
}

#[test]
fn copy_rejects_another_dosker_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notice.txt", b"notice");
    let error = match plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("notice.txt")],
        r"C:\DOSKER\OTHER.TXT",
    ) {
        Ok(_) => panic!("the copy should be rejected"),
        Err(error) => error,
    };

    assert_copy_error(error, "reserved DOSKER");
}

#[test]
fn copy_file_to_a_directory_destination_uses_the_file_name() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "hello.txt", b"hello");
    let plan = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("hello.txt")],
        r"C:\APP\",
    )
    .expect("the copy plans");

    assert_eq!(plan.normalized_destination, dos(r"C:\APP"));
    assert_eq!(plan.normalized_instruction, r#"COPY hello.txt "C:\\APP\\""#);
    assert_eq!(plan.delta.files.len(), 1);
    assert_eq!(plan.delta.files[0].path, dos(r"C:\APP\HELLO.TXT"));
}

#[test]
fn copy_directory_copies_its_contents() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/one.txt", b"one");
    write(dir.path(), "src/sub/two.bin", b"two");
    let plan =
        plan(&open(dir.path()), &Tree::new(), &[source("src")], r"C:\APP").expect("the copy plans");

    let files: Vec<_> = plan
        .delta
        .files
        .iter()
        .map(|file| (file.path.clone(), file.bytes.clone()))
        .collect();
    assert_eq!(
        files,
        vec![
            (dos(r"C:\APP\ONE.TXT"), b"one".to_vec()),
            (dos(r"C:\APP\SUB\TWO.BIN"), b"two".to_vec()),
        ]
    );
    let dirs: Vec<_> = plan.delta.dirs.iter().map(|dir| dir.path.clone()).collect();
    assert_eq!(dirs, vec![dos(r"C:\APP"), dos(r"C:\APP\SUB")]);
    assert!(!dirs.contains(&dos(r"C:\APP\SRC")));
}

#[test]
fn copy_merges_an_existing_directory() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/new.txt", b"new");
    let tree = tree_with_dirs(&[r"C:\APP"]);
    let plan = plan(&open(dir.path()), &tree, &[source("src")], r"C:\APP").expect("the copy plans");

    assert!(plan.delta.dirs.is_empty());
    assert_eq!(plan.delta.files.len(), 1);
    assert_eq!(plan.delta.files[0].path, dos(r"C:\APP\NEW.TXT"));
    assert!(matches!(tree.get(&dos(r"C:\APP")), Some(TreeNode::Dir(_))));
}

#[test]
fn copy_replaces_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", b"new");
    let tree = tree_with_files(&[(r"C:\APP\A.TXT", b"old")]);
    let plan =
        plan(&open(dir.path()), &tree, &[source("a.txt")], r"C:\APP\").expect("the copy plans");

    assert_eq!(plan.delta.files.len(), 1);
    assert_eq!(plan.delta.files[0].path, dos(r"C:\APP\A.TXT"));
    assert_eq!(plan.delta.files[0].bytes, b"new");
}

#[test]
fn copy_sets_archive_and_the_fixed_timestamp() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/app.txt", b"app");
    write(dir.path(), "src/data/one.bin", b"one");
    let plan =
        plan(&open(dir.path()), &Tree::new(), &[source("src")], r"C:\APP").expect("the copy plans");

    for file in &plan.delta.files {
        assert_eq!(
            file.attributes,
            Attributes {
                archive: true,
                ..Attributes::default()
            }
        );
        assert_eq!(file.timestamp, fixed_timestamp());
    }
    for dir in &plan.delta.dirs {
        assert_eq!(dir.attributes, Attributes::default());
        assert_eq!(dir.timestamp, fixed_timestamp());
    }
}

#[test]
fn copy_rejects_multiple_sources_without_a_directory_destination() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "one.txt", b"one");
    write(dir.path(), "two.txt", b"two");
    let error = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("one.txt"), source("two.txt")],
        r"C:\APP",
    )
    .expect_err("multiple sources need a directory destination");

    assert_copy_error(error, r"C:\APP");
}

#[test]
fn copy_rejects_a_destination_under_dosker() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "one.txt", b"one");
    let error = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("one.txt")],
        r"C:\DOSKER\ONE.TXT",
    )
    .expect_err("the reserved directory is rejected");

    assert_copy_error(error, r"C:\DOSKER");
}

#[test]
fn copy_rejects_an_invalid_source_name() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "LONGFILENAME.TXT", b"long");
    let error = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("LONGFILENAME.TXT")],
        r"C:\APP\",
    )
    .expect_err("a long source name is rejected");

    assert_copy_error(error, "LONGFILENAME.TXT");
}

#[test]
fn copy_rejects_an_invalid_destination_name() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "one.txt", b"one");
    let error = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("one.txt")],
        r"C:\APP\TOOLONGNAME.TXT",
    )
    .expect_err("a long destination name is rejected");

    assert_copy_error(error, "TOOLONGNAME.TXT");
}

#[cfg(unix)]
#[test]
fn copy_rejects_a_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "target.txt", b"target");
    symlink(dir.path().join("target.txt"), dir.path().join("link.txt")).unwrap();
    let error = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("link.txt")],
        r"C:\APP\",
    )
    .expect_err("a symlink source is rejected");

    assert_copy_error(error, "link.txt");
}

#[test]
fn copy_rejects_a_traversal_path() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "one.txt", b"one");
    let error = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("../secret.txt")],
        r"C:\APP\",
    )
    .expect_err("a traversal source is rejected");

    assert_copy_error(error, "../secret.txt");
}

#[test]
fn copy_rejects_a_source_outside_the_context() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = write(outside.path(), "secret.txt", b"secret");
    let error = plan(
        &open(dir.path()),
        &Tree::new(),
        &[secret.to_string_lossy().into_owned()],
        r"C:\APP\",
    )
    .expect_err("a source outside the context is rejected");

    assert_copy_error(error, "secret.txt");
}

#[test]
fn copy_rejects_two_sources_at_one_destination() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "one/A.TXT", b"one");
    write(dir.path(), "two/A.TXT", b"two");
    let error = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("one/A.TXT"), source("two/A.TXT")],
        r"C:\DEST\",
    )
    .expect_err("two sources may not share a destination");

    assert_copy_error(error, r"C:\DEST\A.TXT");
}

#[test]
fn copy_into_a_new_directory_adds_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "one.txt", b"one");
    let plan = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("one.txt")],
        r"C:\NEW\SUB\",
    )
    .expect("the copy plans");

    let dirs: Vec<_> = plan.delta.dirs.iter().map(|dir| dir.path.clone()).collect();
    assert_eq!(dirs, vec![dos(r"C:\NEW"), dos(r"C:\NEW\SUB")]);
    assert_eq!(plan.delta.files[0].path, dos(r"C:\NEW\SUB\ONE.TXT"));
}

#[test]
fn copy_to_the_root_does_not_add_a_root_entry() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", b"a");
    let plan =
        plan(&open(dir.path()), &Tree::new(), &[source("a.txt")], r"C:\").expect("the copy plans");

    assert!(plan.delta.dirs.is_empty());
    assert_eq!(plan.delta.files.len(), 1);
    assert_eq!(plan.delta.files[0].path, dos(r"A.TXT"));

    let mut tree = Tree::new();
    tree.apply_delta(&plan.delta)
        .expect("the root destination applies");
    assert!(matches!(tree.get(&dos(r"A.TXT")), Some(TreeNode::File(_))));
}

#[test]
fn copy_rejects_a_target_under_dosker() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/DOSKER/CONFIG.INI", b"config");
    let error = plan(&open(dir.path()), &Tree::new(), &[source("src")], r"C:\")
        .expect_err("a target under DOSKER is rejected");

    assert_copy_error(error, r"C:\DOSKER");
}

#[test]
fn copy_preserves_an_empty_source_directory() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/one.txt", b"one");
    fs::create_dir_all(dir.path().join("src/EMPTY")).unwrap();
    let plan =
        plan(&open(dir.path()), &Tree::new(), &[source("src")], r"C:\APP").expect("the copy plans");

    let dirs: Vec<_> = plan.delta.dirs.iter().map(|dir| dir.path.clone()).collect();
    assert_eq!(dirs, vec![dos(r"C:\APP"), dos(r"C:\APP\EMPTY")]);
}

#[test]
fn copy_input_hashes_change_for_an_empty_directory() {
    let plain = tempfile::tempdir().unwrap();
    write(plain.path(), "src/one.txt", b"one");

    let with_empty = tempfile::tempdir().unwrap();
    write(with_empty.path(), "src/one.txt", b"one");
    fs::create_dir_all(with_empty.path().join("src/EMPTY")).unwrap();

    let first = plan(
        &open(plain.path()),
        &Tree::new(),
        &[source("src")],
        r"C:\APP",
    )
    .expect("the first copy plans");
    let second = plan(
        &open(with_empty.path()),
        &Tree::new(),
        &[source("src")],
        r"C:\APP",
    )
    .expect("the second copy plans");

    assert_ne!(first.input_hashes, second.input_hashes);
}

#[test]
fn copy_empty_directory_into_a_new_nested_destination_adds_the_ancestors() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    let plan = plan(
        &open(dir.path()),
        &Tree::new(),
        &[source("src")],
        r"C:\NEW\SUB\",
    )
    .expect("the copy plans");

    let dirs: Vec<_> = plan.delta.dirs.iter().map(|dir| dir.path.clone()).collect();
    assert_eq!(dirs, vec![dos(r"C:\NEW"), dos(r"C:\NEW\SUB")]);
    assert!(plan.delta.files.is_empty());
}

#[test]
fn copy_rejects_a_file_at_an_ancestor_path() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    let tree = tree_with_files(&[(r"C:\NEW", b"file")]);
    let error = plan(&open(dir.path()), &tree, &[source("src")], r"C:\NEW\SUB\")
        .expect_err("a file at the ancestor path is rejected");

    assert_copy_error(error, r"C:\NEW");
}

#[test]
fn copy_resolves_a_relative_destination_against_the_workdir() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "one.txt", b"one");
    let plan = plan_copy(
        &open(dir.path()),
        &Tree::new(),
        &dos(r"C:\PROJECT"),
        &[source("one.txt")],
        r"APP\",
        7,
    )
    .expect("the copy plans");

    assert_eq!(plan.normalized_destination, dos(r"C:\PROJECT\APP"));
    assert_eq!(plan.delta.files[0].path, dos(r"C:\PROJECT\APP\ONE.TXT"));
}

#[test]
fn copy_dot_skips_root_metadata() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Doskerfile", b"FROM base:latest\n");
    write(dir.path(), "Doskerfile.lock", b"version = 1\n");
    write(dir.path(), ".doskerignore", b"");
    write(dir.path(), "app.txt", b"app");
    let plan =
        plan(&open(dir.path()), &Tree::new(), &[source(".")], r"C:\APP\").expect("the copy plans");

    assert_eq!(plan.normalized_sources, vec![".".to_owned()]);
    let files: Vec<_> = plan
        .delta
        .files
        .iter()
        .map(|file| file.path.as_str().to_owned())
        .collect();
    assert_eq!(files, vec!["APP/APP.TXT".to_owned()]);
}
