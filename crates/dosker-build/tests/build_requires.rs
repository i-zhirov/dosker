mod support;

use std::time::Duration;

use chrono::{TimeZone, Utc};
use dosker_build::{BuildError, BuildOptions};
use dosker_image::ImageReference;
use support::{ScriptedRunner, runner_fixture};

fn options(tag: Option<&str>) -> BuildOptions {
    BuildOptions {
        tag: tag.map(|value| ImageReference::parse(value).expect("the reference parses")),
        update_lock: false,
        no_cache: false,
        timeout: Duration::from_secs(30),
        now: Utc
            .timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("the fixed time is valid"),
    }
}

#[test]
fn build_rejects_a_missing_required_capability() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    fixture.doskerfile("FROM base:latest\nENGINE qemu\nREQUIRES audio\n");

    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::Capability { message }) => assert_eq!(
            message,
            "engine qemu does not provide the audio capability; no engine provides it"
        ),
        other => panic!("expected a capability error, got {other:?}"),
    }
    assert_eq!(runner.call_count(), 0, "no step runs");
}

#[test]
fn build_accepts_a_required_capability() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    fixture.doskerfile("FROM base:latest\nREQUIRES gdbstub\n");

    fixture
        .build(&options(Some("built:latest")), &runner)
        .0
        .expect("the build succeeds");
}

#[test]
fn build_checks_requires_before_a_run_step() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    fixture.doskerfile("FROM base:latest\nENGINE qemu\nRUN ECHO HI\nREQUIRES audio\n");

    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::Capability { message }) => assert!(
            message.contains("audio"),
            "the error names the missing capability: {message}"
        ),
        other => panic!("expected a capability error, got {other:?}"),
    }
    assert_eq!(runner.call_count(), 0, "the RUN step never boots");
}
