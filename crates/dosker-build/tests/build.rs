use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use dosker_build::{
    BuildError, BuildOptions, BuildOutcome, Builder, EngineResolver, InDosRunner, ResolvedEngine,
};
use dosker_core::{LOCK_FILE_NAME, LockFile};
use dosker_image::inject::WRAPPED_AUTOEXEC_BYTES;
use dosker_image::{
    DosPath, ImageError, ImageRef, ImageReference, OciStore, TreeNode, read_tree, sha256_digest,
};
use support::{
    FakeResolver, TestReporter, empty_tree, store_booter_base, store_tree_base, write_fat_image,
};

mod support;

const AGENT_A: &[u8] = b"MZ fake agent A";
const AGENT_B: &[u8] = b"MZ fake agent B";

struct MissingEngine;

impl EngineResolver for MissingEngine {
    fn resolve(&self, engine: &str) -> Result<ResolvedEngine, BuildError> {
        Err(BuildError::Engine {
            engine: engine.to_owned(),
            reason: "the engine binary was not found".to_owned(),
        })
    }
}

struct Fixture {
    root: tempfile::TempDir,
    context: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            root: tempfile::tempdir().expect("the data root is created"),
            context: tempfile::tempdir().expect("the context is created"),
        };
        fixture.write("agent/DOSKER.EXE", AGENT_A);
        fixture
    }

    fn root_path(&self) -> &Path {
        self.root.path()
    }

    fn context_path(&self) -> &Path {
        self.context.path()
    }

    fn agent_path(&self) -> PathBuf {
        self.context_path().join("agent/DOSKER.EXE")
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.context.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("the source directory is created");
        }
        fs::write(path, bytes).expect("the fixture file is written");
    }

    fn doskerfile(&self, text: &str) {
        self.write("Doskerfile", text.as_bytes());
    }

    fn store(&self) -> OciStore {
        OciStore::new(self.root.path().to_path_buf())
    }

    fn build(&self, options: &BuildOptions) -> (Result<BuildOutcome, BuildError>, TestReporter) {
        let mut reporter = TestReporter::default();
        let result = builder(self.root.path(), &self.agent_path()).build(
            self.context.path(),
            options,
            &mut reporter,
        );
        (result, reporter)
    }

    #[allow(dead_code)]
    fn build_with_runner(
        &self,
        options: &BuildOptions,
        runner: Box<dyn InDosRunner>,
    ) -> (Result<BuildOutcome, BuildError>, TestReporter) {
        self.build_with_runner_and_resolver(options, Box::new(FakeResolver), runner)
    }

    #[allow(dead_code)]
    fn build_with_runner_and_resolver(
        &self,
        options: &BuildOptions,
        resolver: Box<dyn EngineResolver>,
        runner: Box<dyn InDosRunner>,
    ) -> (Result<BuildOutcome, BuildError>, TestReporter) {
        let mut reporter = TestReporter::default();
        let result = Builder::with_resolver(self.root.path().to_path_buf(), resolver)
            .with_agent_executable(self.agent_path())
            .with_in_dos_runner(runner)
            .build(self.context.path(), options, &mut reporter);
        (result, reporter)
    }

    fn lock_text(&self) -> String {
        fs::read_to_string(self.context.path().join(LOCK_FILE_NAME)).expect("the lock file reads")
    }

    fn lock(&self) -> LockFile {
        LockFile::from_toml(&self.lock_text()).expect("the lock file parses")
    }
}

fn builder(root: &Path, agent: &Path) -> Builder {
    Builder::with_resolver(root.to_path_buf(), Box::new(FakeResolver))
        .with_agent_executable(agent.to_path_buf())
}

fn fixed_now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("the fixed time is valid")
}

fn options(tag: Option<&str>) -> BuildOptions {
    BuildOptions {
        tag: tag.map(|value| ImageReference::parse(value).expect("the tag parses")),
        update_lock: false,
        no_cache: false,
        timeout: Duration::from_secs(30),
        now: fixed_now(),
    }
}

fn tag(value: &str) -> ImageRef {
    ImageRef::parse(value).expect("the reference parses")
}

fn reference(value: &str) -> ImageReference {
    ImageReference::parse(value).expect("the reference parses")
}

fn dos(path: &str) -> DosPath {
    DosPath::parse(path).expect("the DOS path parses")
}

fn tree_has(tree: &dosker_image::Tree, path: &str) -> bool {
    tree.get(&dos(path)).is_some()
}

fn built_tree(fixture: &Fixture) -> dosker_image::Tree {
    fixture
        .store()
        .load_tree(&tag("built:latest"))
        .expect("the built tree loads")
}

fn imports(fixture: &Fixture) -> Vec<dosker_image::Image> {
    fixture
        .store()
        .list()
        .expect("the image list loads")
        .into_iter()
        .filter(|image| image.name == "dosker-build")
        .collect()
}

#[test]
fn build_from_a_stored_tag_applies_copy_run_host_and_config() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        Some("1.2.3"),
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile(
        "FROM base:latest\n\
         COPY HELLO.TXT C:/APP/\n\
         RUN --host echo run-ok && echo made > RAN.TXT\n\
         ENV GREETING=hi\n\
         WORKDIR C:\\APP\n\
         CMD HELLO\n\
         MACHINE memory=32M\n",
    );

    let (result, reporter) = fixture.build(&options(Some("built:latest")));
    let outcome = result.expect("the build succeeds");

    assert_eq!(
        outcome.reference,
        Some(reference("built:latest")),
        "the outcome names the tag"
    );
    assert_eq!(outcome.steps, 6, "six steps follow FROM");
    assert_eq!(outcome.cached_steps, 0, "the first build reuses nothing");

    let tree = built_tree(&fixture);
    assert!(tree_has(&tree, r"C:\APP\HELLO.TXT"), "COPY lands");
    assert!(tree_has(&tree, r"C:\RAN.TXT"), "RUN --host lands");
    assert!(
        reporter.output_text().contains("run-ok"),
        "RUN --host output reaches the reporter"
    );

    let config = fixture
        .store()
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    assert_eq!(config.env.get("GREETING").map(String::as_str), Some("hi"));
    assert_eq!(config.workdir, r"C:\APP");
    assert_eq!(config.cmd.as_deref(), Some("HELLO"));
    assert_eq!(config.machine.memory_mb, 32);
}

#[cfg(unix)]
#[test]
fn run_host_preserves_a_quoted_glob_in_the_command() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.write("ONE.TXT", b"one");
    fixture.write("TWO.TXT", b"two");
    fixture.doskerfile(
        "FROM base:latest\n\
         COPY ONE.TXT C:/\n\
         COPY TWO.TXT C:/\n\
         RUN --host printf '%s' \"*.TXT\" > GLOB.TXT\n",
    );

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let tree = built_tree(&fixture);
    match tree.get(&dos(r"C:\GLOB.TXT")) {
        Some(TreeNode::File(file)) => assert_eq!(
            file.bytes, b"*.TXT",
            "the quoted glob reaches the shell as a literal"
        ),
        other => panic!("expected GLOB.TXT, got {other:?}"),
    }
}

#[cfg(windows)]
#[test]
fn run_host_preserves_a_windows_path_in_the_command() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.doskerfile("FROM base:latest\nRUN --host echo C:\\APP\\BIN\\APP.EXE> PATH.TXT\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let tree = built_tree(&fixture);
    match tree.get(&dos(r"C:\PATH.TXT")) {
        Some(TreeNode::File(file)) => assert_eq!(
            String::from_utf8_lossy(&file.bytes).trim(),
            r"C:\APP\BIN\APP.EXE",
            "the Windows path keeps its single backslashes"
        ),
        other => panic!("expected PATH.TXT, got {other:?}"),
    }
}

#[test]
fn build_persists_a_raw_cmd_dos_path() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.doskerfile("FROM base:latest\nCMD C:\\APP\\BIN\\APP.EXE /FAST\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let config = fixture
        .store()
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    assert_eq!(
        config.cmd.as_deref(),
        Some(r"C:\APP\BIN\APP.EXE /FAST"),
        "the raw CMD text keeps its single backslashes"
    );
    assert_eq!(config.created_by, r"CMD C:\APP\BIN\APP.EXE /FAST");
}

#[test]
fn build_wraps_a_config_normalize_error_with_the_line() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.doskerfile("FROM base:latest\nWORKDIR D:\\APP\n");

    match fixture.build(&options(Some("built:latest"))).0 {
        Err(BuildError::Instruction {
            line,
            instruction,
            reason,
        }) => {
            assert_eq!(line, 2, "the error names the line");
            assert_eq!(instruction, "WORKDIR");
            assert!(
                reason.contains("D:"),
                "the reason names the drive: {reason}"
            );
        }
        other => panic!("expected an instruction error, got {other:?}"),
    }
}

#[test]
fn build_from_a_digest_reference_pins_the_digest() {
    let fixture = Fixture::new();
    let digest = store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    fixture.doskerfile(&format!("FROM base@{digest}\nENV A=1\n"));

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let lock = fixture.lock();
    assert_eq!(lock.version, 1);
    assert_eq!(lock.base.len(), 1);
    assert_eq!(lock.base[0].name, "base");
    assert_eq!(lock.base[0].tag, "", "a digest base has an empty tag");
    assert_eq!(lock.base[0].digest, digest);
}

#[test]
fn build_from_a_namespaced_local_tag_records_the_full_base_pin() {
    let fixture = Fixture::new();
    let _digest = store_tree_base(
        fixture.root_path(),
        "owner/name:tag",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    fixture.doskerfile("FROM owner/name:tag\nENV A=1\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let lock = fixture.lock();
    assert_eq!(lock.base.len(), 1);
    assert_eq!(lock.base[0].name, "owner/name");
    assert_eq!(lock.base[0].tag, "tag");
}

#[test]
fn build_from_a_namespaced_local_digest_pins_an_empty_tag() {
    let fixture = Fixture::new();
    let digest = store_tree_base(
        fixture.root_path(),
        "owner/name:tag",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    fixture.doskerfile(&format!("FROM owner/name@{digest}\nENV A=1\n"));

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let lock = fixture.lock();
    assert_eq!(lock.base[0].name, "owner/name");
    assert_eq!(lock.base[0].tag, "");
    assert_eq!(lock.base[0].digest, digest);
}

#[test]
fn build_from_a_host_qualified_local_tag_pins_the_host_and_repository() {
    let fixture = Fixture::new();
    let _digest = store_tree_base(
        fixture.root_path(),
        "owner/name:tag",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    fixture.doskerfile("FROM example.com/owner/name:tag\nENV A=1\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let lock = fixture.lock();
    assert_eq!(lock.base[0].name, "example.com/owner/name");
    assert_eq!(lock.base[0].tag, "tag");
}

#[test]
fn build_from_a_bare_digest_keeps_an_empty_pin_name() {
    let fixture = Fixture::new();
    let digest = store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    fixture.doskerfile(&format!("FROM {digest}\nENV A=1\n"));

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let lock = fixture.lock();
    assert_eq!(lock.base[0].name, "");
    assert_eq!(lock.base[0].tag, "");
    assert_eq!(lock.base[0].digest, digest);
}

#[test]
fn build_rejects_a_missing_namespaced_base_with_a_pull_hint() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM owner/missing:latest\nENV A=1\n");

    match fixture.build(&options(Some("built:latest"))).0 {
        Err(BuildError::RegistryBaseNotLocal { reference }) => {
            assert_eq!(reference, "owner/missing:latest");
        }
        _ => panic!("expected a registry base error"),
    }
}

#[test]
fn build_from_a_booter_tag_changes_only_config() {
    let fixture = Fixture::new();
    store_booter_base(fixture.root_path(), "boot:latest", b"BOOTER");
    fixture.doskerfile("FROM boot:latest\nMACHINE memory=32M\n");

    let first = fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the first build succeeds");
    let (result, _) = fixture.build(&options(Some("built:latest")));
    let second = result.expect("the second build succeeds");
    assert_eq!(second.steps, 1);
    assert_eq!(
        second.cached_steps, second.steps,
        "every config step is cached on a booter base"
    );
    assert_eq!(first.manifest_digest, second.manifest_digest);

    let store = fixture.store();
    let config = store
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    assert!(config.booter, "the base stays booter");
    assert_eq!(config.machine.memory_mb, 32);

    let base = store
        .load_manifest(&tag("boot:latest"))
        .expect("the base manifest loads");
    let built = store
        .load_manifest(&tag("built:latest"))
        .expect("the built manifest loads");
    assert_eq!(built.layers, base.layers, "config changes add no layer");
    assert_eq!(built.raw_disk_digest(), None);
}

#[test]
fn build_imports_a_local_disk_image() {
    let fixture = Fixture::new();
    write_fat_image(&fixture.context_path().join("base.img"));
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM ./base.img\nCOPY HELLO.TXT C:/APP/\n");

    let (result, _) = fixture.build(&options(Some("built:latest")));
    result.expect("the build succeeds");

    let tree = built_tree(&fixture);
    assert!(tree_has(&tree, r"C:\APP\HELLO.TXT"), "COPY applies");
    assert!(
        tree_has(&tree, r"C:\DOSKER\DOSKER.EXE"),
        "the agent is injected"
    );
    assert!(
        tree_has(&tree, r"C:\DOSKER\AUTOEXEC.OLD"),
        "the original startup batch is saved"
    );
    match tree.get(&dos(r"C:\AUTOEXEC.BAT")) {
        Some(TreeNode::File(file)) => assert_eq!(file.bytes, WRAPPED_AUTOEXEC_BYTES),
        other => panic!("expected the wrapped AUTOEXEC.BAT, got {other:?}"),
    }
    match tree.get(&dos(r"C:\DOSKER\DOSKER.EXE")) {
        Some(TreeNode::File(file)) => assert_eq!(file.bytes, AGENT_A),
        other => panic!("expected the injected agent, got {other:?}"),
    }

    let manifest = fixture
        .store()
        .load_manifest(&tag("built:latest"))
        .expect("the built manifest loads");
    assert!(
        manifest.raw_disk_digest().is_some(),
        "the import keeps the raw disk"
    );
    assert_eq!(imports(&fixture).len(), 1, "the import is stored once");
}

#[test]
fn build_from_a_local_import_pins_the_agent() {
    let fixture = Fixture::new();
    write_fat_image(&fixture.context_path().join("base.img"));
    fixture.doskerfile("FROM ./base.img\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let lock = fixture.lock();
    assert_eq!(
        lock.agent.as_ref().map(|agent| agent.version.as_str()),
        Some("1.0.0")
    );
}

#[test]
fn build_reimports_a_local_image_when_the_agent_changes() {
    let fixture = Fixture::new();
    write_fat_image(&fixture.context_path().join("base.img"));
    fixture.doskerfile("FROM ./base.img\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the first build succeeds");

    fixture.write("agent/DOSKER.EXE", AGENT_B);
    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the second build succeeds");

    assert_eq!(
        imports(&fixture).len(),
        2,
        "a changed agent produces a new import"
    );
    let tree = built_tree(&fixture);
    match tree.get(&dos(r"C:\DOSKER\DOSKER.EXE")) {
        Some(TreeNode::File(file)) => assert_eq!(file.bytes, AGENT_B),
        other => panic!("expected the second agent, got {other:?}"),
    }
}

#[test]
fn build_reimports_a_pre_slice_local_import() {
    let fixture = Fixture::new();
    let source = fixture.context_path().join("base.img");
    write_fat_image(&source);
    let source_bytes = fs::read(&source).expect("the source reads");
    let source_digest = sha256_digest(&source_bytes);
    let hex = source_digest
        .strip_prefix("sha256:")
        .expect("the source digest is prefixed");
    let old_reference = format!("dosker-build:{hex}");
    store_tree_base(
        fixture.root_path(),
        &old_reference,
        &read_tree(&source).expect("the source tree reads"),
        Some(&source_bytes),
        None,
    );
    fixture.doskerfile("FROM ./base.img\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let tree = built_tree(&fixture);
    assert!(
        tree_has(&tree, r"C:\DOSKER\DOSKER.EXE"),
        "the combined import injects the agent"
    );
    assert!(
        fixture.store().load(&tag(&old_reference)).is_ok(),
        "the old source-only tag remains"
    );
}

#[test]
fn build_reimports_a_combined_tag_without_the_source_annotation() {
    let fixture = Fixture::new();
    let source = fixture.context_path().join("base.img");
    write_fat_image(&source);
    let source_bytes = fs::read(&source).expect("the source reads");
    let source_digest = sha256_digest(&source_bytes);
    let agent_bytes = fs::read(fixture.agent_path()).expect("the agent reads");
    let agent_digest = sha256_digest(&agent_bytes);
    let combined = sha256_digest(format!("{source_digest}\n{agent_digest}").as_bytes());
    let hex = combined
        .strip_prefix("sha256:")
        .expect("the combined digest is prefixed");
    let combined_reference = format!("dosker-build:{hex}");
    store_tree_base(
        fixture.root_path(),
        &combined_reference,
        &read_tree(&source).expect("the source tree reads"),
        Some(&source_bytes),
        Some("1.0.0"),
    );
    fixture.doskerfile("FROM ./base.img\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let tree = built_tree(&fixture);
    assert!(
        tree_has(&tree, r"C:\DOSKER\DOSKER.EXE"),
        "the re-import injects the agent"
    );
    let manifest = fixture
        .store()
        .load_manifest(&tag(&combined_reference))
        .expect("the combined tag loads");
    assert_eq!(
        manifest.source_disk_digest(),
        Some(source_digest.as_str()),
        "the re-import records the source annotation"
    );
}

#[test]
fn build_second_run_from_a_local_import_reuses_the_image() {
    let fixture = Fixture::new();
    write_fat_image(&fixture.context_path().join("base.img"));
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM ./base.img\nCOPY HELLO.TXT C:/APP/\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the first build succeeds");
    let (result, _) = fixture.build(&options(Some("built:latest")));
    let second = result.expect("the second build succeeds");

    assert_eq!(second.cached_steps, second.steps, "every step is cached");
    assert_eq!(imports(&fixture).len(), 1, "the import is reused");
}

#[test]
fn build_imports_a_local_booter_image() {
    let fixture = Fixture::new();
    fixture.write("boot.bin", b"OPAQUE");
    fixture.doskerfile("FROM booter:./boot.bin\nENGINE qemu\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let store = fixture.store();
    let config = store
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    assert!(config.booter);
    let manifest = store
        .load_manifest(&tag("built:latest"))
        .expect("the built manifest loads");
    assert_eq!(manifest.layers.len(), 1);
    assert_eq!(manifest.raw_disk_digest(), None);
    assert_eq!(imports(&fixture).len(), 1);
}

#[test]
fn build_rejects_a_missing_local_image() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM ./missing.img\n");

    let (result, _) = fixture.build(&options(Some("built:latest")));
    match result {
        Err(BuildError::Io { path, .. }) => {
            assert!(
                path.to_string_lossy().ends_with("missing.img"),
                "the error names the path: {}",
                path.display()
            );
        }
        other => panic!("expected an i/o error, got {other:?}"),
    }
    assert!(
        !fixture.context_path().join(LOCK_FILE_NAME).exists(),
        "a failed build writes no lock"
    );
}

#[test]
fn build_rejects_an_in_image_run_without_an_agent() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.doskerfile("FROM base:latest\nRUN echo hi\n");

    let (result, _) = fixture.build(&options(Some("built:latest")));
    match result {
        Err(BuildError::RunBootFailed { line, reason }) => {
            assert_eq!(line, 2);
            assert!(
                reason.contains("no agent"),
                "the reason names the missing agent: {reason}"
            );
        }
        other => panic!("expected a RUN boot error, got {other:?}"),
    }
}

#[test]
fn build_rejects_a_booter_copy_and_run_host() {
    let copy = Fixture::new();
    store_booter_base(copy.root_path(), "boot:latest", b"BOOTER");
    copy.write("HELLO.TXT", b"hello");
    copy.doskerfile("FROM boot:latest\nCOPY HELLO.TXT C:/APP/\n");
    match copy.build(&options(Some("built:latest"))).0 {
        Err(BuildError::Instruction {
            instruction,
            reason,
            ..
        }) => {
            assert_eq!(instruction, "COPY");
            assert!(
                reason.contains("booter"),
                "the reason names the booter base: {reason}"
            );
        }
        other => panic!("expected a COPY instruction error, got {other:?}"),
    }

    let run = Fixture::new();
    store_booter_base(run.root_path(), "boot:latest", b"BOOTER");
    run.doskerfile("FROM boot:latest\nRUN --host echo hi\n");
    match run.build(&options(Some("built:latest"))).0 {
        Err(BuildError::Instruction {
            instruction,
            reason,
            ..
        }) => {
            assert_eq!(instruction, "RUN --host");
            assert!(
                reason.contains("booter"),
                "the reason names the booter base: {reason}"
            );
        }
        other => panic!("expected a RUN --host instruction error, got {other:?}"),
    }
}

#[test]
fn build_writes_the_lock_file() {
    let fixture = Fixture::new();
    let digest = store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        Some("1.2.3"),
    );
    fixture.doskerfile("FROM base:latest\nENV A=1\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let text = fixture.lock_text();
    assert!(text.ends_with('\n'), "the lock file ends with a newline");
    let lock = LockFile::from_toml(&text).expect("the lock file parses");
    assert_eq!(lock.version, 1);
    assert_eq!(lock.base[0].name, "base");
    assert_eq!(lock.base[0].tag, "latest");
    assert_eq!(lock.base[0].digest, digest);
    let engine = lock.engines.get("qemu").expect("the engine is pinned");
    assert_eq!(engine.version, "qemu 9.0.0");
    assert_eq!(engine.source, "path");
    assert_eq!(
        lock.agent.as_ref().map(|agent| agent.version.as_str()),
        Some("1.2.3")
    );
}

#[test]
fn build_writes_the_image_manifest_and_layers() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nCOPY HELLO.TXT C:/APP/\n");

    let (result, _) = fixture.build(&options(Some("built:latest")));
    let outcome = result.expect("the build succeeds");

    let store = fixture.store();
    let base = store
        .load_manifest(&tag("base:latest"))
        .expect("the base manifest loads");
    let built = store
        .load_manifest(&tag("built:latest"))
        .expect("the built manifest loads");
    assert_eq!(built.layers.len(), base.layers.len() + 1);
    let config = store
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    assert_eq!(
        outcome.config_digest,
        config.digest().expect("the config digests")
    );
    assert!(tree_has(&built_tree(&fixture), r"C:\APP\HELLO.TXT"));
}

#[test]
fn build_keeps_the_base_raw_disk_annotation() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        Some(b"DISK"),
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nCOPY HELLO.TXT C:/APP/\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let store = fixture.store();
    let base = store
        .load_manifest(&tag("base:latest"))
        .expect("the base manifest loads");
    let built = store
        .load_manifest(&tag("built:latest"))
        .expect("the built manifest loads");
    assert_eq!(built.raw_disk_digest(), base.raw_disk_digest());
    assert_eq!(
        built.raw_disk_digest(),
        Some(sha256_digest(b"DISK").as_str())
    );
}

#[test]
fn build_records_a_history_entry_for_each_instruction() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nENV A=1\nCOPY HELLO.TXT C:/\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let config = fixture
        .store()
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    assert_eq!(
        config.history.len(),
        3,
        "one base entry and two instructions"
    );
}

#[test]
fn config_only_history_entries_are_empty_layers() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nENV A=1\nCOPY HELLO.TXT C:/\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let config = fixture
        .store()
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    let env = config
        .history
        .iter()
        .find(|entry| entry.created_by == "ENV A=1")
        .expect("the ENV entry exists");
    assert!(env.empty_layer, "config-only entries are empty layers");
    let copy = config
        .history
        .iter()
        .find(|entry| entry.created_by.starts_with("COPY"))
        .expect("the COPY entry exists");
    assert!(!copy.empty_layer, "filesystem entries are not empty layers");
}

#[test]
fn build_sets_created_by_to_the_last_instruction() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.doskerfile("FROM base:latest\nENV A=1\nCMD HELLO\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let config = fixture
        .store()
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    assert_eq!(config.created_by, "CMD HELLO");
}

#[test]
fn workdir_directory_appears_after_a_later_copy() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nWORKDIR C:\\APP\nCOPY HELLO.TXT C:/APP/\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let tree = built_tree(&fixture);
    assert!(
        matches!(tree.get(&dos(r"C:\APP")), Some(TreeNode::Dir(_))),
        "the WORKDIR directory materializes through the later COPY"
    );
    assert!(tree_has(&tree, r"C:\APP\HELLO.TXT"));
    let config = fixture
        .store()
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    assert_eq!(config.workdir, r"C:\APP");
}

#[test]
fn a_build_ending_after_workdir_keeps_the_config_only() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.doskerfile("FROM base:latest\nWORKDIR C:\\APP\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the build succeeds");

    let store = fixture.store();
    let config = store
        .load_config(&tag("built:latest"))
        .expect("the built config loads");
    assert_eq!(config.workdir, r"C:\APP");
    assert!(
        config.history.last().is_some_and(|entry| entry.empty_layer),
        "the WORKDIR entry is empty"
    );
    let tree = built_tree(&fixture);
    assert!(
        !tree_has(&tree, r"C:\APP"),
        "WORKDIR adds no empty directory to the image tree"
    );
    let base = store
        .load_manifest(&tag("base:latest"))
        .expect("the base manifest loads");
    let built = store
        .load_manifest(&tag("built:latest"))
        .expect("the built manifest loads");
    assert_eq!(
        built.layers.len(),
        base.layers.len(),
        "WORKDIR adds no layer"
    );
}

#[test]
fn workdir_then_copy_second_build_reports_a_cache_hit() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nWORKDIR C:\\APP\nCOPY HELLO.TXT C:/APP/\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the first build succeeds");
    let (result, _) = fixture.build(&options(Some("built:latest")));
    let second = result.expect("the second build succeeds");

    assert_eq!(second.steps, 2);
    assert_eq!(second.cached_steps, 2, "both steps are cached");
}

#[test]
fn build_second_run_reports_cached_for_every_step() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nCOPY HELLO.TXT C:/\nENV A=1\n");

    let first = fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the first build succeeds");
    let second = fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the second build succeeds");

    assert_eq!(second.cached_steps, second.steps);
    assert_eq!(first.manifest_digest, second.manifest_digest);
}

#[test]
fn copy_dot_second_build_reports_a_cache_hit() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nCOPY . C:/APP/\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the first build succeeds");
    let (result, _) = fixture.build(&options(Some("built:latest")));
    let second = result.expect("the second build succeeds");

    assert_eq!(second.steps, 1);
    assert_eq!(
        second.cached_steps, 1,
        "the lock file does not invalidate COPY ."
    );
}

#[test]
fn no_cache_rebuilds_every_step() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nCOPY HELLO.TXT C:/\nENV A=1\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the first build succeeds");
    let mut rebuild = options(Some("built:latest"));
    rebuild.no_cache = true;
    let second = fixture
        .build(&rebuild)
        .0
        .expect("the second build succeeds");

    assert_eq!(second.steps, 2);
    assert_eq!(second.cached_steps, 0, "no-cache rebuilds every step");
}

#[test]
fn env_change_invalidates_later_steps() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.write("HELLO.TXT", b"hello");
    fixture.doskerfile("FROM base:latest\nENV A=1\nCOPY HELLO.TXT C:/\n");

    fixture
        .build(&options(Some("built:latest")))
        .0
        .expect("the first build succeeds");

    fixture.doskerfile("FROM base:latest\nENV A=2\nCOPY HELLO.TXT C:/\n");
    let (result, _) = fixture.build(&options(Some("built:latest")));
    let second = result.expect("the second build succeeds");

    assert_eq!(second.steps, 2);
    assert_eq!(
        second.cached_steps, 0,
        "the ENV change invalidates the COPY"
    );
}

#[test]
fn a_failed_instruction_writes_no_image_or_lock() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.doskerfile("FROM base:latest\nCOPY MISSING.TXT C:/\n");

    let (result, _) = fixture.build(&options(Some("built:latest")));
    assert!(result.is_err(), "the build fails");

    assert!(
        matches!(
            fixture.store().load(&tag("built:latest")),
            Err(ImageError::NotFound(_))
        ),
        "no image is written"
    );
    assert!(
        !fixture.context_path().join(LOCK_FILE_NAME).exists(),
        "no lock is written"
    );
}

#[test]
fn build_without_the_engine_fails() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.doskerfile("FROM base:latest\nENV A=1\n");

    let mut reporter = TestReporter::default();
    let result = Builder::with_resolver(fixture.root_path().to_path_buf(), Box::new(MissingEngine))
        .build(
            fixture.context_path(),
            &options(Some("built:latest")),
            &mut reporter,
        );

    match result {
        Err(BuildError::Engine { engine, .. }) => assert_eq!(engine, "qemu"),
        other => panic!("expected an engine error, got {other:?}"),
    }
    assert!(
        matches!(
            fixture.store().load(&tag("built:latest")),
            Err(ImageError::NotFound(_))
        ),
        "no image is written"
    );
    assert!(!fixture.context_path().join(LOCK_FILE_NAME).exists());
}

#[test]
fn build_reports_progress_lines() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        None,
    );
    fixture.doskerfile("FROM base:latest\nENV A=1\n");

    let (result, reporter) = fixture.build(&options(Some("built:latest")));
    result.expect("the build succeeds");

    assert_eq!(
        reporter.lines.first().map(String::as_str),
        Some("FROM base:latest")
    );
    assert!(reporter.lines.contains(&"Step 1/1: ENV A=1".to_owned()));
    assert!(reporter.lines.contains(&"Step 1/1: built".to_owned()));
}
