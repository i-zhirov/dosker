//! End-to-end checks for `dosker engines` and `dosker doctor`.

mod common;

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::process::Command;
use std::time::Duration;

use common::{CliRoot, run_captured, stderr_text, stdout_text};

const RUN_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(unix)]
const VERSION_LINE: &str = "QEMU emulator version 11.1.3";
#[cfg(unix)]
const CAPABILITIES: &str = "boot,serial,stop,headless,debug_native,gdbstub,screenshot,vnc";
#[cfg(unix)]
const SECURITY: &str =
    "security: DOSker does not sandbox untrusted images; use an outer VM for untrusted input.";

#[cfg(unix)]
fn write_qemu_script(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("qemu-system-i386");
    fs::write(&path, body).expect("write the qemu script");
    let mut permissions = fs::metadata(&path)
        .expect("read the script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("set the script permissions");
    path
}

#[cfg(unix)]
fn write_qemu(dir: &std::path::Path) -> std::path::PathBuf {
    write_qemu_script(dir, &format!("#!/bin/sh\necho '{VERSION_LINE}'\n"))
}

#[cfg(unix)]
fn run_with_qemu(root: &CliRoot, qemu: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut command = root.base_command();
    command.env("DOSKER_QEMU", qemu);
    command.args(args);
    run_captured(&mut command, RUN_TIMEOUT)
}

fn run_without_qemu(root: &CliRoot, args: &[&str]) -> std::process::Output {
    let missing = root.path().join("missing-qemu");
    let mut command = root.base_command();
    command.env("DOSKER_QEMU", missing);
    command.env("PATH", "");
    command.args(args);
    run_captured(&mut command, RUN_TIMEOUT)
}

#[cfg(unix)]
#[test]
fn engines_reports_the_qemu_version_and_path() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());

    let output = run_with_qemu(&root, &qemu, &["engines"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let line = text.lines().next().expect("engines printed a line");
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(fields.first(), Some(&"qemu"), "stdout: {text}");
    assert_eq!(fields.get(1), Some(&"11.1.3"), "stdout: {text}");
    assert_eq!(fields.get(2).copied(), qemu.to_str(), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn engines_lists_the_proven_capabilities() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());

    let output = run_with_qemu(&root, &qemu, &["engines"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let line = text.lines().next().expect("engines printed a line");
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(fields.get(3), Some(&CAPABILITIES), "stdout: {text}");
    let capabilities: Vec<&str> = fields[3].split(',').collect();
    assert!(!capabilities.contains(&"host_dir_live"), "stdout: {text}");
    assert!(!capabilities.contains(&"nat"), "stdout: {text}");
    assert!(!capabilities.contains(&"audio"), "stdout: {text}");
}

#[test]
fn engines_exits_one_when_qemu_is_missing() {
    let root = CliRoot::new();

    let output = run_without_qemu(&root, &["engines"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("qemu not found"),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn doctor_reports_the_data_root_and_qemu() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());

    let output = run_with_qemu(&root, &qemu, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(
        text.contains(&format!("data root: {}", root.path().display())),
        "stdout: {text}"
    );
    assert!(text.contains("data root writable: yes"), "stdout: {text}");
    assert!(
        text.contains(&format!("qemu: 11.1.3 ({})", qemu.display())),
        "stdout: {text}"
    );
    assert!(
        text.contains(&format!("capabilities: {CAPABILITIES}")),
        "stdout: {text}"
    );
}

#[cfg(unix)]
#[test]
fn doctor_prints_the_security_statement() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());

    let output = run_with_qemu(&root, &qemu, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains(SECURITY),
        "stdout: {}",
        stdout_text(&output)
    );
}

#[test]
fn doctor_exits_one_when_qemu_is_missing() {
    let root = CliRoot::new();

    let output = run_without_qemu(&root, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("qemu: not found"),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn doctor_reports_unavailable_when_qemu_exits_nonzero() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu_script(dir.path(), "#!/bin/sh\nexit 1\n");

    let output = run_with_qemu(&root, &qemu, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("qemu: unavailable ("), "stdout: {text}");
    assert!(!text.contains("qemu: not found"), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn doctor_reports_unavailable_when_the_qemu_version_is_unparsable() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu_script(dir.path(), "#!/bin/sh\necho 'not qemu'\n");

    let output = run_with_qemu(&root, &qemu, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("qemu: unavailable ("), "stdout: {text}");
    assert!(!text.contains("qemu: not found"), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn engines_exits_one_for_a_broken_qemu() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu_script(dir.path(), "#!/bin/sh\nexit 1\n");

    let output = run_with_qemu(&root, &qemu, &["engines"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert!(
        !stdout_text(&output).contains("qemu not found"),
        "stdout: {}",
        stdout_text(&output)
    );
    assert!(
        stderr_text(&output).contains("engine setup failed"),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn doctor_reports_an_unwritable_root() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());
    let root_file = root.path().join("not-a-directory");
    fs::write(&root_file, b"not a directory").expect("write the root file");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dosker"));
    command.env("DOSKER_ROOT", &root_file);
    command.env("DOSKER_QEMU", &qemu);
    command.arg("doctor");
    let output = run_captured(&mut command, RUN_TIMEOUT);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("data root writable: no"),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
}
