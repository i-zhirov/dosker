use std::collections::BTreeMap;

use dosker_core::{AgentPin, BasePin, EnginePin, LOCK_VERSION, LockFile, LockFileError};

fn sample_lock() -> LockFile {
    LockFile {
        version: LOCK_VERSION,
        base: vec![BasePin {
            name: "freedos".to_owned(),
            tag: "1.3".to_owned(),
            digest: "sha256:0123456789abcdef".to_owned(),
        }],
        engines: BTreeMap::from([(
            "qemu".to_owned(),
            EnginePin {
                version: "11.1.3".to_owned(),
                source: "homebrew".to_owned(),
            },
        )]),
        agent: Some(AgentPin {
            version: "1.0.0".to_owned(),
        }),
    }
}

#[test]
fn lock_file_round_trips_through_toml() {
    let lock = sample_lock();
    let text = lock.to_toml().expect("the lock serializes");

    assert_eq!(lock.to_toml().expect("stable output"), text);
    assert!(text.contains("[[base]]"));
    assert!(text.contains("[engines.qemu]"));
    assert!(text.contains("[agent]"));
    assert_eq!(LockFile::from_toml(&text).expect("the lock parses"), lock);
}

#[test]
fn lock_file_reports_unsupported_version_with_unknown_fields() {
    let error = LockFile::from_toml("version = 2\nfuture_field = \"value\"\n")
        .expect_err("version 2 is rejected");

    assert!(matches!(error, LockFileError::UnsupportedVersion(2)));
}

#[test]
fn to_toml_rejects_an_unsupported_version() {
    let lock = LockFile {
        version: 2,
        ..LockFile::new()
    };
    let error = lock.to_toml().expect_err("version 2 is not written");

    assert!(matches!(error, LockFileError::Invalid { .. }));
    assert!(error.to_string().starts_with("invalid lock file: "));
}

#[test]
fn lock_file_rejects_malformed_toml() {
    let error = LockFile::from_toml("version = \n").expect_err("malformed TOML is rejected");

    assert!(matches!(error, LockFileError::Invalid { .. }));
}

#[test]
fn lock_file_rejects_a_missing_required_field() {
    let error = LockFile::from_toml("version = 1\n\n[[base]]\nname = \"freedos\"\ntag = \"1.3\"\n")
        .expect_err("a base pin without a digest is rejected");

    assert!(matches!(error, LockFileError::Invalid { .. }));
}

#[test]
fn lock_file_rejects_an_unknown_version() {
    let error = LockFile::from_toml("version = 2\n").expect_err("version 2 is rejected");

    assert!(matches!(error, LockFileError::UnsupportedVersion(2)));
    assert_eq!(error.to_string(), "unsupported lock file version: 2");
}

#[test]
fn lock_file_allows_missing_base_engine_and_agent_sections() {
    let lock = LockFile::from_toml("version = 1\n").expect("a version-only lock parses");

    assert_eq!(lock, LockFile::new());
    assert!(lock.base.is_empty());
    assert!(lock.engines.is_empty());
    assert_eq!(lock.agent, None);
}

#[test]
fn lock_file_rejects_an_unknown_field() {
    let error = LockFile::from_toml("version = 1\n\n[agent]\nverison = \"1.0.0\"\n")
        .expect_err("a typo is rejected");

    assert!(matches!(error, LockFileError::Invalid { .. }));
    assert!(error.to_string().starts_with("invalid lock file: "));
}

#[test]
fn empty_sections_are_omitted_from_toml() {
    assert_eq!(
        LockFile::new().to_toml().expect("the lock serializes"),
        "version = 1\n"
    );
}

#[test]
fn base_pin_keeps_an_empty_tag() {
    let lock = LockFile {
        base: vec![BasePin {
            name: "base".to_owned(),
            tag: String::new(),
            digest: "sha256:0123456789abcdef".to_owned(),
        }],
        ..LockFile::new()
    };
    let text = lock.to_toml().expect("the lock serializes");

    assert!(text.contains("tag = \"\""));
    assert_eq!(LockFile::from_toml(&text).expect("the lock parses"), lock);
}

#[test]
fn engine_pin_keeps_version_and_source() {
    let lock = sample_lock();
    let text = lock.to_toml().expect("the lock serializes");
    let parsed = LockFile::from_toml(&text).expect("the lock parses");

    let engine = parsed.engines.get("qemu").expect("the qemu pin");
    assert_eq!(engine.version, "11.1.3");
    assert_eq!(engine.source, "homebrew");
}
