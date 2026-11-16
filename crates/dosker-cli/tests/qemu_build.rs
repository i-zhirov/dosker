#![cfg(unix)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use common::{CliRoot, stderr_text, stdout_text};
use dosker_core::ContainerId;
use dosker_image::{DosPath, ImageRef, OciStore, TreeNode};
use dosker_test_support::fixtures::{integration_enabled, qemu_image};

const COPY_TEXT: &[u8] = b"COPIED\r\n";
const HOST_TEXT: &[u8] = b"HOST";
const CONTAINER_NAME: &str = "build-check";

// A real guest boots here, and the container name is fixed, so only one
// test may run at a time even when the test harness is parallel.
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
        "base:latest",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
}

fn store(root: &CliRoot) -> OciStore {
    OciStore::new(root.path().to_path_buf())
}

fn path(value: &str) -> DosPath {
    DosPath::parse(value).expect("the DOS path parses")
}

fn file_bytes(tree: &dosker_image::Tree, value: &str) -> Vec<u8> {
    match tree.get(&path(value)) {
        Some(TreeNode::File(file)) => file.bytes.clone(),
        Some(TreeNode::Dir(_)) => panic!("{value} is a directory, not a file"),
        None => panic!("the tree has no {value}"),
    }
}

#[test]
fn built_image_boots_and_answers_exec() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _serial = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);

    let context = tempfile::tempdir().expect("create a build context");
    fs::write(context.path().join("HELLO.TXT"), COPY_TEXT).expect("write the copy source");
    fs::write(
        context.path().join("Doskerfile"),
        "FROM base:latest\n\
         COPY HELLO.TXT C:/APPS/\n\
         RUN --host printf HOST > GEN.TXT\n\
         ENV GREETING=HELLO\n\
         WORKDIR C:/APPS\n\
         CMD ECHO BUILT\n\
         MACHINE memory=32M\n",
    )
    .expect("write the Doskerfile");

    let build = root.run_with_qemu(&[
        "build",
        "-t",
        "built:latest",
        context.path().to_str().expect("context path is utf-8"),
    ]);
    assert_eq!(
        build.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&build),
        stderr_text(&build)
    );
    assert!(
        stdout_text(&build).contains("built built:latest sha256:"),
        "stdout: {}",
        stdout_text(&build)
    );

    let store = store(&root);
    let built = ImageRef::parse("built:latest").expect("the built reference parses");
    let base = ImageRef::parse("base:latest").expect("the base reference parses");

    let tree = store.load_tree(&built).expect("the built tree loads");
    assert_eq!(
        file_bytes(&tree, r"C:\APPS\HELLO.TXT"),
        COPY_TEXT,
        "the COPY destination is absent or changed"
    );
    assert_eq!(
        file_bytes(&tree, r"C:\GEN.TXT"),
        HOST_TEXT,
        "the RUN --host output is absent or changed"
    );

    let config = store.load_config(&built).expect("the built config loads");
    assert_eq!(
        config.env.get("GREETING").map(String::as_str),
        Some("HELLO"),
        "the ENV value is not recorded"
    );
    assert_eq!(config.workdir, r"C:\APPS", "the WORKDIR is not recorded");
    assert_eq!(
        config.machine.memory_mb, 32,
        "the MACHINE value is not recorded"
    );
    assert_eq!(
        config.cmd.as_deref(),
        Some("ECHO BUILT"),
        "the CMD is not recorded"
    );

    let built_manifest = store
        .load_manifest(&built)
        .expect("the built manifest loads");
    let base_manifest = store.load_manifest(&base).expect("the base manifest loads");
    let built_disk = built_manifest.raw_disk_digest();
    assert!(
        built_disk.is_some(),
        "the built image lost its raw-disk annotation"
    );
    assert_eq!(
        built_disk,
        base_manifest.raw_disk_digest(),
        "the built image changed the raw-disk annotation"
    );

    // The built image boots the retained base disk. The tree checks above prove
    // COPY and RUN --host. This boot proves boot and exec only.
    let started = root.run_with_qemu(&["run", "-d", "--name", CONTAINER_NAME, "built:latest"]);
    assert_eq!(
        started.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&started),
        stderr_text(&started)
    );
    let id = ContainerId::parse(stdout_text(&started).trim()).expect("parse the container id");

    let hello = root.run_with_qemu(&["exec", id.as_str(), "ECHO", "HELLO"]);
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

    let stopped = root.run_with_qemu(&["stop", id.as_str()]);
    assert_eq!(
        stopped.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(&stopped),
        stderr_text(&stopped)
    );
}
