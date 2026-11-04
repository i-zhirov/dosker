use std::fs;
use std::path::Path;

use dosker_platform::{CopyOutcome, PlatformError, copy_full, copy_reflink};

fn write_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directory");
    }
    fs::write(path, bytes).expect("write fixture file");
}

fn read_file(path: &Path) -> Vec<u8> {
    fs::read(path).expect("read copied file")
}

#[test]
fn copy_reflink_copies_the_contents() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("source.img");
    let destination = dir.path().join("destination.img");
    let bytes = b"private disk contents";
    write_file(&source, bytes);

    let outcome = copy_reflink(&source, &destination).expect("copy the disk");

    assert!(matches!(
        outcome,
        CopyOutcome::Reflink | CopyOutcome::FullCopy
    ));
    assert_eq!(read_file(&destination), bytes);
}

#[test]
fn copy_reflink_replaces_an_existing_destination() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("source.img");
    let destination = dir.path().join("destination.img");
    let bytes = b"short";
    write_file(&source, bytes);
    write_file(&destination, b"a much longer old destination");

    copy_reflink(&source, &destination).expect("replace the destination");

    assert_eq!(read_file(&destination), bytes);
}

#[test]
fn copy_reflink_leaves_the_source_unchanged() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("source.img");
    let destination = dir.path().join("destination.img");
    let bytes = b"read-only source bytes";
    write_file(&source, bytes);
    #[cfg(unix)]
    set_mode(&source, 0o444);
    #[cfg(unix)]
    let original_mode = mode(&source);

    copy_reflink(&source, &destination).expect("copy the disk");

    assert_eq!(read_file(&source), bytes);
    #[cfg(unix)]
    assert_eq!(mode(&source), original_mode);
}

#[test]
fn copy_full_reports_source_errors_at_the_source_path() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("missing.img");
    let destination = dir.path().join("destination.img");

    let error = copy_full(&source, &destination).expect_err("the source is missing");

    match error {
        PlatformError::Io { path, .. } => assert_eq!(path, source),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn copy_full_copies_the_contents() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("source.img");
    let destination = dir.path().join("destination.img");
    let bytes = b"full copy contents";
    write_file(&source, bytes);

    copy_full(&source, &destination).expect("copy the disk");

    assert_eq!(read_file(&destination), bytes);
}

#[test]
fn copy_full_replaces_an_existing_destination() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("source.img");
    let destination = dir.path().join("destination.img");
    let bytes = b"small";
    write_file(&source, bytes);
    write_file(&destination, b"an older destination that is longer");

    copy_full(&source, &destination).expect("replace the destination");

    assert_eq!(read_file(&destination), bytes);
}

#[test]
fn copy_creates_the_destination_parent() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("source.img");
    let bytes = b"nested destination contents";
    write_file(&source, bytes);

    let reflink_destination = dir.path().join("missing").join("reflink.img");
    copy_reflink(&source, &reflink_destination).expect("copy into a missing directory");

    let full_destination = dir.path().join("absent").join("full.img");
    copy_full(&source, &full_destination).expect("copy into a missing directory");

    assert_eq!(read_file(&reflink_destination), bytes);
    assert_eq!(read_file(&full_destination), bytes);
}

#[cfg(unix)]
#[test]
fn copy_makes_the_destination_writable() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("source.img");
    let bytes = b"read-only source";
    write_file(&source, bytes);
    set_mode(&source, 0o444);

    let reflink_destination = dir.path().join("reflink.img");
    copy_reflink(&source, &reflink_destination).expect("copy the disk");
    assert_eq!(mode(&reflink_destination), 0o600);

    let full_destination = dir.path().join("full.img");
    copy_full(&source, &full_destination).expect("copy the disk");
    assert_eq!(mode(&full_destination), 0o600);
}

#[cfg(windows)]
#[test]
fn copy_clears_the_windows_read_only_attribute() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("source.img");
    let destination = dir.path().join("destination.img");
    let bytes = b"read-only source bytes";
    write_file(&source, bytes);

    let mut permissions = fs::metadata(&source)
        .expect("read source metadata")
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&source, permissions).expect("mark the source read-only");

    copy_full(&source, &destination).expect("copy the disk");

    assert_eq!(read_file(&destination), bytes);
    assert!(
        !fs::metadata(&destination)
            .expect("read destination metadata")
            .permissions()
            .readonly(),
        "the private disk must not inherit the read-only attribute"
    );
    fs::OpenOptions::new()
        .append(true)
        .open(&destination)
        .expect("the private disk must be writable");
}

#[cfg(windows)]
#[test]
fn copy_reflink_uses_full_copy_on_windows() {
    let dir = tempfile::tempdir().expect("create temporary directory");
    let source = dir.path().join("source.img");
    let destination = dir.path().join("destination.img");
    let bytes = b"windows copy contents";
    write_file(&source, bytes);

    let outcome = copy_reflink(&source, &destination).expect("copy the disk");

    assert_eq!(outcome, CopyOutcome::FullCopy);
    assert_eq!(read_file(&destination), bytes);
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set file mode");
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .expect("read file metadata")
        .permissions()
        .mode()
        & 0o777
}
