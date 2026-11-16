use std::fs;
use std::path::Path;

use dosker_build::{BuildContext, BuildError, SourceKind};
use sha2::{Digest, Sha256};

fn write(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut text, "{byte:02x}").unwrap();
    }
    text
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn context_digest_is_stable_for_the_same_files() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.txt"), b"one");
    write(&dir.path().join("sub").join("b.txt"), b"two");

    let first = BuildContext::open(dir.path()).expect("the context opens");
    let second = BuildContext::open(dir.path()).expect("the context opens again");

    assert_eq!(first.digest(), second.digest());
    assert!(!first.digest().is_empty());
}

#[test]
fn context_digest_changes_when_a_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    write(&file, b"one");
    let before = BuildContext::open(dir.path()).unwrap().digest().to_owned();

    write(&file, b"two");
    let after = BuildContext::open(dir.path()).unwrap().digest().to_owned();

    assert_ne!(before, after);
}

#[test]
fn context_digest_ignores_the_lock_and_ignore_file() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.txt"), b"one");
    let before = BuildContext::open(dir.path()).unwrap().digest().to_owned();

    write(&dir.path().join("Doskerfile.lock"), b"version = 1\n");
    write(&dir.path().join(".doskerignore"), b"# a comment\n");
    let after = BuildContext::open(dir.path()).unwrap().digest().to_owned();

    assert_eq!(before, after);
}

#[test]
fn context_digest_skips_ignored_files() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".doskerignore"), b"ignored.txt\n");
    write(&dir.path().join("ignored.txt"), b"one");
    write(&dir.path().join("kept.txt"), b"kept");
    let before = BuildContext::open(dir.path()).unwrap().digest().to_owned();

    write(&dir.path().join("ignored.txt"), b"two");
    let after = BuildContext::open(dir.path()).unwrap().digest().to_owned();

    assert_eq!(before, after);
}

#[test]
fn resolve_source_rejects_a_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    let context = BuildContext::open(dir.path()).unwrap();

    let error = context
        .resolve_source("missing.txt")
        .expect_err("a missing source is rejected");

    assert!(matches!(error, BuildError::Context { .. }));
    assert!(error.to_string().contains("missing.txt"));
}

#[test]
fn resolve_source_rejects_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let context = BuildContext::open(dir.path()).unwrap();

    let error = context
        .resolve_source("../outside.txt")
        .expect_err("a traversal path is rejected");

    assert!(matches!(error, BuildError::Context { .. }));
    assert!(error.to_string().contains(".."));
}

#[cfg(unix)]
#[test]
fn resolve_source_rejects_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("target.txt"), b"target");
    std::os::unix::fs::symlink("target.txt", dir.path().join("link.txt")).unwrap();
    let context = BuildContext::open(dir.path()).unwrap();

    let error = context
        .resolve_source("link.txt")
        .expect_err("a symlink source is rejected");

    assert!(matches!(error, BuildError::Context { .. }));
    assert!(error.to_string().contains("link.txt"));
}

#[test]
fn resolve_source_rejects_a_path_outside_the_context() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("outside.txt");
    write(&outside_file, b"outside");
    let context = BuildContext::open(dir.path()).unwrap();

    let error = context
        .resolve_source(&path_string(&outside_file))
        .expect_err("an absolute path outside the context is rejected");

    assert!(matches!(error, BuildError::Context { .. }));
}

#[test]
fn resolve_source_accepts_an_absolute_path_inside_the_context() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("src").join("app.txt");
    write(&file, b"app");
    let context = BuildContext::open(dir.path()).unwrap();

    let source = context
        .resolve_source(&path_string(&file))
        .expect("an absolute path inside the context is accepted");

    assert_eq!(source.relative, "src/app.txt");
    assert_eq!(source.kind, SourceKind::File);
    assert_eq!(source.host_path, file);
}

#[test]
fn resolve_source_rejects_a_glob() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("app.txt"), b"app");
    let context = BuildContext::open(dir.path()).unwrap();

    let error = context
        .resolve_source("*.txt")
        .expect_err("a glob source is rejected");

    assert!(matches!(error, BuildError::Context { .. }));
}

#[test]
fn resolve_source_rejects_an_ignored_source() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".doskerignore"), b"secret.txt\n");
    write(&dir.path().join("secret.txt"), b"secret");
    let context = BuildContext::open(dir.path()).unwrap();

    let error = context
        .resolve_source("secret.txt")
        .expect_err("an ignored source is rejected");

    assert!(matches!(error, BuildError::Context { .. }));
    assert!(error.to_string().contains("secret.txt"));
}

#[test]
fn read_source_files_walks_a_directory_and_skips_ignored_children() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".doskerignore"), b"*.log\n");
    write(&dir.path().join("src").join("keep.txt"), b"keep");
    write(&dir.path().join("src").join("skip.log"), b"skip");
    write(
        &dir.path().join("src").join("nested").join("more.txt"),
        b"more",
    );
    let context = BuildContext::open(dir.path()).unwrap();
    let source = context.resolve_source("src").unwrap();

    let files = context.read_source_files(&source).unwrap();

    let relative: Vec<&str> = files.iter().map(|file| file.relative.as_str()).collect();
    assert_eq!(relative, vec!["src/keep.txt", "src/nested/more.txt"]);
    assert_eq!(files[0].bytes, b"keep");
    assert_eq!(files[1].bytes, b"more");
}

#[test]
fn read_source_files_includes_a_hidden_file() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".hidden"), b"hidden");
    write(&dir.path().join("visible.txt"), b"visible");
    let context = BuildContext::open(dir.path()).unwrap();
    let source = context.resolve_source(".").unwrap();

    let files = context.read_source_files(&source).unwrap();

    let relative: Vec<&str> = files.iter().map(|file| file.relative.as_str()).collect();
    assert!(relative.contains(&".hidden"));
    assert!(relative.contains(&"visible.txt"));
}

#[cfg(unix)]
#[test]
fn read_source_files_rejects_a_symlink_child() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("src").join("file.txt"), b"file");
    std::os::unix::fs::symlink("file.txt", dir.path().join("src").join("link.txt")).unwrap();
    let context = BuildContext::open(dir.path()).unwrap();
    let source = context.resolve_source("src").unwrap();

    let error = context
        .read_source_files(&source)
        .expect_err("a symlink child is rejected");

    assert!(matches!(error, BuildError::Context { .. }));
    assert!(error.to_string().contains("link.txt"));
}

#[test]
fn source_hashes_include_the_context_and_file_content() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("app.bin"), b"hello");
    let context = BuildContext::open(dir.path()).unwrap();
    let source = context.resolve_source("app.bin").unwrap();

    let hashes = context.source_hashes(&[source]).unwrap();

    assert_eq!(hashes[0], format!("context:{}", context.digest()));
    assert!(hashes.contains(&format!("file:app.bin:{}", sha256_hex(b"hello"))));
}

#[test]
fn source_hashes_skip_root_context_metadata() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("Doskerfile"), b"FROM test:latest\n");
    write(&dir.path().join("Doskerfile.lock"), b"version = 1\n");
    write(&dir.path().join(".doskerignore"), b"# comment\n");
    write(&dir.path().join("app.txt"), b"app");
    let context = BuildContext::open(dir.path()).unwrap();
    let source = context.resolve_source(".").unwrap();

    let hashes = context.source_hashes(&[source]).unwrap();

    assert!(hashes.iter().any(|hash| hash.starts_with("dir:")));
    assert!(hashes.iter().all(|hash| !hash.contains("Doskerfile")));
    assert!(hashes.iter().all(|hash| !hash.contains(".doskerignore")));
    assert!(hashes.contains(&format!("file:app.txt:{}", sha256_hex(b"app"))));
}

#[test]
fn context_digest_includes_a_gitignored_file() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".gitignore"), b"secret.txt\n");
    write(&dir.path().join("secret.txt"), b"one");
    let before = BuildContext::open(dir.path()).unwrap().digest().to_owned();

    write(&dir.path().join("secret.txt"), b"two");
    let after = BuildContext::open(dir.path()).unwrap().digest().to_owned();

    assert_ne!(before, after);
}
