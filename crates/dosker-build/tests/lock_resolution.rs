use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use dosker_build::{
    BuildError, ResolvedEngine, check_engine_pin, record_engine_pin, resolve_lock, write_lock,
};
use dosker_core::{AgentPin, BasePin, EnginePin, LOCK_FILE_NAME, LOCK_VERSION, LockFile};

fn base(name: &str, tag: &str, digest: &str) -> BasePin {
    BasePin {
        name: name.to_owned(),
        tag: tag.to_owned(),
        digest: digest.to_owned(),
    }
}

fn engine(name: &str, version: &str, source: &str) -> ResolvedEngine {
    ResolvedEngine {
        name: name.to_owned(),
        version: version.to_owned(),
        source: source.to_owned(),
    }
}

fn existing_lock(base: BasePin, agent: Option<&str>) -> LockFile {
    LockFile {
        version: LOCK_VERSION,
        base: vec![base],
        engines: BTreeMap::from([(
            "qemu".to_owned(),
            EnginePin {
                version: "8.2.0".to_owned(),
                source: "homebrew".to_owned(),
            },
        )]),
        agent: agent.map(|version| AgentPin {
            version: version.to_owned(),
        }),
    }
}

fn write_existing(context: &Path, lock: &LockFile) {
    fs::write(
        context.join(LOCK_FILE_NAME),
        lock.to_toml().expect("serialize the lock"),
    )
    .expect("write the lock");
}

fn read_lock(context: &Path) -> LockFile {
    let text = fs::read_to_string(context.join(LOCK_FILE_NAME)).expect("read the lock");
    LockFile::from_toml(&text).expect("parse the lock")
}

#[test]
fn a_hand_built_namespaced_pin_round_trips() {
    let context = tempfile::tempdir().unwrap();
    let digest = format!("sha256:{}", "a".repeat(64));
    let pin = base("owner/name", "tag", &digest);
    let lock = existing_lock(base("owner/name", "tag", &digest), None);
    write_existing(context.path(), &lock);

    let parsed = read_lock(context.path());

    assert_eq!(parsed.base, vec![pin]);
}

#[test]
fn record_engine_pin_adds_a_second_engine() {
    let mut lock = LockFile::new();

    record_engine_pin(&mut lock, &engine("qemu", "8.2.0", "homebrew"));
    record_engine_pin(&mut lock, &engine("dosbox-x", "0.83", "path"));

    assert_eq!(lock.engines.len(), 2);
    assert_eq!(
        lock.engines.get("qemu"),
        Some(&EnginePin {
            version: "8.2.0".to_owned(),
            source: "homebrew".to_owned(),
        })
    );
    assert_eq!(
        lock.engines.get("dosbox-x"),
        Some(&EnginePin {
            version: "0.83".to_owned(),
            source: "path".to_owned(),
        })
    );
}

#[test]
fn check_engine_pin_rejects_a_mismatched_version() {
    let existing = existing_lock(base("base", "latest", "sha256:base"), Some("1.0.0"));

    let error = check_engine_pin(&existing, &engine("qemu", "8.3.0", "homebrew"), false)
        .expect_err("a mismatched version must fail");

    let BuildError::LockMismatch { message } = error else {
        panic!("unexpected error: {error}");
    };
    assert!(message.contains("8.2.0"), "message: {message}");
    assert!(message.contains("8.3.0"), "message: {message}");

    check_engine_pin(&existing, &engine("qemu", "8.3.0", "homebrew"), true)
        .expect("update accepts the new version");
}

#[test]
fn resolve_lock_keeps_existing_engine_pins() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let mut existing = existing_lock(base("base", "latest", "sha256:base"), Some("1.0.0"));
    existing.engines.insert(
        "dosbox-x".to_owned(),
        EnginePin {
            version: "0.83".to_owned(),
            source: "path".to_owned(),
        },
    );
    write_existing(dir.path(), &existing);

    let lock = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:base"),
        Some("1.0.0"),
        &engine("dosbox-x", "0.83", "path"),
        false,
    )
    .expect("the final engine resolves");

    assert_eq!(
        lock.engines.get("qemu"),
        Some(&EnginePin {
            version: "8.2.0".to_owned(),
            source: "homebrew".to_owned(),
        }),
        "the non-final engine pin is retained"
    );
    assert_eq!(
        lock.engines.get("dosbox-x"),
        Some(&EnginePin {
            version: "0.83".to_owned(),
            source: "path".to_owned(),
        })
    );
}

#[test]
fn a_missing_lock_file_resolves_all_pins() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let base = base("freedos", "1.3", "sha256:aaaa");
    let engine = engine("qemu", "8.2.0", "homebrew");

    let lock = resolve_lock(dir.path(), base.clone(), Some("1.0.0"), &engine, false)
        .expect("resolve the missing lock");

    assert_eq!(lock.version, LOCK_VERSION);
    assert_eq!(lock.base, vec![base]);
    assert_eq!(
        lock.engines.get("qemu"),
        Some(&EnginePin {
            version: "8.2.0".to_owned(),
            source: "homebrew".to_owned(),
        })
    );
    assert_eq!(
        lock.agent,
        Some(AgentPin {
            version: "1.0.0".to_owned(),
        })
    );
}

#[test]
fn matching_pins_stay_unchanged() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let base = base("freedos", "1.3", "sha256:aaaa");
    let existing = existing_lock(base.clone(), Some("1.0.0"));
    write_existing(dir.path(), &existing);

    let lock = resolve_lock(
        dir.path(),
        base,
        Some("1.0.0"),
        &engine("qemu", "8.2.0", "homebrew"),
        false,
    )
    .expect("matching pins resolve");

    assert_eq!(lock, existing);
}

#[test]
fn base_tag_movement_fails_without_update() {
    let dir = tempfile::tempdir().expect("create temp directory");
    write_existing(
        dir.path(),
        &existing_lock(base("base", "latest", "sha256:old"), Some("1.0.0")),
    );

    let error = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:new"),
        Some("1.0.0"),
        &engine("qemu", "8.2.0", "homebrew"),
        false,
    )
    .expect_err("a moved tag must fail");

    let BuildError::LockMismatch { message } = error else {
        panic!("unexpected error: {error}");
    };
    assert!(message.contains("sha256:old"), "message: {message}");
    assert!(message.contains("sha256:new"), "message: {message}");
}

#[test]
fn base_tag_movement_updates_with_update() {
    let dir = tempfile::tempdir().expect("create temp directory");
    write_existing(
        dir.path(),
        &existing_lock(base("base", "latest", "sha256:old"), Some("1.0.0")),
    );
    let moved = base("base", "latest", "sha256:new");

    let lock = resolve_lock(
        dir.path(),
        moved.clone(),
        Some("1.0.0"),
        &engine("qemu", "8.2.0", "homebrew"),
        true,
    )
    .expect("update accepts the moved tag");

    assert_eq!(lock.base, vec![moved]);
}

#[test]
fn engine_version_mismatch_fails_without_update() {
    let dir = tempfile::tempdir().expect("create temp directory");
    write_existing(
        dir.path(),
        &existing_lock(base("base", "latest", "sha256:base"), Some("1.0.0")),
    );

    let error = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:base"),
        Some("1.0.0"),
        &engine("qemu", "8.3.0", "homebrew"),
        false,
    )
    .expect_err("an engine version mismatch must fail");

    let BuildError::LockMismatch { message } = error else {
        panic!("unexpected error: {error}");
    };
    assert!(message.contains("8.2.0"), "message: {message}");
    assert!(message.contains("8.3.0"), "message: {message}");
}

#[test]
fn engine_version_mismatch_updates_with_update() {
    let dir = tempfile::tempdir().expect("create temp directory");
    write_existing(
        dir.path(),
        &existing_lock(base("base", "latest", "sha256:base"), Some("1.0.0")),
    );

    let lock = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:base"),
        Some("1.0.0"),
        &engine("qemu", "8.3.0", "homebrew"),
        true,
    )
    .expect("update accepts the engine version");

    assert_eq!(
        lock.engines.get("qemu"),
        Some(&EnginePin {
            version: "8.3.0".to_owned(),
            source: "homebrew".to_owned(),
        })
    );
}

#[test]
fn agent_version_mismatch_fails_without_update() {
    let dir = tempfile::tempdir().expect("create temp directory");
    write_existing(
        dir.path(),
        &existing_lock(base("base", "latest", "sha256:base"), Some("1.0.0")),
    );

    let error = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:base"),
        Some("1.1.0"),
        &engine("qemu", "8.2.0", "homebrew"),
        false,
    )
    .expect_err("an agent version mismatch must fail");

    let BuildError::LockMismatch { message } = error else {
        panic!("unexpected error: {error}");
    };
    assert!(message.contains("1.0.0"), "message: {message}");
    assert!(message.contains("1.1.0"), "message: {message}");
}

#[test]
fn missing_agent_pin_is_written() {
    let dir = tempfile::tempdir().expect("create temp directory");
    write_existing(
        dir.path(),
        &existing_lock(base("base", "latest", "sha256:base"), None),
    );

    let lock = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:base"),
        Some("1.0.0"),
        &engine("qemu", "8.2.0", "homebrew"),
        false,
    )
    .expect("a missing agent pin resolves");

    assert_eq!(
        lock.agent,
        Some(AgentPin {
            version: "1.0.0".to_owned(),
        })
    );
}

#[test]
fn a_base_without_an_agent_version_omits_the_agent_pin() {
    let dir = tempfile::tempdir().expect("create temp directory");

    let lock = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:base"),
        None,
        &engine("qemu", "8.2.0", "homebrew"),
        false,
    )
    .expect("a base without an agent resolves");

    assert_eq!(lock.agent, None);
}

#[test]
fn a_base_without_an_agent_version_removes_a_stale_agent_pin() {
    let dir = tempfile::tempdir().expect("create temp directory");
    write_existing(
        dir.path(),
        &existing_lock(base("base", "latest", "sha256:base"), Some("1.0.0")),
    );

    let lock = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:base"),
        None,
        &engine("qemu", "8.2.0", "homebrew"),
        false,
    )
    .expect("a stale agent pin is removed");

    assert_eq!(lock.agent, None);
}

#[test]
fn digest_base_ignores_a_tag_pin_for_the_same_name() {
    let dir = tempfile::tempdir().expect("create temp directory");
    write_existing(
        dir.path(),
        &existing_lock(base("base", "latest", "sha256:tag"), Some("1.0.0")),
    );
    let digest = base("base", "", "sha256:digest");

    let lock = resolve_lock(
        dir.path(),
        digest.clone(),
        Some("1.0.0"),
        &engine("qemu", "8.2.0", "homebrew"),
        false,
    )
    .expect("a digest base does not compare with a tag pin");

    assert_eq!(lock.base, vec![digest]);
}

#[test]
fn the_lock_file_ends_with_a_newline() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let lock = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:base"),
        Some("1.0.0"),
        &engine("qemu", "8.2.0", "homebrew"),
        false,
    )
    .expect("resolve the lock");

    write_lock(dir.path(), &lock).expect("write the lock");

    let bytes = fs::read(dir.path().join(LOCK_FILE_NAME)).expect("read the lock bytes");
    assert!(bytes.ends_with(b"\n"), "the lock must end with a newline");
}

#[test]
fn the_lock_file_round_trips_through_disk() {
    let dir = tempfile::tempdir().expect("create temp directory");
    let lock = resolve_lock(
        dir.path(),
        base("base", "latest", "sha256:base"),
        Some("1.0.0"),
        &engine("qemu", "8.2.0", "homebrew"),
        false,
    )
    .expect("resolve the lock");

    write_lock(dir.path(), &lock).expect("write the lock");

    assert_eq!(read_lock(dir.path()), lock);
}
