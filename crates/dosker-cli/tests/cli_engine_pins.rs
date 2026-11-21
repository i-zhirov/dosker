#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{CliRoot, run_captured, stderr_text, stdout_text};

const TIMEOUT: Duration = Duration::from_secs(10);

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write the fake engine script");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
        .expect("make the fake engine script executable");
    path
}

fn prepare(root: &CliRoot, qemu_body: &str) -> PathBuf {
    write_script(root.path(), "dosbox-x", "#!/bin/sh\nexit 1\n");
    write_script(root.path(), "bochs", "#!/bin/sh\nexit 1\n");
    write_script(root.path(), "qemu-system-i386", qemu_body)
}

#[test]
fn engines_reports_a_version_pin_mismatch() {
    let root = CliRoot::new();
    let qemu = prepare(&root, "#!/bin/sh\necho 'QEMU emulator version 11.1.3'\n");
    let mut command = root.base_command();
    command.env("DOSKER_QEMU", &qemu);
    command.env("DOSKER_DOSBOX_X", root.path().join("dosbox-x"));
    command.env("DOSKER_BOCHS", root.path().join("bochs"));
    command.env("DOSKER_QEMU_VERSION", "9.9.9");
    command.args(["engines"]);
    let output = run_captured(&mut command, TIMEOUT);

    let stdout = stdout_text(&output);
    assert!(stdout.contains("qemu unavailable:"), "{stdout}");
    assert!(stdout.contains("version mismatch"), "{stdout}");
    assert!(stdout.contains("9.9.9"), "{stdout}");
    assert!(stdout.contains("11.1.3"), "{stdout}");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_text(&output));
}

#[test]
fn engines_reports_a_checksum_pin_mismatch() {
    let root = CliRoot::new();
    let qemu = prepare(&root, "#!/bin/sh\necho 'QEMU emulator version 11.1.3'\n");
    let mut command = root.base_command();
    command.env("DOSKER_QEMU", &qemu);
    command.env("DOSKER_DOSBOX_X", root.path().join("dosbox-x"));
    command.env("DOSKER_BOCHS", root.path().join("bochs"));
    command.env("DOSKER_QEMU_SHA256", "0".repeat(64));
    command.args(["engines"]);
    let output = run_captured(&mut command, TIMEOUT);

    let stdout = stdout_text(&output);
    assert!(stdout.contains("qemu unavailable:"), "{stdout}");
    assert!(stdout.contains("checksum mismatch"), "{stdout}");
}

#[test]
fn engines_accepts_a_matching_checksum_pin() {
    let root = CliRoot::new();
    let body = "#!/bin/sh\necho 'QEMU emulator version 11.1.3'\n";
    let qemu = prepare(&root, body);
    let digest = dosker_image::sha256_digest(body.as_bytes());
    let sha256 = digest.strip_prefix("sha256:").expect("a prefixed digest");
    let mut command = root.base_command();
    command.env("DOSKER_QEMU", &qemu);
    command.env("DOSKER_DOSBOX_X", root.path().join("dosbox-x"));
    command.env("DOSKER_BOCHS", root.path().join("bochs"));
    command.env("DOSKER_QEMU_SHA256", sha256);
    command.args(["engines"]);
    let output = run_captured(&mut command, TIMEOUT);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("qemu 11.1.3"));
}

#[test]
fn doctor_reports_a_checksum_pin_mismatch() {
    let root = CliRoot::new();
    let qemu = prepare(&root, "#!/bin/sh\necho 'QEMU emulator version 11.1.3'\n");
    let mut command = root.base_command();
    command.env("DOSKER_QEMU", &qemu);
    command.env("DOSKER_DOSBOX_X", root.path().join("dosbox-x"));
    command.env("DOSKER_BOCHS", root.path().join("bochs"));
    command.env("DOSKER_QEMU_SHA256", "0".repeat(64));
    command.args(["doctor"]);
    let output = run_captured(&mut command, TIMEOUT);

    let stdout = stdout_text(&output);
    assert!(stdout.contains("qemu: unavailable ("), "{stdout}");
    assert!(stdout.contains("checksum mismatch"), "{stdout}");
}
