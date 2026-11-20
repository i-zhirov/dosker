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
const CAPABILITIES: &str =
    "boot,serial,stop,headless,host_dir_live,debug_native,gdbstub,screenshot,vnc,nat";
#[cfg(unix)]
const DOSBOX_X_VERSION_LINE: &str =
    "DOSBox-X version 2026.10.01 SDL2, copyright 2011-2026 The DOSBox-X Team.";
#[cfg(unix)]
const DOSBOX_X_CAPABILITIES: &str = "boot,serial,stop,headless,debug_native";
#[cfg(unix)]
const SECURITY: &str =
    "security: DOSker does not sandbox untrusted images; use an outer VM for untrusted input.";
#[cfg(unix)]
const DEVELOPMENT_WARNING: &str = "development mode: --bind uses QEMU vvfat (beta) and can corrupt host files. Use it for development only, never in CI.";

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
fn write_dosbox_x_script(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("dosbox-x");
    fs::write(&path, body).expect("write the dosbox-x script");
    let mut permissions = fs::metadata(&path)
        .expect("read the script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("set the script permissions");
    path
}

#[cfg(unix)]
fn write_dosbox_x(dir: &std::path::Path) -> std::path::PathBuf {
    write_dosbox_x_script(dir, &format!("#!/bin/sh\necho '{DOSBOX_X_VERSION_LINE}'\n"))
}

fn run_with_engines(
    root: &CliRoot,
    qemu: &std::path::Path,
    dosbox_x: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    let mut command = root.base_command();
    command.env("DOSKER_QEMU", qemu);
    command.env("DOSKER_DOSBOX_X", dosbox_x);
    command.args(args);
    run_captured(&mut command, RUN_TIMEOUT)
}

#[cfg(unix)]
#[test]
fn engines_reports_both_engines() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &qemu, &dosbox_x, &["engines"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let qemu_line = format!("qemu 11.1.3 {} {CAPABILITIES}", qemu.display());
    assert!(
        text.lines().any(|line| line == qemu_line),
        "expected {qemu_line:?} in stdout: {text}"
    );
    let dosbox_x_line = format!(
        "dosbox-x 2026.10.01 {} {DOSBOX_X_CAPABILITIES}",
        dosbox_x.display()
    );
    assert!(
        text.lines().any(|line| line == dosbox_x_line),
        "expected {dosbox_x_line:?} in stdout: {text}"
    );
}

#[cfg(unix)]
#[test]
fn engines_lists_the_dosbox_x_proven_capabilities() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &qemu, &dosbox_x, &["engines"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let dosbox_x_line = text
        .lines()
        .find(|line| line.starts_with("dosbox-x "))
        .expect("engines printed a dosbox-x line");
    let dosbox_x_fields: Vec<&str> = dosbox_x_line.split_whitespace().collect();
    assert_eq!(
        dosbox_x_fields.get(3),
        Some(&DOSBOX_X_CAPABILITIES),
        "stdout: {text}"
    );
    let qemu_line = text
        .lines()
        .find(|line| line.starts_with("qemu "))
        .expect("engines printed a qemu line");
    let qemu_fields: Vec<&str> = qemu_line.split_whitespace().collect();
    let capabilities: Vec<&str> = qemu_fields
        .get(3)
        .expect("the qemu line has a capability field")
        .split(',')
        .collect();
    assert!(capabilities.contains(&"host_dir_live"), "stdout: {text}");
    assert!(capabilities.contains(&"nat"), "stdout: {text}");
    assert!(!capabilities.contains(&"audio"), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn engines_exits_zero_when_only_one_engine_is_found() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let missing_qemu = root.path().join("missing-qemu");
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &missing_qemu, &dosbox_x, &["engines"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("qemu not found"), "stdout: {text}");
    assert!(text.contains("dosbox-x 2026.10.01"), "stdout: {text}");
}

#[test]
fn engines_exits_one_when_no_engine_is_found() {
    let root = CliRoot::new();
    let missing_qemu = root.path().join("missing-qemu");
    let missing_dosbox_x = root.path().join("missing-dosbox-x");

    let output = run_with_engines(&root, &missing_qemu, &missing_dosbox_x, &["engines"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("qemu not found"), "stdout: {text}");
    assert!(text.contains("dosbox-x not found"), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn engines_reports_an_unavailable_engine() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu_script(dir.path(), "#!/bin/sh\nexit 1\n");
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &qemu, &dosbox_x, &["engines"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("qemu unavailable:"), "stdout: {text}");
    assert!(!text.contains("qemu not found"), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn doctor_reports_the_data_root_and_both_engines() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &qemu, &dosbox_x, &["doctor"]);

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
        text.contains(&format!("dosbox-x: 2026.10.01 ({})", dosbox_x.display())),
        "stdout: {text}"
    );
}

#[cfg(unix)]
#[test]
fn doctor_lists_each_engine_with_its_capabilities() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &qemu, &dosbox_x, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let qemu_line = format!("qemu: 11.1.3 ({}) [{CAPABILITIES}]", qemu.display());
    assert!(text.contains(&qemu_line), "stdout: {text}");
    let dosbox_x_line = format!(
        "dosbox-x: 2026.10.01 ({}) [{DOSBOX_X_CAPABILITIES}]",
        dosbox_x.display()
    );
    assert!(text.contains(&dosbox_x_line), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn doctor_exits_zero_when_only_one_engine_is_found() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let missing_qemu = root.path().join("missing-qemu");
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &missing_qemu, &dosbox_x, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("qemu: not found"), "stdout: {text}");
    assert!(text.contains("dosbox-x: 2026.10.01"), "stdout: {text}");
}

#[test]
fn doctor_exits_one_when_no_engine_is_found() {
    let root = CliRoot::new();
    let missing_qemu = root.path().join("missing-qemu");
    let missing_dosbox_x = root.path().join("missing-dosbox-x");

    let output = run_with_engines(&root, &missing_qemu, &missing_dosbox_x, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("qemu: not found"), "stdout: {text}");
    assert!(text.contains("dosbox-x: not found"), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn doctor_reports_unavailable_when_an_engine_fails() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu_script(dir.path(), "#!/bin/sh\nexit 1\n");
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &qemu, &dosbox_x, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(0),
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
fn doctor_reports_unavailable_when_an_engine_version_is_unparsable() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu_script(dir.path(), "#!/bin/sh\necho 'not qemu'\n");
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &qemu, &dosbox_x, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(0),
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
fn doctor_reports_an_unwritable_root() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());
    let dosbox_x = write_dosbox_x(dir.path());
    let root_file = root.path().join("not-a-directory");
    fs::write(&root_file, b"not a directory").expect("write the root file");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dosker"));
    command.env("DOSKER_ROOT", &root_file);
    command.env("DOSKER_QEMU", &qemu);
    command.env("DOSKER_DOSBOX_X", &dosbox_x);
    command.arg("doctor");
    let output = run_captured(&mut command, RUN_TIMEOUT);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(
        text.contains("data root writable: no"),
        "stdout: {text}; stderr: {}",
        stderr_text(&output)
    );
    assert!(
        text.contains(&format!("qemu: 11.1.3 ({})", qemu.display())),
        "stdout: {text}"
    );
    assert!(
        text.contains(&format!("dosbox-x: 2026.10.01 ({})", dosbox_x.display())),
        "stdout: {text}"
    );
}

#[cfg(unix)]
#[test]
fn doctor_prints_the_security_statement() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &qemu, &dosbox_x, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains(SECURITY), "stdout: {text}");
    assert!(
        text.contains(&format!("qemu: 11.1.3 ({})", qemu.display())),
        "stdout: {text}"
    );
    assert!(
        text.contains(&format!("dosbox-x: 2026.10.01 ({})", dosbox_x.display())),
        "stdout: {text}"
    );
}

#[cfg(unix)]
#[test]
fn doctor_prints_the_development_mode_warning() {
    let root = CliRoot::new();
    let dir = tempfile::tempdir().expect("create a script directory");
    let qemu = write_qemu(dir.path());
    let dosbox_x = write_dosbox_x(dir.path());

    let output = run_with_engines(&root, &qemu, &dosbox_x, &["doctor"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains(DEVELOPMENT_WARNING), "stdout: {text}");
    assert!(
        text.contains(&format!("qemu: 11.1.3 ({})", qemu.display())),
        "stdout: {text}"
    );
    assert!(
        text.contains(&format!("dosbox-x: 2026.10.01 ({})", dosbox_x.display())),
        "stdout: {text}"
    );
}
