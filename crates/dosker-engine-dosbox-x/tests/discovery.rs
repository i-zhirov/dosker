use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dosker_engine::EngineError;
use dosker_engine_dosbox_x::DosboxXBinary;
#[cfg(unix)]
use sha2::{Digest, Sha256};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvRestore {
    values: Vec<(&'static str, Option<OsString>)>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            values: [
                "DOSKER_DOSBOX_X",
                "DOSKER_DOSBOX_X_VERSION",
                "DOSKER_DOSBOX_X_SHA256",
                "PATH",
            ]
            .into_iter()
            .map(|name| (name, std::env::var_os(name)))
            .collect(),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (name, value) in &self.values {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(unix)]
fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    let mut permissions = std::fs::metadata(&path)
        .expect("read script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("set script permissions");
    path
}

#[cfg(unix)]
fn version_script(dir: &Path) -> PathBuf {
    write_script(
        dir,
        "dosbox-x",
        "echo 'DOSBox-X version 2026.10.01 SDL2, copyright 2011-2026 The DOSBox-X Team.'",
    )
}

#[cfg(unix)]
fn lowercase_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(unix)]
#[test]
fn parses_the_version_from_the_version_line() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = version_script(dir.path());

    let binary = DosboxXBinary::from_path(&script).expect("probe dosbox-x");

    assert_eq!(binary.version_string(), "2026.10.01");
    assert_eq!(binary.path(), script);
}

#[cfg(unix)]
#[test]
fn parses_the_version_when_the_probe_exits_non_zero() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = write_script(
        dir.path(),
        "dosbox-x",
        "echo 'DOSBox-X version 2026.10.01 SDL2, copyright 2011-2026 The DOSBox-X Team.'\nexit 1",
    );

    let binary = DosboxXBinary::from_path(&script).expect("a printed version must be accepted");

    assert_eq!(binary.version_string(), "2026.10.01");
}

#[test]
fn rejects_a_missing_binary() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let path = Path::new("dosbox-x-missing-binary");

    let error = DosboxXBinary::from_path(path).expect_err("missing binary must fail");

    match error {
        EngineError::NotFound(missing) => {
            assert_eq!(missing, PathBuf::from(path));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[cfg(unix)]
#[test]
fn checksum_pin_accepts_a_matching_file() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = version_script(dir.path());
    let expected = lowercase_hex(&Sha256::digest(
        std::fs::read(&script).expect("read script"),
    ));

    unsafe {
        std::env::set_var("DOSKER_DOSBOX_X_SHA256", &expected);
    }

    let binary = DosboxXBinary::from_path(&script).expect("a matching checksum must pass");

    assert_eq!(binary.version_string(), "2026.10.01");
}

#[cfg(unix)]
#[test]
fn checksum_pin_rejects_a_modified_file() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = version_script(dir.path());
    let expected = lowercase_hex(&Sha256::digest(
        std::fs::read(&script).expect("read script"),
    ));
    std::fs::write(&script, "#!/bin/sh\necho modified\n").expect("modify script");

    unsafe {
        std::env::set_var("DOSKER_DOSBOX_X_SHA256", &expected);
    }

    let error = DosboxXBinary::from_path(&script).expect_err("a modified file must fail");

    match error {
        EngineError::Setup(message) => {
            assert!(message.contains(&expected), "message: {message}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[cfg(unix)]
#[test]
fn version_pin_rejects_a_mismatch() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = version_script(dir.path());

    unsafe {
        std::env::set_var("DOSKER_DOSBOX_X_VERSION", "2025.01.01");
    }

    let error = DosboxXBinary::from_path(&script).expect_err("a pinned mismatch must fail");

    match error {
        EngineError::Setup(message) => {
            assert!(message.contains("2025.01.01"), "message: {message}");
            assert!(message.contains("2026.10.01"), "message: {message}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[cfg(unix)]
#[test]
fn discovers_the_binary_named_by_the_environment() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = version_script(dir.path());
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).expect("create empty directory");

    unsafe {
        std::env::set_var("DOSKER_DOSBOX_X", &script);
        std::env::set_var("PATH", &empty);
    }

    let binary = DosboxXBinary::discover().expect("discover dosbox-x");

    assert_eq!(binary.path(), script);
    assert_eq!(binary.version_string(), "2026.10.01");
}
