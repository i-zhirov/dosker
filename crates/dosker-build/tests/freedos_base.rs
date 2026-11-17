mod support;

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use dosker_build::{BuildError, BuildOptions, BuildOutcome, Builder};
use dosker_core::{BasePin, LOCK_FILE_NAME, LockFile};
use dosker_image::freedos::{
    BaseImage, FreedosPins, FreedosVersion, build_base_image, load_freedos_binaries,
};
use dosker_image::{
    ANNOTATION_RAW_DISK_DIGEST, BaseInputs, DosFlavour, ImageError, ImageRef, ImageReference,
    Layer, OciStore,
};
use support::{FakeResolver, TestReporter, synthetic_inputs, write_agent, write_freedos_source};

const SOURCE_ENV: &str = "DOSKER_FREEDOS_SOURCE";

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn with_source_env<T>(value: Option<&Path>, action: impl FnOnce() -> T) -> T {
    let _guard = lock_env();
    let previous = env::var_os(SOURCE_ENV);
    // SAFETY: the module mutex serializes every environment mutation in this
    // test binary.
    unsafe {
        match value {
            Some(path) => env::set_var(SOURCE_ENV, path),
            None => env::remove_var(SOURCE_ENV),
        }
    }

    struct Restore(Option<OsString>);

    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: the module mutex is still held while the value is restored.
            unsafe {
                match self.0.take() {
                    Some(value) => env::set_var(SOURCE_ENV, value),
                    None => env::remove_var(SOURCE_ENV),
                }
            }
        }
    }

    let _restore = Restore(previous);
    action()
}

fn fixed_now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("the fixed time is valid")
}

fn options(tag: Option<&str>) -> BuildOptions {
    BuildOptions {
        tag: tag.map(reference),
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

fn version() -> FreedosVersion {
    FreedosVersion::parse("1.3").expect("the pinned version parses")
}

struct Fixture {
    root: tempfile::TempDir,
    context: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().expect("the data root is created"),
            context: tempfile::tempdir().expect("the context is created"),
        }
    }

    fn root_path(&self) -> &Path {
        self.root.path()
    }

    fn context_path(&self) -> &Path {
        self.context.path()
    }

    fn doskerfile(&self, text: &str) {
        fs::write(self.context_path().join("Doskerfile"), text).expect("the Doskerfile is written");
    }

    fn store(&self) -> OciStore {
        OciStore::new(self.root_path().to_path_buf())
    }

    fn source_path(&self) -> PathBuf {
        self.context_path().join("freedos.img")
    }

    fn agent_path(&self) -> PathBuf {
        self.context_path().join("DOSKER.EXE")
    }

    fn inputs(&self) -> BaseInputs {
        let source = self.source_path();
        write_freedos_source(&source);
        let agent = self.agent_path();
        write_agent(&agent);
        synthetic_inputs(&source, &agent)
    }

    fn builder(&self) -> Builder {
        Builder::with_resolver(self.root_path().to_path_buf(), Box::new(FakeResolver))
    }

    fn build(
        &self,
        builder: &Builder,
        options: &BuildOptions,
    ) -> (Result<BuildOutcome, BuildError>, TestReporter) {
        let mut reporter = TestReporter::default();
        let result = builder.build(self.context_path(), options, &mut reporter);
        (result, reporter)
    }

    fn lock_path(&self) -> PathBuf {
        self.context_path().join(LOCK_FILE_NAME)
    }

    fn lock(&self) -> LockFile {
        let text = fs::read_to_string(self.lock_path()).expect("the lock file reads");
        LockFile::from_toml(&text).expect("the lock file parses")
    }
}

fn commit_stale_base(store: &OciStore, inputs: &BaseInputs) -> BaseImage {
    let binaries = load_freedos_binaries(inputs).expect("the inputs load");
    let image = build_base_image(&binaries, &version()).expect("the base builds");
    let mut config = image.base_config();
    config.agent_version = Some("0.0.0".to_owned());

    let layer =
        Layer::from_delta(&image.tree.to_delta(), "freedos-base", None, None).expect("the layer");
    let descriptor = store.put_layer(&layer).expect("the layer stores");
    let raw = store.put_blob(&image.raw).expect("the raw disk stores");
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_RAW_DISK_DIGEST.to_owned(), raw.digest);
    store
        .commit_image(
            Some(&reference("freedos:1.3")),
            &config,
            vec![descriptor],
            annotations,
        )
        .expect("the stale base commits");
    image
}

#[test]
fn from_freedos_builds_and_stores_the_base() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM freedos:1.3\n");
    let inputs = fixture.inputs();
    let builder = fixture.builder().with_freedos_inputs(inputs);

    let (result, _) = fixture.build(&builder, &options(Some("built:latest")));
    let outcome = result.expect("the build succeeds");

    let store = fixture.store();
    let base = store.load(&tag("freedos:1.3")).expect("the base is stored");
    let config = store
        .load_config(&tag("freedos:1.3"))
        .expect("the base config loads");
    assert_eq!(config.dos_flavour, DosFlavour::Freedos);
    assert_eq!(config.agent_version.as_deref(), Some("1.0.0"));
    assert_eq!(
        outcome.manifest_digest, base.manifest_digest,
        "the build outcome names the base manifest"
    );
}

#[test]
fn from_freedos_writes_the_base_and_agent_pins() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM freedos:1.3\n");
    let inputs = fixture.inputs();
    let builder = fixture.builder().with_freedos_inputs(inputs);

    let (result, _) = fixture.build(&builder, &options(Some("built:latest")));
    result.expect("the build succeeds");

    let text = fs::read_to_string(fixture.lock_path()).expect("the lock file reads");
    let lock = fixture.lock();
    assert_eq!(lock.base.len(), 1, "one base pin is written");
    assert_eq!(lock.base[0].name, "freedos");
    assert_eq!(lock.base[0].tag, "1.3");
    assert!(
        lock.base[0].digest.starts_with("sha256:"),
        "the base pin is a digest: {}",
        lock.base[0].digest
    );
    assert!(
        text.contains("name = \"freedos\""),
        "the lock text names freedos: {text}"
    );
    assert!(
        text.contains("tag = \"1.3\""),
        "the lock text names the tag: {text}"
    );
    assert_eq!(
        lock.agent.as_ref().map(|agent| agent.version.as_str()),
        Some("1.0.0")
    );
}

#[test]
fn from_freedos_reuses_a_stored_base_without_source() {
    with_source_env(None, || {
        let fixture = Fixture::new();
        fixture.doskerfile("FROM freedos:1.3\n");
        let inputs = fixture.inputs();

        let (first, _) = fixture.build(
            &fixture.builder().with_freedos_inputs(inputs),
            &options(Some("built:latest")),
        );
        let first = first.expect("the first build succeeds");
        let base_digest = fixture
            .store()
            .load(&tag("freedos:1.3"))
            .expect("the base loads")
            .manifest_digest;

        let (second, _) = fixture.build(&fixture.builder(), &options(Some("built:latest")));
        let second = second.expect("the stored base is reused without the source");

        assert_eq!(
            second.manifest_digest, first.manifest_digest,
            "the reused base produces the same build"
        );
        assert_eq!(
            fixture
                .store()
                .load(&tag("freedos:1.3"))
                .expect("the base loads")
                .manifest_digest,
            base_digest,
            "the stored base is not rebuilt"
        );
    });
}

#[test]
fn from_freedos_rejects_an_unknown_version() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM freedos:1.2\n");

    let (result, _) = fixture.build(&fixture.builder(), &options(Some("built:latest")));
    match result {
        Err(error) => assert!(
            error.to_string().contains("supported: 1.3"),
            "the error names the supported version: {error}"
        ),
        Ok(outcome) => panic!("expected an unsupported version, got {outcome:?}"),
    }
}

#[test]
fn from_freedos_requires_the_source_when_no_base_exists() {
    with_source_env(None, || {
        let fixture = Fixture::new();
        fixture.doskerfile("FROM freedos:1.3\n");

        let (result, _) = fixture.build(&fixture.builder(), &options(Some("built:latest")));
        match result {
            Err(BuildError::Image(ImageError::FreedosSourceNotSet)) => {}
            other => panic!("expected FreedosSourceNotSet, got {other:?}"),
        }
        assert!(
            !fixture.lock_path().exists(),
            "a failed build writes no lock file"
        );

        let missing = fixture.context_path().join("missing.img");
        let agent = fixture.agent_path();
        write_agent(&agent);
        let inputs = BaseInputs {
            source: missing.clone(),
            agent_executable: agent,
            pins: FreedosPins::default(),
        };

        let (result, _) = fixture.build(
            &fixture.builder().with_freedos_inputs(inputs),
            &options(Some("built:latest")),
        );
        match result {
            Err(BuildError::Image(ImageError::FreedosSourceMissing { path })) => {
                assert_eq!(path, missing, "the error names the missing source")
            }
            other => panic!("expected FreedosSourceMissing, got {other:?}"),
        }
        assert!(
            !fixture.lock_path().exists(),
            "a failed build writes no lock file"
        );
    });
}

#[test]
fn from_freedos_requires_inputs_to_rebuild_a_stale_base() {
    with_source_env(None, || {
        let fixture = Fixture::new();
        fixture.doskerfile("FROM freedos:1.3\n");
        let inputs = fixture.inputs();
        commit_stale_base(&fixture.store(), &inputs);

        let (result, _) = fixture.build(&fixture.builder(), &options(Some("built:latest")));
        match result {
            Err(BuildError::Image(ImageError::FreedosSourceNotSet)) => {}
            other => panic!("expected FreedosSourceNotSet, got {other:?}"),
        }

        let (result, _) = fixture.build(
            &fixture.builder().with_freedos_inputs(inputs),
            &options(Some("built:latest")),
        );
        result.expect("the stale base is rebuilt with explicit inputs");

        let config = fixture
            .store()
            .load_config(&tag("freedos:1.3"))
            .expect("the rebuilt config loads");
        assert_eq!(config.agent_version.as_deref(), Some("1.0.0"));
    });
}

#[test]
fn from_freedos_refreshes_a_moved_pin_with_update() {
    let fixture = Fixture::new();
    fixture.doskerfile("FROM freedos:1.3\n");
    let inputs = fixture.inputs();

    let mut stale = LockFile::new();
    stale.base.push(BasePin {
        name: "freedos".to_owned(),
        tag: "1.3".to_owned(),
        digest: format!("sha256:{}", "0".repeat(64)),
    });
    fs::write(
        fixture.lock_path(),
        stale.to_toml().expect("the stale lock serializes"),
    )
    .expect("the stale lock is written");

    let (result, _) = fixture.build(
        &fixture.builder().with_freedos_inputs(inputs.clone()),
        &options(Some("built:latest")),
    );
    match result {
        Err(BuildError::LockMismatch { .. }) => {}
        other => panic!("expected LockMismatch, got {other:?}"),
    }

    let mut refresh = options(Some("built:latest"));
    refresh.update_lock = true;
    let (result, _) = fixture.build(&fixture.builder().with_freedos_inputs(inputs), &refresh);
    result.expect("the updated lock refreshes the moved pin");

    let actual = fixture
        .store()
        .load(&tag("freedos:1.3"))
        .expect("the base loads")
        .manifest_digest;
    assert_eq!(
        fixture.lock().base[0].digest,
        actual,
        "the refreshed lock records the actual base digest"
    );
}
