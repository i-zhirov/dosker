use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use dosker_platform::fs::{ensure_dir_mode, write_atomic};

const FIRST: &[u8] = br#"{"payload":"first"}"#;
const SECOND: &[u8] = br#"{"payload":"second","padding":"a longer valid payload"}"#;

#[test]
fn write_atomic_creates_file_with_contents() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("state.json");

    write_atomic(&target, b"exact bytes").unwrap();

    assert_eq!(fs::read(&target).unwrap(), b"exact bytes");
}

#[test]
fn write_atomic_replaces_existing_contents() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("state.json");

    write_atomic(&target, b"a much longer previous payload").unwrap();
    write_atomic(&target, b"short").unwrap();

    assert_eq!(fs::read(&target).unwrap(), b"short");
}

#[test]
fn write_atomic_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("state.json");

    write_atomic(&target, b"contents").unwrap();

    let mut names: Vec<OsString> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();

    assert_eq!(names, vec![OsString::from("state.json")]);
}

#[test]
fn concurrent_readers_never_observe_partial_content() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("state.json");
    write_atomic(&target, FIRST).unwrap();

    let first: serde_json::Value = serde_json::from_slice(FIRST).unwrap();
    let second: serde_json::Value = serde_json::from_slice(SECOND).unwrap();

    let done = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(2));

    let writer_path = target.clone();
    let writer_done = Arc::clone(&done);
    let writer_barrier = Arc::clone(&barrier);
    let writer = thread::spawn(move || {
        let _finished = Finished(writer_done);
        writer_barrier.wait();

        for round in 0..100 {
            let payload = if round % 2 == 0 { SECOND } else { FIRST };
            write_atomic(&writer_path, payload).unwrap();
        }
    });

    let reader_path = target.clone();
    let reader_done = Arc::clone(&done);
    let reader_barrier = Arc::clone(&barrier);
    let reader = thread::spawn(move || {
        reader_barrier.wait();
        let mut reads = 0usize;

        while !reader_done.load(Ordering::Acquire) {
            match fs::read(&reader_path) {
                Ok(bytes) => {
                    let value: serde_json::Value = serde_json::from_slice(&bytes)
                        .unwrap_or_else(|error| panic!("a reader saw partial content: {error}"));
                    assert!(
                        value == first || value == second,
                        "a reader saw an unexpected payload: {value}"
                    );
                    reads += 1;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                // Windows can briefly deny a read while the target is being
                // replaced. A retry is a successful read, not corruption.
                #[cfg(windows)]
                Err(error) if error.raw_os_error() == Some(32) => {}
                Err(error) => panic!("a reader failed: {error}"),
            }
        }

        reads
    });

    writer.join().unwrap();
    let reads = reader.join().unwrap();

    assert!(reads > 0, "the reader did not observe the file");
}

struct Finished(Arc<AtomicBool>);

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(unix)]
#[test]
fn ensure_dir_mode_sets_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a").join("b");

    ensure_dir_mode(&nested, 0o700).unwrap();

    let mode = fs::metadata(&nested).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[cfg(unix)]
#[test]
fn ensure_dir_mode_rejects_a_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = dir.path().join("link");
    symlink(&target, &link).unwrap();

    let error = ensure_dir_mode(&link, 0o700).unwrap_err();
    assert!(matches!(error, dosker_platform::PlatformError::Io { .. }));
}

#[test]
fn ensure_dir_mode_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a").join("b");

    ensure_dir_mode(&nested, 0o700).unwrap();

    #[cfg(unix)]
    let before = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&nested).unwrap().permissions().mode()
    };

    ensure_dir_mode(&nested, 0o700).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let after = fs::metadata(&nested).unwrap().permissions().mode();
        assert_eq!(after, before);
    }

    assert!(Path::new(&nested).is_dir());
}
