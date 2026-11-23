mod support;

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use dosker_build::{BuildError, BuildOptions, InDosRunOutcome, build_config_digest, write_lock};
use dosker_core::{AgentPin, BasePin, EngineName, EnginePin, LOCK_FILE_NAME, LockFile};
use dosker_image::cache::CACHE_SCHEMA_VERSION;
use dosker_image::{
    Attributes, CacheKeyInput, CacheRecord, CacheStore, DosPath, ImageError, ImageRef,
    ImageReference, Layer, Tree, TreeNode, cache_key, fixed_timestamp,
};
use support::{
    AGENT_DISK_PARTITION_LENGTH, AGENT_DISK_PARTITION_OFFSET, RunnerFixture, ScriptedRunner,
    VersionedResolver, flip_disk_byte, move_disk_partition, remove_disk_file, runner_fixture,
    store_agent_base_with_gap_byte, store_booter_base, store_tree_base, write_disk_file,
};

const RUN_INSTRUCTION: &str = "RUN ECHO HI";

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
        no_pull: false,
    }
}

fn reference(value: &str) -> ImageReference {
    ImageReference::parse(value).expect("the reference parses")
}

fn tag(value: &str) -> ImageRef {
    ImageRef::parse(value).expect("the reference parses")
}

fn dos(path: &str) -> DosPath {
    DosPath::parse(path).expect("the DOS path parses")
}

fn archive() -> Attributes {
    Attributes {
        archive: true,
        ..Attributes::default()
    }
}

fn ok() -> InDosRunOutcome {
    InDosRunOutcome {
        exit_code: 0,
        output: String::new(),
        truncated: false,
    }
}

fn built_tree(fixture: &RunnerFixture) -> Tree {
    fixture
        .store()
        .load_tree(&tag("built:latest"))
        .expect("the built tree loads")
}

fn run_layer(fixture: &RunnerFixture, from_end: usize) -> Layer {
    let manifest = fixture
        .store()
        .load_manifest(&tag("built:latest"))
        .expect("the built manifest loads");
    let index = manifest
        .layers
        .len()
        .checked_sub(1 + from_end)
        .expect("the layer exists");
    fixture
        .store()
        .load_layer(&manifest.layers[index].digest)
        .expect("the built layer loads")
}

fn lock(fixture: &RunnerFixture) -> LockFile {
    let text = fs::read_to_string(fixture.context_path().join(LOCK_FILE_NAME))
        .expect("the lock file reads");
    LockFile::from_toml(&text).expect("the lock file parses")
}

fn assert_no_image(fixture: &RunnerFixture) {
    assert!(
        matches!(
            fixture.store().load(&tag("built:latest")),
            Err(ImageError::NotFound(_))
        ),
        "the failed build commits no image"
    );
}

fn assert_no_temp_disk(fixture: &RunnerFixture) {
    let tmp = fixture.root_path().join("tmp");
    if tmp.exists() {
        let mut entries = fs::read_dir(&tmp).expect("the temp directory reads");
        assert!(entries.next().is_none(), "the temporary disk is removed");
    }
}

fn run_cache_key(tree: &str, config: &str, engine: &str, disk: &str) -> String {
    let input_hashes = vec![
        format!("staged:{tree}"),
        format!("engine:{engine}"),
        "agent:1.0.0".to_owned(),
        format!("disk:{disk}"),
    ];
    cache_key(&CacheKeyInput {
        parent_tree_digest: tree,
        config_digest: config,
        instruction: RUN_INSTRUCTION,
        input_hashes: &input_hashes,
    })
}

#[test]
fn run_in_dos_writes_a_file_into_the_layer() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|request| {
        write_disk_file(request.disk_path, r"C:\MADE.TXT", b"made");
        Ok(ok())
    });
    fixture.doskerfile("FROM base:latest\nRUN COPY HELLO.TXT MADE.TXT\n");

    let (result, _) = fixture.build(&options(Some("built:latest")), &runner);
    result.expect("the build succeeds");
    assert_eq!(runner.call_count(), 1);

    match built_tree(&fixture).get(&dos(r"C:\MADE.TXT")) {
        Some(TreeNode::File(file)) => {
            assert_eq!(file.bytes, b"made");
            assert_eq!(file.attributes, archive());
            assert_eq!(file.timestamp, fixed_timestamp());
        }
        other => panic!("expected MADE.TXT, got {other:?}"),
    }
}

#[test]
fn run_in_dos_replaces_a_file() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|request| {
        write_disk_file(request.disk_path, r"C:\HELLO.TXT", b"changed");
        Ok(ok())
    });
    fixture.doskerfile("FROM base:latest\nRUN ECHO changed> HELLO.TXT\n");

    fixture
        .build(&options(Some("built:latest")), &runner)
        .0
        .expect("the build succeeds");

    match built_tree(&fixture).get(&dos(r"C:\HELLO.TXT")) {
        Some(TreeNode::File(file)) => assert_eq!(file.bytes, b"changed"),
        other => panic!("expected HELLO.TXT, got {other:?}"),
    }
}

#[test]
fn run_in_dos_deletes_a_file() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|request| {
        remove_disk_file(request.disk_path, r"C:\OLD.TXT");
        Ok(ok())
    });
    fixture.doskerfile("FROM base:latest\nRUN DEL OLD.TXT\n");

    fixture
        .build(&options(Some("built:latest")), &runner)
        .0
        .expect("the build succeeds");

    let tree = built_tree(&fixture);
    assert!(
        tree.get(&dos(r"C:\OLD.TXT")).is_none(),
        "the deleted file leaves the tree"
    );
    assert!(
        tree.get(&dos(r"C:\OLD\INNER.TXT")).is_some(),
        "the sibling file stays"
    );
}

#[test]
fn run_in_dos_ignores_agent_files() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|request| {
        write_disk_file(request.disk_path, r"C:\DOSKER\AGENT.TMP", b"temporary");
        Ok(ok())
    });
    fixture.doskerfile("FROM base:latest\nRUN ECHO temporary> DOSKER\\AGENT.TMP\n");

    fixture
        .build(&options(Some("built:latest")), &runner)
        .0
        .expect("the build succeeds");

    let tree = built_tree(&fixture);
    assert!(
        tree.get(&dos(r"C:\DOSKER\AGENT.TMP")).is_none(),
        "agent temporary files never enter a layer"
    );
    match tree.get(&dos(r"C:\DOSKER\DOSKER.EXE")) {
        Some(TreeNode::File(file)) => assert_eq!(file.bytes, b"MZ synthetic agent"),
        other => panic!("expected the untouched agent, got {other:?}"),
    }
}

#[test]
fn run_in_dos_writes_config_ini_before_boot() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|_| Ok(ok()));
    fixture
        .doskerfile("FROM base:latest\nMACHINE serial=3 control_port=3 baud=9600\nRUN ECHO HI\n");

    fixture
        .build(&options(Some("built:latest")), &runner)
        .0
        .expect("the build succeeds");

    let calls = runner.calls();
    assert_eq!(calls.len(), 1, "one VM boots");
    assert_eq!(
        calls[0].config_ini.as_deref(),
        Some("CONTROL_PORT=3\r\nBAUD=9600\r\n")
    );
    assert_eq!(calls[0].serial_ports, 3);
    assert_eq!(calls[0].control_port, 3);
    assert_eq!(calls[0].disk_interface, "ide");
}

#[test]
fn run_in_dos_reports_the_command_output() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|_| {
        Ok(InDosRunOutcome {
            exit_code: 0,
            output: "hello from dos".to_owned(),
            truncated: false,
        })
    });
    fixture.doskerfile("FROM base:latest\nRUN ECHO hello from dos\n");

    let (result, reporter) = fixture.build(&options(Some("built:latest")), &runner);
    result.expect("the build succeeds");

    assert!(
        reporter.output_text().contains("hello from dos"),
        "the builder reports the captured output"
    );
}

#[test]
fn run_in_dos_reports_empty_output() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    fixture.doskerfile("FROM base:latest\nRUN CLS\n");

    let (result, reporter) = fixture.build(&options(Some("built:latest")), &runner);
    result.expect("the build succeeds");

    assert_eq!(reporter.output_text(), "", "empty output writes no bytes");
    assert!(
        reporter
            .lines
            .iter()
            .any(|line| line == "RUN produced no captured output"),
        "the build log explains the empty output: {:?}",
        reporter.lines
    );
}

#[test]
fn run_in_dos_uses_the_command_and_boot_timeouts() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    fixture.doskerfile("FROM base:latest\nRUN ECHO HI\n");

    let mut options = options(Some("built:latest"));
    options.timeout = Duration::from_secs(42);
    fixture
        .build(&options, &runner)
        .0
        .expect("the build succeeds");

    let calls = runner.calls();
    assert_eq!(calls[0].timeout, Duration::from_secs(42));
    assert_eq!(calls[0].boot_timeout, dosker_build::RUN_BOOT_TIMEOUT);
}

#[test]
fn run_in_dos_records_a_boot_override() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    let label_offset = AGENT_DISK_PARTITION_OFFSET + 43;
    runner.push(move |request| {
        flip_disk_byte(request.disk_path, label_offset);
        Ok(ok())
    });
    fixture.doskerfile("FROM base:latest\nRUN FLIP\n");

    fixture
        .build(&options(Some("built:latest")), &runner)
        .0
        .expect("the build succeeds");

    let boot = run_layer(&fixture, 0)
        .meta
        .boot
        .expect("the changed boot artifacts become an override");
    assert!(
        !fixture.boot.boot_artifacts_equal(&boot),
        "the override differs from the base boot"
    );
}

#[test]
fn run_in_dos_keeps_boot_null_when_unchanged() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    fixture.doskerfile("FROM base:latest\nRUN ECHO HI\n");

    fixture
        .build(&options(Some("built:latest")), &runner)
        .0
        .expect("the build succeeds");

    assert!(
        run_layer(&fixture, 0).meta.boot.is_none(),
        "an unchanged boot writes no override"
    );
}

#[test]
fn run_in_dos_projects_the_previous_boot_override() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    let label_offset = AGENT_DISK_PARTITION_OFFSET + 43;
    runner.push(move |request| {
        flip_disk_byte(request.disk_path, label_offset);
        Ok(ok())
    });
    runner.push(|_| Ok(ok()));
    fixture.doskerfile("FROM base:latest\nRUN FLIP\nRUN ECHO AFTER\n");

    fixture
        .build(&options(Some("built:latest")), &runner)
        .0
        .expect("the build succeeds");
    assert_eq!(runner.call_count(), 2);

    let first = run_layer(&fixture, 1)
        .meta
        .boot
        .expect("the first RUN records the boot override");
    let second = runner.calls()[1].boot.clone();
    assert!(
        first.boot_artifacts_equal(&second),
        "the second RUN boots the overridden boot artifacts"
    );
    assert!(
        run_layer(&fixture, 0).meta.boot.is_none(),
        "the unchanged second boot writes no override"
    );
}

#[test]
fn run_in_dos_reapplies_a_partition_override() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    let new_offset = AGENT_DISK_PARTITION_OFFSET + 1024 * 1024;
    let new_length = AGENT_DISK_PARTITION_LENGTH;
    runner.push(move |request| {
        move_disk_partition(request.disk_path, new_offset, new_length);
        Ok(ok())
    });
    runner.push(|_| Ok(ok()));
    fixture.doskerfile("FROM base:latest\nRUN MOVE\nRUN ECHO AFTER\n");

    fixture
        .build(&options(Some("built:latest")), &runner)
        .0
        .expect("the build succeeds");
    assert_eq!(runner.call_count(), 2);

    let first = run_layer(&fixture, 1)
        .meta
        .boot
        .expect("the first RUN records the moved partition");
    assert_eq!(first.partition_start, Some((new_offset / 512) as u32));
    assert_eq!(first.partition_sectors, Some((new_length / 512) as u32));

    let second = &runner.calls()[1];
    assert_eq!(
        second.boot.partition_start,
        Some((new_offset / 512) as u32),
        "the second RUN projects the overridden partition"
    );
    assert!(
        run_layer(&fixture, 0).meta.boot.is_none(),
        "the second RUN records no further override"
    );
}

#[test]
fn run_in_dos_uses_the_engine_before_a_later_engine() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|request| {
        write_disk_file(request.disk_path, r"C:\MADE.TXT", b"made");
        Ok(ok())
    });
    fixture.doskerfile("FROM base:latest\nRUN COPY HELLO.TXT MADE.TXT\nENGINE dosbox-x\n");
    let resolver = VersionedResolver {
        versions: BTreeMap::from([
            ("qemu".to_owned(), "qemu 9.0.0".to_owned()),
            ("dosbox-x".to_owned(), "0.83".to_owned()),
        ]),
    };

    fixture
        .build_with(&options(Some("built:latest")), Box::new(resolver), &runner)
        .0
        .expect("the build succeeds");

    assert_eq!(
        runner.calls()[0].engine,
        EngineName::Qemu,
        "the RUN step uses the engine effective at that step"
    );
    let lock = lock(&fixture);
    assert!(lock.engines.contains_key("qemu"));
    assert!(lock.engines.contains_key("dosbox-x"));
}

#[test]
fn run_in_dos_cache_key_uses_the_engine_at_the_run_step() {
    let fixture = runner_fixture();
    fixture.doskerfile("FROM base:latest\nRUN ECHO HI\nENGINE dosbox-x\n");

    let first = ScriptedRunner::new();
    let resolver_a = VersionedResolver {
        versions: BTreeMap::from([
            ("qemu".to_owned(), "qemu 9.0.0".to_owned()),
            ("dosbox-x".to_owned(), "0.83".to_owned()),
        ]),
    };
    fixture
        .build_with(&options(Some("built:latest")), Box::new(resolver_a), &first)
        .0
        .expect("the first build succeeds");
    assert_eq!(first.call_count(), 1);

    let second = ScriptedRunner::new();
    let resolver_b = VersionedResolver {
        versions: BTreeMap::from([
            ("qemu".to_owned(), "qemu 9.1.0".to_owned()),
            ("dosbox-x".to_owned(), "0.83".to_owned()),
        ]),
    };
    let mut rebuild = options(Some("built:latest"));
    rebuild.update_lock = true;
    let outcome = fixture
        .build_with(&rebuild, Box::new(resolver_b), &second)
        .0
        .expect("the second build succeeds");

    assert_eq!(
        second.call_count(),
        1,
        "the RUN step misses when its own engine version changed"
    );
    assert_eq!(outcome.cached_steps, 1, "the ENGINE step is still cached");
}

#[test]
fn run_in_dos_cache_key_includes_the_raw_disk() {
    let fixture = runner_fixture();
    store_agent_base_with_gap_byte(fixture.root_path(), "other:latest", 0x7f);

    let store = fixture.store();
    let a_tree = store
        .load_tree(&tag("base:latest"))
        .expect("the first tree loads")
        .digest()
        .expect("the first tree digests");
    let b_tree = store
        .load_tree(&tag("other:latest"))
        .expect("the second tree loads")
        .digest()
        .expect("the second tree digests");
    let a_config = build_config_digest(
        &store
            .load_config(&tag("base:latest"))
            .expect("the first config loads"),
    )
    .expect("the first config digests");
    let b_config = build_config_digest(
        &store
            .load_config(&tag("other:latest"))
            .expect("the second config loads"),
    )
    .expect("the second config digests");
    let a_boot = store
        .load_effective_boot(&tag("base:latest"))
        .expect("the first boot loads");
    let b_boot = store
        .load_effective_boot(&tag("other:latest"))
        .expect("the second boot loads");
    let a_disk = store
        .load_manifest(&tag("base:latest"))
        .expect("the first manifest loads")
        .raw_disk_digest()
        .expect("the first manifest has a raw disk")
        .to_owned();
    let b_disk = store
        .load_manifest(&tag("other:latest"))
        .expect("the second manifest loads")
        .raw_disk_digest()
        .expect("the second manifest has a raw disk")
        .to_owned();

    assert_eq!(a_tree, b_tree, "the trees are identical");
    assert_eq!(a_config, b_config, "the configs are identical");
    assert!(
        a_boot.boot_artifacts_equal(&b_boot),
        "the boot metadata is identical"
    );
    assert_ne!(a_disk, b_disk, "the raw blobs differ");
    assert_ne!(
        run_cache_key(&a_tree, &a_config, "qemu 9.0.0", &a_disk),
        run_cache_key(&b_tree, &b_config, "qemu 9.0.0", &b_disk),
        "the RUN cache key depends on the raw disk"
    );

    fixture.doskerfile("FROM base:latest\nRUN ECHO HI\n");
    let first = ScriptedRunner::new();
    fixture
        .build(&options(Some("built:latest")), &first)
        .0
        .expect("the first build succeeds");
    assert_eq!(first.call_count(), 1);

    fixture.doskerfile("FROM other:latest\nRUN ECHO HI\n");
    let second = ScriptedRunner::new();
    let mut rebuild = options(Some("built:latest"));
    rebuild.update_lock = true;
    fixture
        .build(&rebuild, &second)
        .0
        .expect("the second build succeeds");
    assert_eq!(
        second.call_count(),
        1,
        "the identical tree with a different raw disk misses the RUN cache"
    );
}

#[test]
fn run_in_dos_requires_a_raw_disk() {
    let fixture = runner_fixture();
    store_tree_base(
        fixture.root_path(),
        "tree:latest",
        &support::empty_tree(),
        None,
        Some("1.0.0"),
    );
    fixture.doskerfile("FROM tree:latest\nRUN ECHO HI\n");
    let runner = ScriptedRunner::new();

    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::RunInDos { line, reason }) => {
            assert_eq!(line, 2);
            assert!(
                reason.contains("no raw disk"),
                "the reason names the missing raw disk: {reason}"
            );
        }
        other => panic!("expected a RUN error, got {other:?}"),
    }
    assert_eq!(runner.call_count(), 0, "no VM boots");
}

#[test]
fn run_in_dos_requires_a_raw_disk_before_the_cache() {
    let fixture = runner_fixture();
    store_tree_base(
        fixture.root_path(),
        "tree:latest",
        &support::empty_tree(),
        None,
        Some("1.0.0"),
    );
    fixture.doskerfile("FROM tree:latest\nRUN ECHO HI\n");

    let store = fixture.store();
    let tree = store
        .load_tree(&tag("tree:latest"))
        .expect("the tree loads")
        .digest()
        .expect("the tree digests");
    let config = build_config_digest(
        &store
            .load_config(&tag("tree:latest"))
            .expect("the config loads"),
    )
    .expect("the config digests");
    let input_hashes = vec![
        format!("staged:{tree}"),
        "engine:qemu 9.0.0".to_owned(),
        "agent:1.0.0".to_owned(),
    ];
    let old_key = cache_key(&CacheKeyInput {
        parent_tree_digest: &tree,
        config_digest: &config,
        instruction: RUN_INSTRUCTION,
        input_hashes: &input_hashes,
    });
    let base_layer = store
        .load_manifest(&tag("tree:latest"))
        .expect("the manifest loads")
        .layers[0]
        .digest
        .clone();
    CacheStore::new(fixture.root_path().to_path_buf())
        .store(&CacheRecord {
            schema_version: CACHE_SCHEMA_VERSION,
            key: old_key,
            layer_digest: base_layer,
            instruction: RUN_INSTRUCTION.to_owned(),
            created_at: fixed_timestamp(),
        })
        .expect("the stale cache record writes");

    let runner = ScriptedRunner::new();
    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::RunInDos { reason, .. }) => assert!(
            reason.contains("no raw disk"),
            "the reason names the missing raw disk: {reason}"
        ),
        other => panic!("expected a RUN error, got {other:?}"),
    }
    assert_eq!(
        runner.call_count(),
        0,
        "the raw disk is checked before the cache lookup"
    );
}

#[test]
fn run_in_dos_rejects_a_booter_image() {
    let fixture = runner_fixture();
    store_booter_base(fixture.root_path(), "boot:latest", b"BOOTER");
    fixture.doskerfile("FROM boot:latest\nRUN ECHO HI\n");
    let runner = ScriptedRunner::new();

    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::Instruction {
            line,
            instruction,
            reason,
        }) => {
            assert_eq!(line, 2);
            assert_eq!(instruction, "RUN");
            assert!(
                reason.contains("booter"),
                "the reason names the booter image: {reason}"
            );
        }
        other => panic!("expected a RUN instruction error, got {other:?}"),
    }
    assert_eq!(runner.call_count(), 0, "no VM boots");
}

#[test]
fn run_in_dos_requires_an_agent() {
    let fixture = runner_fixture();
    store_tree_base(
        fixture.root_path(),
        "tree:latest",
        &support::empty_tree(),
        Some(b"DISK"),
        None,
    );
    fixture.doskerfile("FROM tree:latest\nRUN ECHO HI\n");
    let runner = ScriptedRunner::new();

    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::RunBootFailed { line, reason }) => {
            assert_eq!(line, 2);
            assert!(
                reason.contains("no agent"),
                "the reason names the missing agent: {reason}"
            );
        }
        other => panic!("expected a RUN boot error, got {other:?}"),
    }
    assert_eq!(runner.call_count(), 0, "no VM boots");
}

#[test]
fn run_in_dos_rejects_a_non_qemu_engine() {
    let fixture = runner_fixture();
    fixture.doskerfile("FROM base:latest\nENGINE dosbox-x\nRUN ECHO HI\n");
    let runner = ScriptedRunner::new();

    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::Engine { engine, reason }) => {
            assert_eq!(engine, "dosbox-x");
            assert!(reason.contains("qemu"), "the reason names qemu: {reason}");
        }
        other => panic!("expected an engine error, got {other:?}"),
    }
    assert_eq!(runner.call_count(), 0, "no VM boots");
}

#[test]
fn run_in_dos_rejects_a_pinned_non_final_engine_mismatch() {
    let fixture = runner_fixture();
    fixture.doskerfile("FROM base:latest\nRUN COPY HELLO.TXT MADE.TXT\nENGINE dosbox-x\n");

    let mut existing = LockFile::new();
    existing.base.push(BasePin {
        name: "base".to_owned(),
        tag: "latest".to_owned(),
        digest: fixture.base_digest.clone(),
    });
    existing.engines.insert(
        "qemu".to_owned(),
        EnginePin {
            version: "qemu 9.0.0".to_owned(),
            source: "path".to_owned(),
        },
    );
    existing.engines.insert(
        "dosbox-x".to_owned(),
        EnginePin {
            version: "0.83".to_owned(),
            source: "path".to_owned(),
        },
    );
    existing.agent = Some(AgentPin {
        version: "1.0.0".to_owned(),
    });
    write_lock(fixture.context_path(), &existing).expect("the lock writes");

    let runner = ScriptedRunner::new();
    let resolver = VersionedResolver {
        versions: BTreeMap::from([
            ("qemu".to_owned(), "qemu 9.1.0".to_owned()),
            ("dosbox-x".to_owned(), "0.83".to_owned()),
        ]),
    };
    match fixture
        .build_with(&options(Some("built:latest")), Box::new(resolver), &runner)
        .0
    {
        Err(BuildError::LockMismatch { message }) => assert!(
            message.contains("qemu"),
            "the mismatch names the non-final engine: {message}"
        ),
        other => panic!("expected a lock mismatch, got {other:?}"),
    }
    assert_eq!(runner.call_count(), 0, "no VM boots");
}

#[test]
fn run_in_dos_cache_hit_boots_no_vm() {
    let fixture = runner_fixture();
    fixture.doskerfile("FROM base:latest\nRUN ECHO HI\n");

    let first = ScriptedRunner::new();
    fixture
        .build(&options(Some("built:latest")), &first)
        .0
        .expect("the first build succeeds");
    assert_eq!(first.call_count(), 1);

    let second = ScriptedRunner::new();
    let outcome = fixture
        .build(&options(Some("built:latest")), &second)
        .0
        .expect("the second build succeeds");

    assert_eq!(outcome.cached_steps, 1);
    assert_eq!(second.call_count(), 0, "a cached RUN boots no VM");
}

#[test]
fn run_in_dos_failure_writes_no_cache_record() {
    let fixture = runner_fixture();
    fixture.doskerfile("FROM base:latest\nRUN ECHO HI\n");

    let failing = ScriptedRunner::new();
    failing.push(|request| {
        Err(BuildError::RunExited {
            line: request.line,
            code: 1,
        })
    });
    assert!(
        fixture
            .build(&options(Some("built:latest")), &failing)
            .0
            .is_err()
    );
    assert_no_image(&fixture);
    assert_no_temp_disk(&fixture);

    let succeeding = ScriptedRunner::new();
    let outcome = fixture
        .build(&options(Some("built:latest")), &succeeding)
        .0
        .expect("the second build succeeds");
    assert_eq!(outcome.cached_steps, 0, "the failed RUN is not cached");
    assert_eq!(succeeding.call_count(), 1, "the RUN runs again");
}

#[test]
fn run_in_dos_removes_the_temporary_disk() {
    let fixture = runner_fixture();
    fixture.doskerfile("FROM base:latest\nRUN ECHO HI\n");

    fixture
        .build(&options(Some("built:latest")), &ScriptedRunner::new())
        .0
        .expect("the build succeeds");

    assert_no_temp_disk(&fixture);
}

#[test]
fn run_in_dos_reports_a_non_zero_exit() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|_| {
        Ok(InDosRunOutcome {
            exit_code: 3,
            output: "failure".to_owned(),
            truncated: false,
        })
    });
    fixture.doskerfile("FROM base:latest\nRUN FAIL\n");

    let (result, reporter) = fixture.build(&options(Some("built:latest")), &runner);
    match result {
        Err(BuildError::RunExited { line, code }) => {
            assert_eq!(line, 2);
            assert_eq!(code, 3);
        }
        other => panic!("expected a non-zero exit error, got {other:?}"),
    }
    assert!(
        reporter.output_text().contains("failure"),
        "the output is reported before the error"
    );
    assert_no_image(&fixture);
    assert_no_temp_disk(&fixture);
}

#[test]
fn run_in_dos_reports_a_guest_reboot() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|request| Err(BuildError::GuestRebooted { line: request.line }));
    fixture.doskerfile("FROM base:latest\nRUN REBOOT\n");

    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::GuestRebooted { line }) => assert_eq!(line, 2),
        other => panic!("expected a guest reboot error, got {other:?}"),
    }
    assert_no_image(&fixture);
    assert_no_temp_disk(&fixture);
}

#[test]
fn run_in_dos_reports_a_timeout() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|request| Err(BuildError::RunTimeout { line: request.line }));
    fixture.doskerfile("FROM base:latest\nRUN SLEEP\n");

    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::RunTimeout { line }) => assert_eq!(line, 2),
        other => panic!("expected a timeout error, got {other:?}"),
    }
    assert_no_image(&fixture);
    assert_no_temp_disk(&fixture);
}

#[test]
fn run_in_dos_reports_a_boot_failure() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|request| {
        Err(BuildError::RunBootFailed {
            line: request.line,
            reason: "the guest did not boot".to_owned(),
        })
    });
    fixture.doskerfile("FROM base:latest\nRUN ECHO HI\n");

    match fixture.build(&options(Some("built:latest")), &runner).0 {
        Err(BuildError::RunBootFailed { line, reason }) => {
            assert_eq!(line, 2);
            assert!(reason.contains("did not boot"));
        }
        other => panic!("expected a boot failure, got {other:?}"),
    }
    assert_no_image(&fixture);
    assert_no_temp_disk(&fixture);
}

#[test]
fn run_in_dos_reports_a_truncated_output() {
    let fixture = runner_fixture();
    let runner = ScriptedRunner::new();
    runner.push(|_| {
        Ok(InDosRunOutcome {
            exit_code: 0,
            output: "partial".to_owned(),
            truncated: true,
        })
    });
    fixture.doskerfile("FROM base:latest\nRUN DIR\n");

    let (result, reporter) = fixture.build(&options(Some("built:latest")), &runner);
    result.expect("the build succeeds");
    assert!(
        reporter
            .lines
            .iter()
            .any(|line| line == "RUN output was truncated"),
        "the build log reports truncation: {:?}",
        reporter.lines
    );
}
