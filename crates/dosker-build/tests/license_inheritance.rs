use std::fs;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use dosker_build::{BuildError, BuildOptions, BuildOutcome, Builder};
use dosker_image::{ImageRef, OciStore};

use support::{FakeResolver, TestReporter, sample_license, store_licensed_tree_base};

mod support;

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
        fixture.write("agent/DOSKER.EXE", b"MZ fake agent");
        fixture
    }

    fn root_path(&self) -> &Path {
        self.root.path()
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

    fn build(&self) -> (Result<BuildOutcome, BuildError>, TestReporter) {
        let mut reporter = TestReporter::default();
        let result = Builder::with_resolver(self.root.path().to_path_buf(), Box::new(FakeResolver))
            .with_agent_executable(self.context.path().join("agent/DOSKER.EXE"))
            .build(self.context.path(), &options(), &mut reporter);
        (result, reporter)
    }
}

fn fixed_now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("the fixed time is valid")
}

fn options() -> BuildOptions {
    BuildOptions {
        tag: Some(dosker_image::ImageReference::parse("built:latest").expect("the tag parses")),
        update_lock: false,
        no_cache: false,
        timeout: Duration::from_secs(30),
        now: fixed_now(),
    }
}

fn built_config(fixture: &Fixture) -> dosker_image::ImageConfig {
    fixture
        .store()
        .load_config(&ImageRef::parse("built:latest").expect("the reference parses"))
        .expect("the built config loads")
}

#[test]
fn a_child_appends_its_entries_to_the_base_entries() {
    let fixture = Fixture::new();
    store_licensed_tree_base(
        fixture.root_path(),
        "base:latest",
        vec![sample_license("BASE")],
    );
    fixture.doskerfile("FROM base:latest\nLICENSE NAME=APP SPDX=MIT STATUS=redistributable\n");

    let (result, _) = fixture.build();
    result.expect("the build succeeds");

    let config = built_config(&fixture);
    let names: Vec<&str> = config
        .licenses
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, vec!["BASE", "APP"]);
}

#[test]
fn an_identical_base_entry_is_deduplicated() {
    let fixture = Fixture::new();
    store_licensed_tree_base(
        fixture.root_path(),
        "base:latest",
        vec![sample_license("APP")],
    );
    fixture.doskerfile(
        "FROM base:latest\nLICENSE NAME=APP SPDX=MIT STATUS=redistributable BASIS=open-source-license\n",
    );

    let (result, _) = fixture.build();
    result.expect("the build succeeds");

    let config = built_config(&fixture);
    assert_eq!(config.licenses.len(), 1);
    assert_eq!(config.licenses[0].name, "APP");
}

#[test]
fn a_conflicting_base_entry_fails_the_build() {
    let fixture = Fixture::new();
    store_licensed_tree_base(
        fixture.root_path(),
        "base:latest",
        vec![sample_license("APP")],
    );
    fixture.doskerfile(
        "FROM base:latest\nLICENSE NAME=APP SPDX=Apache-2.0 STATUS=redistributable BASIS=open-source-license\n",
    );

    let (result, _) = fixture.build();
    let error = result.expect_err("the conflicting LICENSE fails");

    match error {
        BuildError::Instruction { instruction, .. } => assert_eq!(instruction, "LICENSE"),
        other => panic!("expected a LICENSE instruction error, got {other}"),
    }
    assert!(
        fixture
            .store()
            .load_config(&ImageRef::parse("built:latest").expect("the reference parses"))
            .is_err(),
        "no tag is stored"
    );
}
