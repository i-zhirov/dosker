//! End-to-end checks for `dosker build`.

mod common;

use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::Output;
use std::time::Duration;

use common::{CliRoot, run_captured, stderr_text, stdout_text};
#[cfg(unix)]
use dosker_core::{AgentPin, BasePin, ContainerId, LOCK_FILE_NAME, LockFile};
#[cfg(unix)]
use dosker_image::{DosPath, ImageDigest, ImageRef, ImageReference, OciStore, TreeNode, read_tree};

const RUN_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(unix)]
const VERSION_LINE: &str = "QEMU emulator version 11.1.3";

struct Fixture {
    root: CliRoot,
    context: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = CliRoot::new();
        root.import_image_as("base:latest");
        let context = tempfile::tempdir().expect("create a build context");
        Self { root, context }
    }

    fn context_path(&self) -> &Path {
        self.context.path()
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.context_path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the source directory");
        }
        fs::write(path, bytes).expect("write the fixture file");
    }

    fn doskerfile(&self, text: &str) {
        self.write("Doskerfile", text.as_bytes());
    }

    #[cfg(unix)]
    fn store(&self) -> OciStore {
        OciStore::new(self.root.path().to_path_buf())
    }

    #[cfg(unix)]
    fn manifest_digest(&self, reference: &str) -> String {
        let reference = ImageReference::parse(reference).expect("the reference parses");
        self.store()
            .load(&ImageRef::Tag(reference))
            .expect("the image loads")
            .manifest_digest
    }

    #[cfg(unix)]
    fn base_digest(&self) -> String {
        self.manifest_digest("base:latest")
    }

    #[cfg(unix)]
    fn lock(&self) -> LockFile {
        LockFile::from_toml(&self.lock_text()).expect("the lock file parses")
    }

    #[cfg(unix)]
    fn lock_text(&self) -> String {
        fs::read_to_string(self.context_path().join(LOCK_FILE_NAME)).expect("the lock file reads")
    }

    #[cfg(unix)]
    fn write_lock(&self, lock: &LockFile) {
        let text = lock.to_toml().expect("the lock file serializes");
        fs::write(self.context_path().join(LOCK_FILE_NAME), text).expect("the lock file writes");
    }

    fn run_build(&self, args: &[&str], qemu: &Path) -> Output {
        let mut command = self.root.base_command();
        command.env("DOSKER_QEMU", qemu);
        command.arg("build");
        command.args(args);
        command.arg(self.context_path());
        run_captured(&mut command, RUN_TIMEOUT)
    }

    #[cfg(unix)]
    fn build(&self, args: &[&str]) -> Output {
        let dir = tempfile::tempdir().expect("create a qemu script directory");
        let qemu = write_qemu(dir.path());
        self.run_build(args, &qemu)
    }
}

#[cfg(unix)]
fn write_qemu(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("qemu-system-i386");
    fs::write(&path, format!("#!/bin/sh\necho '{VERSION_LINE}'\n")).expect("write the qemu script");
    let mut permissions = fs::metadata(&path)
        .expect("read the script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("set the script permissions");
    path
}

#[cfg(unix)]
fn built_digest(output: &Output) -> String {
    let text = stdout_text(output);
    let line = text
        .lines()
        .find(|line| line.starts_with("built "))
        .expect("the build prints a result line");
    let digest = line
        .split_whitespace()
        .last()
        .expect("the result line ends with a digest");
    assert!(digest.starts_with("sha256:"), "result line: {line}");
    digest.to_owned()
}

#[cfg(unix)]
#[test]
fn build_prints_a_tag_and_the_manifest_digest() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM base:latest\nENV A=1\n");

    let output = fixture.build(&["-t", "built:latest"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "FROM base:latest", "stdout: {text}");
    assert!(
        lines.contains(&"Step 1/1: ENV A=1"),
        "the step instruction is printed: {text}"
    );
    assert!(
        lines.contains(&"Step 1/1: built"),
        "the step status is printed: {text}"
    );
    let expected = format!(
        "built built:latest {}",
        fixture.manifest_digest("built:latest")
    );
    assert_eq!(lines.last().copied(), Some(expected.as_str()));
}

#[cfg(unix)]
#[test]
fn build_without_a_tag_prints_the_manifest_digest() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM base:latest\nENV A=1\n");

    let output = fixture.build(&[]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let result = text.lines().last().expect("the build prints a result line");
    let digest = result
        .strip_prefix("built ")
        .expect("the result line starts with built");
    let reference = ImageRef::Digest(ImageDigest::parse(digest).expect("the digest parses"));
    let image = fixture
        .store()
        .load(&reference)
        .expect("the printed digest resolves");
    assert_eq!(image.manifest_digest, digest);
}

#[cfg(unix)]
#[test]
fn build_timeout_overrides_the_default() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM base:latest\nRUN --host sleep 5\n");

    let output = fixture.build(&["--timeout", "1"]);

    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("timed out after 1 seconds"),
        "stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn build_lists_the_tag_in_images() {
    let fixture = Fixture::new();
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nCOPY HELLO.TXT C:/\n");

    let build = fixture.build(&["-t", "built:latest"]);
    assert_eq!(
        build.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&build)
    );
    let digest = fixture.manifest_digest("built:latest");

    let output = fixture.root.run(&["images"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    let expected = format!("built:latest {digest} ");
    assert!(
        text.lines().any(|line| line.starts_with(&expected)),
        "the built tag is listed: {text}"
    );
}

#[cfg(unix)]
#[test]
fn build_writes_a_version_one_lock_file() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM base:latest\nENV A=1\n");

    let output = fixture.build(&["-t", "built:latest"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = fixture.lock_text();
    assert!(text.ends_with('\n'), "the lock file ends with a newline");
    let lock = fixture.lock();
    assert_eq!(lock.version, 1);
    assert_eq!(
        lock.base,
        vec![BasePin {
            name: "base".to_owned(),
            tag: "latest".to_owned(),
            digest: fixture.base_digest(),
        }]
    );
    let engine = lock.engines.get("qemu").expect("the engine is pinned");
    assert_eq!(engine.version, "11.1.3");
    assert_eq!(engine.source, "env");
    assert_eq!(
        lock.agent,
        Some(AgentPin {
            version: "1.0.0".to_owned()
        })
    );
}

#[cfg(unix)]
#[test]
fn build_reports_cached_steps_on_a_second_run() {
    let fixture = Fixture::new();
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nCOPY HELLO.TXT C:/\n");

    let first = fixture.build(&["-t", "built:latest"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&first)
    );
    assert!(stdout_text(&first).contains("Step 1/1: built"));

    let second = fixture.build(&["-t", "built:latest"]);

    assert_eq!(
        second.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&second)
    );
    let text = stdout_text(&second);
    assert!(text.contains("Step 1/1: cached"), "stdout: {text}");
    assert!(!text.contains("Step 1/1: built"), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn build_env_change_reports_later_steps_built() {
    let fixture = Fixture::new();
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nENV A=1\nCOPY HELLO.TXT C:/\n");

    let first = fixture.build(&["-t", "built:latest"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&first)
    );

    fixture.doskerfile("FROM base:latest\nENV A=2\nCOPY HELLO.TXT C:/\n");
    let second = fixture.build(&["-t", "built:latest"]);

    assert_eq!(
        second.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&second)
    );
    let text = stdout_text(&second);
    assert!(text.contains("Step 1/2: built"), "stdout: {text}");
    assert!(text.contains("Step 2/2: built"), "stdout: {text}");
    assert!(!text.contains("cached"), "stdout: {text}");
}

#[cfg(unix)]
#[test]
fn build_rejects_a_bad_8_3_name_with_the_path() {
    let fixture = Fixture::new();
    fixture.write("LONGFILENAME.TXT", b"long");
    fixture.doskerfile("FROM base:latest\nCOPY LONGFILENAME.TXT C:/\n");

    let output = fixture.build(&[]);

    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("LONGFILENAME.TXT"), "stderr: {stderr}");
    assert!(stderr.contains("8.3"), "stderr: {stderr}");
}

#[cfg(unix)]
#[test]
fn build_rejects_a_symlink_with_the_path() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    fixture.write("target.txt", b"target");
    symlink(
        fixture.context_path().join("target.txt"),
        fixture.context_path().join("link.txt"),
    )
    .expect("create the symlink");
    fixture.doskerfile("FROM base:latest\nCOPY link.txt C:/\n");

    let output = fixture.build(&[]);

    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("link.txt"), "stderr: {stderr}");
    assert!(stderr.contains("symlink"), "stderr: {stderr}");
}

#[cfg(unix)]
#[test]
fn build_rejects_a_traversal_path_with_the_path() {
    let fixture = Fixture::new();
    fixture.write("one.txt", b"one");
    fixture.doskerfile("FROM base:latest\nCOPY ../secret.txt C:/\n");

    let output = fixture.build(&[]);

    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("../secret.txt"), "stderr: {stderr}");
}

#[test]
fn build_rejects_a_bad_instruction_with_the_line() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM base:latest\nBOGUS value\n");
    let missing = fixture.root.path().join("missing-qemu");

    let output = fixture.run_build(&[], &missing);

    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("Doskerfile line 2"), "stderr: {stderr}");
}

#[cfg(unix)]
#[test]
fn build_lock_mismatch_stops_and_update_refreshes() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM base:latest\nENV A=1\n");
    let first = fixture.build(&["-t", "built:latest"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&first)
    );

    let mut lock = fixture.lock();
    lock.base[0].digest = format!("sha256:{}", "0".repeat(64));
    fixture.write_lock(&lock);

    let stopped = fixture.build(&["-t", "built:latest"]);

    assert_eq!(stopped.status.code(), Some(1));
    let stderr = stderr_text(&stopped);
    assert!(stderr.contains("digest mismatch"), "stderr: {stderr}");

    let refreshed = fixture.build(&["-t", "built:latest", "--update"]);

    assert_eq!(
        refreshed.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&refreshed)
    );
    assert_eq!(fixture.lock().base[0].digest, fixture.base_digest());
}

#[cfg(unix)]
#[test]
fn build_no_cache_rebuilds() {
    let fixture = Fixture::new();
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nCOPY HELLO.TXT C:/\n");

    let first = fixture.build(&["-t", "built:latest"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&first)
    );

    let second = fixture.build(&["-t", "built:latest", "--no-cache"]);

    assert_eq!(
        second.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&second)
    );
    let text = stdout_text(&second);
    assert!(text.contains("Step 1/1: built"), "stdout: {text}");
    assert!(!text.contains("cached"), "stdout: {text}");
}

#[test]
fn build_without_qemu_fails() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM base:latest\nENV A=1\n");
    let missing = fixture.root.path().join("missing-qemu");

    let output = fixture.run_build(&[], &missing);

    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_text(&output);
    assert!(stderr.contains("engine qemu"), "stderr: {stderr}");
    assert!(stderr.contains("not found"), "stderr: {stderr}");
    assert!(
        stdout_text(&output).is_empty(),
        "no progress is printed: {}",
        stdout_text(&output)
    );
}

#[cfg(unix)]
#[test]
fn build_prints_run_host_output() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM base:latest\nRUN --host echo host-output\n");

    let output = fixture.build(&["-t", "built:latest"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let text = stdout_text(&output);
    assert!(text.contains("host-output"), "stdout: {text}");
    assert!(text.contains("Step 1/1: built"), "stdout: {text}");
    assert_eq!(built_digest(&output).len(), "sha256:".len() + 64);
}

#[cfg(unix)]
#[test]
fn create_projects_copy_layers_onto_the_private_disk() {
    let fixture = Fixture::new();
    let payload = b"projected\r\n";
    fixture.write("ADD.TXT", payload);
    fixture.doskerfile("FROM base:latest\nCOPY ADD.TXT C:/APPS/\n");

    let build = fixture.build(&["-t", "built:latest"]);
    assert_eq!(
        build.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&build)
    );

    let output = fixture
        .root
        .run(&["create", "built:latest", "--name", "projected"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    let id = ContainerId::parse(stdout_text(&output).trim()).expect("parse the container id");
    let record = fixture
        .root
        .store()
        .load(&id)
        .expect("load the created container");
    let disk = record.disk_path.expect("the record has a private disk");

    let tree = read_tree(&disk).expect("read the projected container disk");
    let path = DosPath::parse(r"C:\APPS\ADD.TXT").expect("the DOS path parses");
    match tree.get(&path) {
        Some(TreeNode::File(file)) => {
            assert_eq!(file.bytes.as_slice(), payload);
        }
        Some(TreeNode::Dir(_)) => panic!("the COPY destination is a directory"),
        None => panic!("the projected disk has no COPY destination"),
    }
}

#[cfg(unix)]
#[test]
fn create_rejects_a_tree_that_does_not_fit_the_disk() {
    let fixture = Fixture::new();
    let payload = vec![b'x'; 40 * 1024 * 1024];
    fixture.write("BIG.TXT", &payload);
    fixture.doskerfile("FROM base:latest\nCOPY BIG.TXT C:/\n");

    let build = fixture.build(&["-t", "built:latest"]);
    assert_eq!(
        build.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&build)
    );

    let output = fixture
        .root
        .run(&["create", "built:latest", "--name", "overflow"]);
    assert_eq!(output.status.code(), Some(1));

    let ps = fixture.root.run(&["ps", "-a"]);
    assert_eq!(ps.status.code(), Some(0), "stderr: {}", stderr_text(&ps));
    assert!(
        !stdout_text(&ps).contains("overflow"),
        "the failed create left a container: {}",
        stdout_text(&ps)
    );
}
