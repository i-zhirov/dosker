#![cfg(unix)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use common::{CliRoot, run_captured, stderr_text, stdout_text};
use dosker_core::ContainerId;
use dosker_test_support::fixtures::{integration_enabled, kill_process, qemu_image};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const QEMU_COMMAND_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_STEP: Duration = Duration::from_millis(200);
const VVFAT_DISABLED_ENV: &str = "DOSKER_NO_VVFAT";

// A real guest boots here, so only one test may run at a time even when the
// test harness is parallel.
static INTEGRATION_LOCK: Mutex<()> = Mutex::new(());

fn prepared_image() -> Option<PathBuf> {
    if !integration_enabled() {
        return None;
    }

    match qemu_image() {
        Ok(path) => Some(path),
        Err(error) => {
            eprintln!(
                "the integration tests are enabled, but the prepared image is unavailable: {error}"
            );
            panic!("the prepared QEMU image is unavailable: {error}");
        }
    }
}

fn integration_lock() -> MutexGuard<'static, ()> {
    INTEGRATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn import_base(root: &CliRoot, image: &Path) {
    let output = root.run_with_qemu(&[
        "image",
        "import",
        image.to_str().expect("image path is utf-8"),
        "freedos:latest",
    ]);
    assert_succeeds(&output);
}

fn bind(drive: &str, dir: &Path) -> String {
    format!("{drive}{}", dir.display())
}

/// Starts a detached container with one bind mount and returns its guard.
fn run_detached<'a>(root: &'a CliRoot, name: &str, value: &str) -> Container<'a> {
    let output = root.run_with_qemu(&[
        "run",
        "-d",
        "--name",
        name,
        "--bind",
        value,
        "freedos:latest",
    ]);
    container_from_output(root, &output)
}

/// Runs a command with `vvfat` disabled and captures its output.
fn run_with_vvfat_disabled(root: &CliRoot, args: &[&str]) -> Output {
    let mut command = root.command_with_qemu();
    command.env(VVFAT_DISABLED_ENV, "1");
    command.args(args);
    run_captured(&mut command, QEMU_COMMAND_TIMEOUT)
}

fn container_from_output<'a>(root: &'a CliRoot, output: &Output) -> Container<'a> {
    assert_succeeds(output);
    let stdout = stdout_text(output);
    assert_eq!(
        stdout.lines().count(),
        1,
        "run -d did not print exactly one line: {stdout:?}"
    );
    let id = ContainerId::parse(stdout.trim()).expect("parse the container id");
    Container::new(root, id)
}

fn exec(root: &CliRoot, name: &str, command: &[&str]) -> Output {
    let mut args = vec!["exec", name];
    args.extend_from_slice(command);
    root.run_with_qemu(&args)
}

/// Repeats an `exec` until its stdout matches `expected` or ten seconds pass.
///
/// `refresh` runs before each attempt: FreeDOS buffers the sectors it read and
/// QEMU caches the cluster it last served, so the guest must touch the live
/// directory again before it re-reads a file that changed on the host.
fn exec_until(
    root: &CliRoot,
    name: &str,
    refresh: &[&str],
    command: &[&str],
    expected: &str,
) -> Output {
    let deadline = Instant::now() + POLL_TIMEOUT;
    loop {
        let _ = exec(root, name, refresh);
        let output = exec(root, name, command);
        if stdout_text(&output) == expected || Instant::now() >= deadline {
            return output;
        }
        std::thread::sleep(POLL_STEP);
    }
}

fn assert_succeeds(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(output),
        stderr_text(output)
    );
}

/// A container that tries to stop and remove itself when it goes out of scope.
struct Container<'a> {
    root: &'a CliRoot,
    id: ContainerId,
}

impl<'a> Container<'a> {
    fn new(root: &'a CliRoot, id: ContainerId) -> Self {
        Self { root, id }
    }

    fn id(&self) -> &ContainerId {
        &self.id
    }
}

impl Drop for Container<'_> {
    fn drop(&mut self) {
        let record = self.root.store().load(&self.id).ok();
        let id = self.id.as_str();
        let stopped = cleanup(self.root, &["stop", id]);
        let removed = cleanup(self.root, &["rm", id]);

        if stopped && removed {
            return;
        }

        if let Some(record) = record {
            if let Some(pid) = record.engine_pid {
                kill_process(pid);
            }
            if let Some(pid) = record.pid {
                kill_process(pid);
            }
        }
    }
}

/// Runs one cleanup command with the harness timeout.
///
/// A timeout panic is caught because this runs from `Drop`, where a second
/// panic would abort the test process.
fn cleanup(root: &CliRoot, args: &[&str]) -> bool {
    let mut command = root.command_with_qemu();
    command.args(args);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_captured(&mut command, CLEANUP_TIMEOUT)
    }))
    .map(|output| output.status.success())
    .unwrap_or(false)
}

#[test]
fn a_host_edit_is_visible_in_the_guest_without_a_rebuild() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);
    let host = tempfile::tempdir().expect("create a host directory");
    let file = host.path().join("HOST.TXT");
    fs::write(&file, b"ONE\r\n").expect("write the host file");

    let _container = run_detached(&root, "bind", &bind("D:", host.path()));

    let initial = exec(&root, "bind", &["TYPE", r"D:\HOST.TXT"]);
    assert_succeeds(&initial);
    assert_eq!(
        stdout_text(&initial),
        "ONE\r\n",
        "the initial host content differs"
    );

    fs::write(&file, b"TWO\r\n").expect("overwrite the host file");

    let updated = exec_until(
        &root,
        "bind",
        &["DIR", r"D:\"],
        &["TYPE", r"D:\HOST.TXT"],
        "TWO\r\n",
    );
    assert_eq!(
        stdout_text(&updated),
        "TWO\r\n",
        "the host edit is not visible in the guest"
    );

    let stopped = root.run_with_qemu(&["stop", "bind"]);
    assert_succeeds(&stopped);
    let removed = root.run_with_qemu(&["rm", "bind"]);
    assert_succeeds(&removed);
}

#[test]
fn an_invalid_8_3_name_is_refused_before_boot_and_names_the_path() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);
    let host = tempfile::tempdir().expect("create a host directory");
    fs::write(host.path().join("TOOLONGNAME.TXT"), b"HOST\r\n").expect("write the host file");
    let value = bind("D:", host.path());

    let output = root.run_with_qemu(&[
        "run",
        "-d",
        "--name",
        "bad",
        "--bind",
        &value,
        "freedos:latest",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {}; stderr: {}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let combined = format!("{}{}", stdout_text(&output), stderr_text(&output));
    assert!(
        combined.contains(&host.path().display().to_string()),
        "the error does not name the host path: {combined:?}"
    );
    assert!(
        combined.contains("TOOLONGNAME.TXT"),
        "the error does not name the offending file: {combined:?}"
    );

    let ps = root.run_with_qemu(&["ps", "--all"]);
    assert_succeeds(&ps);
    let listing = stdout_text(&ps);
    assert!(
        !listing
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some("bad")),
        "ps lists the refused container:\n{listing}"
    );
}

#[test]
fn a_read_only_bind_rejects_a_guest_write() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);
    let host = tempfile::tempdir().expect("create a host directory");
    let file = host.path().join("HOST.TXT");
    let original = b"ONE\r\n";
    fs::write(&file, original).expect("write the host file");

    let value = format!("{}:ro", bind("D:", host.path()));
    let _container = run_detached(&root, "robind", &value);

    let read = exec(&root, "robind", &["TYPE", r"D:\HOST.TXT"]);
    assert_succeeds(&read);
    assert_eq!(
        stdout_text(&read),
        "ONE\r\n",
        "the read-only bind did not show the host content"
    );

    let write = exec(
        &root,
        "robind",
        &["COPY", r"C:\DOSKER\TWO.TXT", r"D:\BLOCKED.TXT"],
    );
    assert_succeeds(&write);

    let stopped = root.run_with_qemu(&["stop", "robind"]);
    assert_succeeds(&stopped);

    assert!(
        !host.path().join("BLOCKED.TXT").exists(),
        "the read-only bind accepted a guest write"
    );
    assert_eq!(
        fs::read(&file).expect("read the host file"),
        original,
        "the read-only bind changed the host file"
    );
}

#[test]
fn the_fallback_shows_the_same_content_when_vvfat_is_disabled() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);
    let host = tempfile::tempdir().expect("create a host directory");
    let content = b"ONE\r\nTWO\r\n";
    fs::write(host.path().join("HOST.TXT"), content).expect("write the host file");
    let value = bind("D:", host.path());

    let output = run_with_vvfat_disabled(
        &root,
        &[
            "run",
            "-d",
            "--name",
            "sync",
            "--bind",
            &value,
            "freedos:latest",
        ],
    );
    let container = container_from_output(&root, &output);

    let read = exec(&root, "sync", &["TYPE", r"D:\HOST.TXT"]);
    assert_succeeds(&read);
    assert_eq!(
        stdout_text(&read),
        "ONE\r\nTWO\r\n",
        "the fallback did not show the host content"
    );

    let write = exec(
        &root,
        "sync",
        &["COPY", r"C:\DOSKER\TWO.TXT", r"D:\FROMDOS.TXT"],
    );
    assert_succeeds(&write);

    let stopped = root.run_with_qemu(&["stop", "sync"]);
    assert_succeeds(&stopped);

    let synced = host.path().join("FROMDOS.TXT");
    assert_eq!(
        fs::read(&synced).expect("read the synced file"),
        content,
        "the fallback did not copy the guest file back to the host"
    );
    let bind_image = root
        .path()
        .join("containers")
        .join(container.id().as_str())
        .join("bind-d.img");
    assert!(
        !bind_image.exists(),
        "the fallback image was not removed after the stop: {}",
        bind_image.display()
    );
}
