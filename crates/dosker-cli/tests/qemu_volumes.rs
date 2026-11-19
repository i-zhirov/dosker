#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use common::{CliRoot, run_captured, stderr_text, stdout_text};
use dosker_core::ContainerId;
use dosker_image::{DosPath, read_tree};
use dosker_test_support::fixtures::{integration_enabled, kill_process, qemu_image};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const PERSISTED_TEXT: &str = "ONE\r\nTWO\r\n";

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

fn create_volume(root: &CliRoot, name: &str) {
    let output = root.run_with_qemu(&["volume", "create", name]);
    assert_succeeds(&output);
}

fn run_detached<'a>(root: &'a CliRoot, image: &str, name: &str, volume: &str) -> Container<'a> {
    let output = root.run_with_qemu(&["run", "-d", "--name", name, "-v", volume, image]);
    assert_succeeds(&output);
    let stdout = stdout_text(&output);
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
fn partitioned_volume_persists_across_rm_and_rejects_a_second_attach() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);
    create_volume(&root, "data");

    let _first = run_detached(&root, "freedos:latest", "vol", "data:D:");

    let write = exec(
        &root,
        "vol",
        &["COPY", r"C:\DOSKER\TWO.TXT", r"D:\PERSIST.TXT"],
    );
    assert_succeeds(&write);

    let second = root.run_with_qemu(&[
        "run",
        "-d",
        "--name",
        "other",
        "-v",
        "data:D:",
        "freedos:latest",
    ]);
    assert_eq!(
        second.status.code(),
        Some(1),
        "stdout: {}; stderr: {}",
        stdout_text(&second),
        stderr_text(&second)
    );
    let combined = format!("{}{}", stdout_text(&second), stderr_text(&second));
    assert!(
        combined.contains("container vol"),
        "the second start does not name the holder: {combined:?}"
    );

    let removed_other = root.run_with_qemu(&["rm", "other"]);
    assert_succeeds(&removed_other);

    let stopped = root.run_with_qemu(&["stop", "vol"]);
    assert_succeeds(&stopped);
    let removed = root.run_with_qemu(&["rm", "vol"]);
    assert_succeeds(&removed);

    let _second_run = run_detached(&root, "freedos:latest", "vol2", "data:D:");

    let persisted = exec(&root, "vol2", &["TYPE", r"D:\PERSIST.TXT"]);
    assert_succeeds(&persisted);
    assert_eq!(
        stdout_text(&persisted),
        PERSISTED_TEXT,
        "the persisted text differs"
    );
}

#[test]
fn a_read_only_volume_discards_a_guest_write() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);
    create_volume(&root, "rovol");

    let _container = run_detached(&root, "freedos:latest", "rovol", "rovol:D:ro");

    let write = exec(
        &root,
        "rovol",
        &["COPY", r"C:\DOSKER\TWO.TXT", r"D:\BLOCKED.TXT"],
    );
    assert_succeeds(&write);

    let stopped = root.run_with_qemu(&["stop", "rovol"]);
    assert_succeeds(&stopped);

    let volume_image = root.path().join("volumes").join("rovol").join("volume.img");
    let tree = read_tree(&volume_image).expect("read the volume tree");
    let blocked = DosPath::parse(r"C:\BLOCKED.TXT").expect("parse the blocked path");
    assert!(
        tree.get(&blocked).is_none(),
        "the read-only volume accepted a guest write"
    );

    let listing = root.run_with_qemu(&["volume", "ls"]);
    assert_succeeds(&listing);
    let text = stdout_text(&listing);
    let line = text
        .lines()
        .find(|line| line.split_whitespace().next() == Some("rovol"))
        .unwrap_or_else(|| panic!("volume ls does not list rovol: {text:?}"));
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(
        fields.get(2),
        Some(&"mbr_partitioned"),
        "the volume layout changed: {line:?}"
    );
}
