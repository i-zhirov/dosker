#![cfg(unix)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use common::{CliRoot, run_captured, stderr_text, stdout_text};
use dosker_test_support::fixtures::{FREEDOS_SOURCE_ENV, freedos_base_image, integration_enabled};

const QEMU_TIMEOUT: Duration = Duration::from_secs(180);
const BUILD_IMAGE: &str = "freedos-built:latest";
const IMPORTED_IMAGE: &str = "injected-image:latest";

// A real guest boots here, and the container names are fixed, so only one
// test may run at a time even when the test harness is parallel.
static INTEGRATION_LOCK: Mutex<()> = Mutex::new(());

fn integration_lock() -> MutexGuard<'static, ()> {
    INTEGRATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn workspace_path(value: &Path) -> PathBuf {
    if value.is_absolute() {
        value.to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(value)
    }
}

fn freedos_source() -> Option<PathBuf> {
    if !integration_enabled() {
        return None;
    }

    let value = std::env::var_os(FREEDOS_SOURCE_ENV).filter(|value| !value.is_empty());
    let Some(value) = value else {
        panic!("the integration tests are enabled, but {FREEDOS_SOURCE_ENV} is not set");
    };

    let path = workspace_path(Path::new(&value));
    if !path.is_file() {
        panic!(
            "the integration tests are enabled, but {FREEDOS_SOURCE_ENV} does not name a file: {}",
            path.display()
        );
    }
    Some(path)
}

fn prepared_base_image() -> Option<PathBuf> {
    if !integration_enabled() {
        return None;
    }

    match freedos_base_image() {
        Ok(path) => Some(path),
        Err(error) => {
            eprintln!(
                "the integration tests are enabled, but the FreeDOS base image is unavailable: {error}"
            );
            panic!("the FreeDOS base image is unavailable: {error}");
        }
    }
}

fn stop(root: &CliRoot, name: &str) {
    let output = root.run_with_qemu(&["stop", name]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
}

#[test]
fn built_freedos_base_boots_and_answers_exec() {
    let Some(source) = freedos_source() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();

    let context = tempfile::tempdir().expect("create a build context");
    fs::write(context.path().join("Doskerfile"), "FROM freedos:1.3\n")
        .expect("write the Doskerfile");

    let mut build = root.command_with_qemu();
    build
        .env(FREEDOS_SOURCE_ENV, &source)
        .args(["build", "-t", BUILD_IMAGE])
        .arg(context.path());
    let build = run_captured(&mut build, QEMU_TIMEOUT);
    assert_eq!(
        build.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&build),
        stderr_text(&build)
    );

    let started = root.run_with_qemu(&["run", "-d", "--name", "freedos-check", BUILD_IMAGE]);
    assert_eq!(
        started.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&started),
        stderr_text(&started)
    );

    let hello = root.run_with_qemu(&["exec", "freedos-check", "ECHO", "HELLO"]);
    assert_eq!(
        hello.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&hello),
        stderr_text(&hello)
    );
    assert!(
        stdout_text(&hello).contains("HELLO"),
        "stdout: {}",
        stdout_text(&hello)
    );

    stop(&root, "freedos-check");
}

#[test]
fn imported_dos_image_with_injected_agent_boots_and_answers_exec() {
    let Some(image) = prepared_base_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();

    let imported = root.run_with_qemu(&[
        "image",
        "import",
        image.to_str().expect("image path is utf-8"),
        IMPORTED_IMAGE,
    ]);
    assert_eq!(
        imported.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&imported),
        stderr_text(&imported)
    );

    let started = root.run_with_qemu(&["run", "-d", "--name", "injected-check", IMPORTED_IMAGE]);
    assert_eq!(
        started.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&started),
        stderr_text(&started)
    );

    let hello = root.run_with_qemu(&["exec", "injected-check", "ECHO", "HELLO"]);
    assert_eq!(
        hello.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&hello),
        stderr_text(&hello)
    );
    assert!(
        stdout_text(&hello).contains("HELLO"),
        "stdout: {}",
        stdout_text(&hello)
    );

    stop(&root, "injected-check");
}
