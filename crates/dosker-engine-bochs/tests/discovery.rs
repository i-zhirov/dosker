use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dosker_engine::EngineError;
use dosker_engine_bochs::BochsBinary;
#[cfg(unix)]
use dosker_engine_bochs::Debugger;
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
                "DOSKER_BOCHS",
                "DOSKER_BOCHS_VERSION",
                "DOSKER_BOCHS_SHA256",
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
fn write_fake_bochs(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("bochs");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write fake bochs");
    let mut permissions = std::fs::metadata(&path)
        .expect("read fake bochs metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("set fake bochs permissions");
    path
}

#[cfg(unix)]
fn banner_script(dir: &Path) -> PathBuf {
    write_fake_bochs(dir, "echo 'Bochs x86 Emulator 3.1'")
}

#[cfg(unix)]
fn lowercase_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(unix)]
#[test]
fn parses_the_version_from_the_banner() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = banner_script(dir.path());

    let binary = BochsBinary::from_path(&script).expect("probe bochs");

    assert_eq!(binary.version_string(), "3.1");
    assert_eq!(binary.path(), script);
    assert!(!binary.has_internal_debugger());
}

#[cfg(unix)]
#[test]
fn parses_the_version_from_the_log_line() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = write_fake_bochs(
        dir.path(),
        "echo '00000000000i[      ] Bochs x86 Emulator 3.1'",
    );

    let binary = BochsBinary::from_path(&script).expect("probe bochs");

    assert_eq!(binary.version_string(), "3.1");
}

#[cfg(unix)]
#[test]
fn reports_the_internal_debugger_from_the_debugger_line() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = write_fake_bochs(
        dir.path(),
        "echo 'Bochs x86 Emulator 3.1'\necho \"Bochs internal debugger, type 'help' for help or 'c' to continue\"",
    );

    let binary = BochsBinary::from_path(&script).expect("probe bochs");

    assert!(binary.has_internal_debugger());
    assert_eq!(binary.debugger, Debugger::Internal);
}

#[cfg(unix)]
#[test]
fn reports_no_internal_debugger_when_the_banner_has_no_debugger_line() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = write_fake_bochs(dir.path(), "echo 'Bochs x86 Emulator 3.1'\nexit 1");

    let binary = BochsBinary::from_path(&script).expect("a printed banner must be accepted");

    assert_eq!(binary.version_string(), "3.1");
    assert_eq!(binary.debugger, Debugger::None);
}

#[cfg(unix)]
#[test]
fn probe_writes_quit_to_the_process_stdin() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = write_fake_bochs(
        dir.path(),
        "read line\nprintf '%s' \"$line\" > \"$0.stdin\"\necho 'Bochs x86 Emulator 3.1'",
    );

    let _ = BochsBinary::from_path(&script).expect("probe bochs");

    let written = std::fs::read_to_string(PathBuf::from(format!("{}.stdin", script.display())))
        .expect("read captured stdin");
    assert_eq!(written, "quit");
}

#[cfg(unix)]
#[test]
fn probe_passes_a_minimal_config() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = write_fake_bochs(
        dir.path(),
        "cp \"$4\" \"$0.config\"\necho 'Bochs x86 Emulator 3.1'",
    );

    let _ = BochsBinary::from_path(&script).expect("probe bochs");

    let config = std::fs::read_to_string(PathBuf::from(format!("{}.config", script.display())))
        .expect("read captured config");
    assert!(config.contains("cpu: model=i486dx4"), "config: {config}");
    assert!(
        config.contains("display_library: nogui"),
        "config: {config}"
    );
    assert!(config.contains("log: -"), "config: {config}");
}

#[test]
fn rejects_a_missing_binary() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let path = Path::new("bochs-missing-binary");

    let error = BochsBinary::from_path(path).expect_err("missing binary must fail");

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
    let script = banner_script(dir.path());
    let expected = lowercase_hex(&Sha256::digest(
        std::fs::read(&script).expect("read script"),
    ));

    unsafe {
        std::env::set_var("DOSKER_BOCHS_SHA256", &expected);
    }

    let binary = BochsBinary::from_path(&script).expect("a matching checksum must pass");

    assert_eq!(binary.version_string(), "3.1");
}

#[cfg(unix)]
#[test]
fn checksum_pin_rejects_a_modified_file() {
    let _lock = lock();
    let _restore = EnvRestore::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = banner_script(dir.path());
    let expected = lowercase_hex(&Sha256::digest(
        std::fs::read(&script).expect("read script"),
    ));
    std::fs::write(&script, "#!/bin/sh\necho modified\n").expect("modify script");

    unsafe {
        std::env::set_var("DOSKER_BOCHS_SHA256", &expected);
    }

    let error = BochsBinary::from_path(&script).expect_err("a modified file must fail");

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
    let script = banner_script(dir.path());

    unsafe {
        std::env::set_var("DOSKER_BOCHS_VERSION", "3.0");
    }

    let error = BochsBinary::from_path(&script).expect_err("a pinned mismatch must fail");

    match error {
        EngineError::Setup(message) => {
            assert!(message.contains("3.0"), "message: {message}");
            assert!(message.contains("3.1"), "message: {message}");
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
    let script = banner_script(dir.path());
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).expect("create empty directory");

    unsafe {
        std::env::set_var("DOSKER_BOCHS", &script);
        std::env::set_var("PATH", &empty);
    }

    let binary = BochsBinary::discover().expect("discover bochs");

    assert_eq!(binary.path(), script);
    assert_eq!(binary.version_string(), "3.1");
}
