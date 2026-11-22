use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use dosker_engine::{EngineError, EngineSource};
use dosker_engine_qemu::{QemuBinary, QemuVersion, engine_source};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    qemu: Option<OsString>,
    path: Option<OsString>,
    current_dir: PathBuf,
}

impl EnvGuard {
    fn capture() -> Self {
        Self {
            qemu: std::env::var_os("DOSKER_QEMU"),
            path: std::env::var_os("PATH"),
            current_dir: std::env::current_dir().expect("read current directory"),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.qemu {
                Some(value) => std::env::set_var("DOSKER_QEMU", value),
                None => std::env::remove_var("DOSKER_QEMU"),
            }
            match &self.path {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
            std::env::set_current_dir(&self.current_dir).expect("restore current directory");
        }
    }
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(unix)]
fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    // Write the script in a child process. A write descriptor in this
    // multithreaded test process can be inherited by another test's fork,
    // which makes exec return ETXTBSY.
    let path = dir.join(name);
    let mut child = Command::new("sh")
        .arg("-c")
        .arg("cat > \"$1\" && chmod 755 \"$1\"")
        .arg("sh")
        .arg(&path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("start the script writer");
    {
        let mut stdin = child.stdin.take().expect("open the script writer stdin");
        stdin
            .write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
            .expect("write the script body");
    }
    let status = child.wait().expect("wait for the script writer");
    assert!(status.success(), "script writer failed: {status:?}");
    path
}

#[test]
fn version_parses_a_three_part_version() {
    let version = QemuVersion::parse("QEMU emulator version 8.2.0").expect("parse version");

    assert_eq!(version, QemuVersion::new(8, 2, 0));
    assert_eq!(version.to_string(), "8.2.0");
}

#[test]
fn version_parses_a_distribution_suffix() {
    let version = QemuVersion::parse("QEMU emulator version 8.2.2 (Debian 8.2.2+ds-0+deb12u1)")
        .expect("parse version");

    assert_eq!(version, QemuVersion::new(8, 2, 2));
}

#[test]
fn version_rejects_a_non_version_line() {
    assert!(QemuVersion::parse("qemu-system-i386: not a version").is_none());
    assert!(QemuVersion::parse("").is_none());
}

#[cfg(unix)]
#[test]
fn discover_uses_the_environment_override() {
    let _lock = lock();
    let _guard = EnvGuard::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = write_script(
        dir.path(),
        "qemu-override",
        "echo 'QEMU emulator version 8.1.2'",
    );

    unsafe {
        std::env::set_var("DOSKER_QEMU", &script);
    }

    let binary = QemuBinary::discover().expect("discover qemu");

    assert_eq!(binary.path, script);
    assert_eq!(binary.version_string(), "8.1.2");
}

#[cfg(unix)]
#[test]
fn discover_finds_a_bare_name_on_path() {
    let _lock = lock();
    let _guard = EnvGuard::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).expect("create bin directory");
    let script = write_script(
        &bin,
        "qemu-system-i386",
        "echo 'QEMU emulator version 7.2.0'",
    );

    unsafe {
        std::env::set_var("DOSKER_QEMU", "qemu-system-i386");
        std::env::set_var("PATH", &bin);
    }

    let binary = QemuBinary::discover().expect("discover qemu");

    assert_eq!(binary.path, script);
    assert_eq!(binary.version, QemuVersion::new(7, 2, 0));
}

#[cfg(unix)]
#[test]
fn discover_resolves_a_path_like_relative_value() {
    let _lock = lock();
    let _guard = EnvGuard::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let tools = dir.path().join("tools");
    std::fs::create_dir_all(&tools).expect("create tools directory");
    let script = write_script(
        &tools,
        "qemu-relative",
        "echo 'QEMU emulator version 6.2.0'",
    );

    unsafe {
        std::env::set_current_dir(dir.path()).expect("enter temp directory");
        std::env::set_var("DOSKER_QEMU", "tools/qemu-relative");
    }

    let binary = QemuBinary::discover().expect("discover qemu");

    let expected = std::env::current_dir()
        .expect("read current directory")
        .join("tools")
        .join("qemu-relative");
    assert_eq!(binary.path, expected);
    assert!(script.is_file());
    assert_eq!(binary.version, QemuVersion::new(6, 2, 0));
}

#[cfg(unix)]
#[test]
fn discover_times_out_a_hanging_binary() {
    let _lock = lock();
    let _guard = EnvGuard::capture();
    let dir = tempfile::tempdir().expect("create temp directory");
    let pid_file = dir.path().join("qemu-hanging.pid");
    let script = write_script(
        dir.path(),
        "qemu-hanging",
        &format!("echo $$ > '{}'\nexec sleep 10", pid_file.display()),
    );
    let started = std::time::Instant::now();

    unsafe {
        std::env::set_var("DOSKER_QEMU", &script);
    }

    let error = QemuBinary::discover().expect_err("discovery must time out");

    assert!(
        matches!(error, EngineError::Setup(_)),
        "unexpected error: {error}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(6));

    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("read the hanging process id")
        .trim()
        .parse()
        .expect("parse the hanging process id");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("kill -0 {pid}"))
        .stderr(std::process::Stdio::null())
        .status()
        .expect("probe the hanging process");
    assert!(!status.success(), "discovery left process {pid} running");
}

#[test]
fn discover_reports_a_missing_binary() {
    let _lock = lock();
    let _guard = EnvGuard::capture();

    unsafe {
        std::env::set_var("DOSKER_QEMU", "qemu-missing-binary");
        std::env::set_var("PATH", "");
    }

    let error = QemuBinary::discover().expect_err("missing binary must fail");

    match error {
        EngineError::NotFound(path) => {
            assert_eq!(path, PathBuf::from("qemu-missing-binary"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn source_reports_an_environment_override() {
    let root = PathBuf::from("/data/dosker");
    let path = root.join("engines").join("qemu-system-i386");

    let source = engine_source(&path, &root, true);

    assert_eq!(source, EngineSource::Env);
    assert_eq!(source.as_str(), "env");
    assert_eq!(EngineSource::parse(source.as_str()), Some(source));
}

#[test]
fn source_reports_a_managed_binary() {
    let root = PathBuf::from("/data/dosker");
    let path = root.join("engines").join("qemu-system-i386");

    let source = engine_source(&path, &root, false);

    assert_eq!(source, EngineSource::Managed);
    assert_eq!(source.as_str(), "managed");
    assert_eq!(EngineSource::parse(source.as_str()), Some(source));
}

#[cfg(target_os = "macos")]
#[test]
fn source_reports_a_homebrew_binary_on_macos() {
    let root = PathBuf::from("/data/dosker");

    let source = engine_source(
        Path::new("/opt/homebrew/bin/qemu-system-i386"),
        &root,
        false,
    );
    assert_eq!(source, EngineSource::Homebrew);

    let intel = engine_source(Path::new("/usr/local/bin/qemu-system-i386"), &root, false);
    assert_eq!(intel, EngineSource::Homebrew);

    assert_eq!(source.as_str(), "homebrew");
    assert_eq!(EngineSource::parse(source.as_str()), Some(source));
}

#[test]
fn source_reports_a_plain_path() {
    let root = PathBuf::from("/data/dosker");

    let source = engine_source(Path::new("/usr/bin/qemu-system-i386"), &root, false);

    assert_eq!(source, EngineSource::Path);
    assert_eq!(source.as_str(), "path");
    assert_eq!(EngineSource::parse(source.as_str()), Some(source));
}

#[cfg(unix)]
#[test]
fn probe_reports_the_parsed_version() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let script = write_script(
        dir.path(),
        "qemu-probe",
        "echo 'QEMU emulator version 9.0.1 (v9.0.1)'",
    );

    let binary = QemuBinary::from_path(script.clone()).expect("probe qemu");

    assert_eq!(binary.path, script);
    assert_eq!(binary.version, QemuVersion::new(9, 0, 1));
    assert_eq!(binary.version_string(), "9.0.1");
}
