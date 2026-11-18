#![cfg(unix)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use common::{CliRoot, run_captured, stderr_text, stdout_text};
use dosker_image::{DosPath, ImageError, ImageRef, Layer, OciStore, TreeNode};
use dosker_test_support::fixtures::{
    FREEDOS_SOURCE_ENV, freedos_source, integration_enabled, qemu_image, reboot_executable,
    setboot_executable,
};

const BUILD_TIMEOUT: Duration = Duration::from_secs(300);
const BUILT: &str = "built:latest";

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

fn reference(value: &str) -> ImageRef {
    ImageRef::parse(value).expect("the reference parses")
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

fn has_path(tree: &dosker_image::Tree, value: &str) -> bool {
    tree.get(&path(value)).is_some()
}

fn layers(store: &OciStore, reference: &str) -> Vec<Layer> {
    let manifest = store
        .load_manifest(&ImageRef::parse(reference).expect("the reference parses"))
        .expect("the image manifest loads");
    manifest
        .layers
        .iter()
        .map(|descriptor| {
            store
                .load_layer(&descriptor.digest)
                .expect("the layer loads")
        })
        .collect()
}

fn layer_digests(store: &OciStore, reference: &str) -> Vec<String> {
    store
        .load_manifest(&ImageRef::parse(reference).expect("the reference parses"))
        .expect("the image manifest loads")
        .layers
        .into_iter()
        .map(|descriptor| descriptor.digest)
        .collect()
}

fn layer_named<'a>(layers: &'a [Layer], created_by: &str) -> &'a Layer {
    layers
        .iter()
        .find(|layer| layer.meta.created_by == created_by)
        .unwrap_or_else(|| panic!("no layer is named {created_by}"))
}

fn build_context(files: &[(&str, &[u8])], doskerfile: &str) -> tempfile::TempDir {
    let context = tempfile::tempdir().expect("create a build context");
    for (name, bytes) in files {
        fs::write(context.path().join(name), bytes).expect("write the context file");
    }
    fs::write(context.path().join("Doskerfile"), doskerfile).expect("write the Doskerfile");
    context
}

fn copy_helper(source: &Path, name: &str) -> Vec<u8> {
    fs::read(source).unwrap_or_else(|error| panic!("read {name} at {}: {error}", source.display()))
}

fn build(root: &CliRoot, context: &Path) -> Output {
    root.run_with_qemu(&[
        "build",
        "-t",
        BUILT,
        context.to_str().expect("context path is utf-8"),
    ])
}

fn build_with_freedos_source(root: &CliRoot, context: &Path, source: &Path) -> Output {
    let mut command = root.command_with_qemu();
    command
        .env(FREEDOS_SOURCE_ENV, source)
        .args(["build", "-t", BUILT])
        .arg(context);
    run_captured(&mut command, BUILD_TIMEOUT)
}

fn assert_success(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        stdout_text(output),
        stderr_text(output)
    );
}

fn assert_no_image(store: &OciStore) {
    assert!(
        matches!(store.load(&reference(BUILT)), Err(ImageError::NotFound(_))),
        "a failed build commits no image"
    );
}

#[test]
fn run_in_dos_writes_and_deletes_files() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _guard = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);

    let context = build_context(
        &[("KEEP.TXT", b"KEEP\r\n"), ("DROP.TXT", b"DROP\r\n")],
        "FROM base:latest\n\
         COPY KEEP.TXT C:/\n\
         COPY DROP.TXT C:/\n\
         RUN COPY KEEP.TXT MADE.TXT\n\
         RUN DEL DROP.TXT\n\
         CMD ECHO BUILT\n",
    );

    let output = build(&root, context.path());
    assert_success(&output);

    let store = store(&root);
    let tree = store.load_tree(&reference(BUILT)).expect("the tree loads");
    assert_eq!(file_bytes(&tree, r"C:\MADE.TXT"), b"KEEP\r\n".to_vec());
    assert_eq!(file_bytes(&tree, r"C:\KEEP.TXT"), b"KEEP\r\n".to_vec());
    assert!(!has_path(&tree, r"C:\DROP.TXT"), "RUN DEL removed the file");
}

#[test]
fn run_in_dos_non_zero_exit_fails_the_build() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _guard = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);

    let context = build_context(
        &[],
        "FROM base:latest\n\
         RUN C:\\DOSKER\\EXITCODE.EXE 3\n\
         CMD ECHO BUILT\n",
    );

    let output = build(&root, context.path());
    assert_ne!(
        output.status.code(),
        Some(0),
        "a non-zero exit fails the build"
    );
    assert!(
        stderr_text(&output).contains("RUN exited with code 3"),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_no_image(&store(&root));
}

#[test]
fn build_runs_and_rebuilds_from_cache() {
    let Some(image) = prepared_image() else {
        return;
    };
    let _guard = integration_lock();
    let root = CliRoot::new();
    import_base(&root, &image);

    let context = build_context(
        &[("KEEP.TXT", b"KEEP\r\n")],
        "FROM base:latest\n\
         COPY KEEP.TXT C:/\n\
         RUN COPY KEEP.TXT MADE.TXT\n\
         CMD ECHO BUILT\n",
    );

    let first = build(&root, context.path());
    assert_success(&first);
    let first_text = stdout_text(&first);
    assert!(
        first_text.contains("Step 2/3: built"),
        "stdout: {first_text}"
    );

    let store = store(&root);
    let first_layers = layer_digests(&store, BUILT);

    let second = build(&root, context.path());
    assert_success(&second);
    let second_text = stdout_text(&second);
    assert!(
        second_text.contains("Step 2/3: cached"),
        "stdout: {second_text}"
    );
    assert!(
        !second_text.contains("Step 2/3: built"),
        "stdout: {second_text}"
    );

    let second_layers = layer_digests(&store, BUILT);
    assert_eq!(first_layers, second_layers, "the rebuild reuses the layers");
}

#[test]
fn run_in_dos_records_a_boot_metadata_change() {
    if !integration_enabled() {
        return;
    }
    let _guard = integration_lock();

    let source = match freedos_source() {
        Ok(path) => path,
        Err(error) => panic!("{error}"),
    };
    let setboot = match setboot_executable() {
        Ok(path) => path,
        Err(error) => panic!("{error}"),
    };

    let root = CliRoot::new();
    let context = build_context(
        &[("SETBOOT.EXE", &copy_helper(&setboot, "SETBOOT.EXE"))],
        "FROM freedos:1.3\n\
         COPY SETBOOT.EXE C:/\n\
         RUN SETBOOT.EXE\n\
         RUN ECHO AFTER\n\
         CMD ECHO BUILT\n",
    );

    let output = build_with_freedos_source(&root, context.path(), &source);
    assert_success(&output);

    let store = store(&root);
    let built_layers = layers(&store, BUILT);
    let base = store
        .load_effective_boot(&reference("freedos:1.3"))
        .expect("the base boot loads");

    let changed = layer_named(&built_layers, "RUN SETBOOT.EXE")
        .meta
        .boot
        .clone()
        .expect("the changed boot artifacts become an override");
    assert!(
        !base.boot_artifacts_equal(&changed),
        "the override differs from the base boot"
    );

    let after = layer_named(&built_layers, "RUN ECHO AFTER");
    assert!(
        after.meta.boot.is_none(),
        "an unchanged boot writes no override"
    );

    let effective = store
        .load_effective_boot(&reference(BUILT))
        .expect("the built boot loads");
    assert!(
        effective.boot_artifacts_equal(&changed),
        "the second RUN boots the overridden boot artifacts"
    );
    assert!(
        !effective.boot_artifacts_equal(&base),
        "the built image keeps the changed boot"
    );

    let tree = store.load_tree(&reference(BUILT)).expect("the tree loads");
    assert!(
        has_path(&tree, r"C:\SETBOOT.EXE"),
        "the built tree contains SETBOOT.EXE"
    );
}

#[test]
fn run_in_dos_reports_a_guest_reboot() {
    if !integration_enabled() {
        return;
    }
    let _guard = integration_lock();

    let source = match freedos_source() {
        Ok(path) => path,
        Err(error) => panic!("{error}"),
    };
    let reboot = match reboot_executable() {
        Ok(path) => path,
        Err(error) => panic!("{error}"),
    };

    let root = CliRoot::new();
    let context = build_context(
        &[("REBOOT.EXE", &copy_helper(&reboot, "REBOOT.EXE"))],
        "FROM freedos:1.3\n\
         COPY REBOOT.EXE C:/\n\
         RUN REBOOT.EXE\n",
    );

    let output = build_with_freedos_source(&root, context.path(), &source);
    assert_ne!(
        output.status.code(),
        Some(0),
        "a guest reboot fails the build"
    );
    assert!(
        stderr_text(&output).contains("the guest rebooted during the command"),
        "stderr: {}",
        stderr_text(&output)
    );
    assert_no_image(&store(&root));
}
