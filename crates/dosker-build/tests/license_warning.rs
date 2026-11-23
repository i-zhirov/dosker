use std::fs;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use dosker_build::{BuildError, BuildOptions, BuildOutcome, Builder, LICENSE_MISSING_WARNING};
use dosker_image::{ImageRef, OciStore};

use support::{
    FakeResolver, TestReporter, empty_tree, sample_license, store_licensed_tree_base,
    store_tree_base,
};

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
        no_pull: false,
    }
}

fn built_config(fixture: &Fixture) -> dosker_image::ImageConfig {
    fixture
        .store()
        .load_config(&ImageRef::parse("built:latest").expect("the reference parses"))
        .expect("the built config loads")
}

fn warns(reporter: &TestReporter) -> bool {
    reporter
        .lines
        .iter()
        .any(|line| line.as_str() == LICENSE_MISSING_WARNING)
}

#[test]
fn a_build_without_entries_succeeds_and_warns() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        Some("1.2.3"),
    );
    fixture.doskerfile("FROM base:latest\nENV A=1\n");

    let (result, reporter) = fixture.build();
    result.expect("the build succeeds");

    assert!(warns(&reporter));
    assert!(built_config(&fixture).licenses.is_empty());
}

#[test]
fn a_build_with_an_entry_does_not_warn() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        Some("1.2.3"),
    );
    fixture.doskerfile("FROM base:latest\nLICENSE NAME=APP SPDX=MIT STATUS=redistributable\n");

    let (result, reporter) = fixture.build();
    result.expect("the build succeeds");

    assert!(!warns(&reporter));
}

#[test]
fn a_build_from_a_licensed_base_does_not_warn() {
    let fixture = Fixture::new();
    store_licensed_tree_base(
        fixture.root_path(),
        "base:latest",
        vec![sample_license("BASE")],
    );
    fixture.doskerfile("FROM base:latest\nENV A=1\n");

    let (result, reporter) = fixture.build();
    result.expect("the build succeeds");

    assert!(!warns(&reporter));
}

#[test]
fn a_build_stores_the_declared_entry_in_the_config() {
    let fixture = Fixture::new();
    store_tree_base(
        fixture.root_path(),
        "base:latest",
        &empty_tree(),
        None,
        Some("1.2.3"),
    );
    fixture.doskerfile("FROM base:latest\nLICENSE NAME=APP SPDX=MIT STATUS=redistributable\n");

    let (result, _) = fixture.build();
    result.expect("the build succeeds");

    let config = built_config(&fixture);
    assert_eq!(config.licenses.len(), 1);
    assert_eq!(config.licenses[0].name, "APP");
    config.validate().expect("the config validates");
}
