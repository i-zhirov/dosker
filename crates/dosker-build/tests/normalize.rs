use std::collections::{BTreeMap, BTreeSet};

use dosker_build::{
    BuildError, build_config_digest, normalize_instruction, render_tokens, resolve_copy_destination,
};
use dosker_core::{Doskerfile, Instruction, InstructionKind};
use dosker_image::{
    BlobRef, BootLayout, BootMetadata, DosFlavour, FatType, Geometry, HistoryEntry, ImageConfig,
    MachineProfile, PortDeclaration, PortProtocol, VolumeDeclaration, fixed_timestamp,
};

fn workdir() -> dosker_image::DosPath {
    dosker_image::DosPath::parse(r"C:\PROJECT").expect("the workdir parses")
}

fn after_from(text: &str) -> Instruction {
    let source = format!("FROM base:latest\n{text}\n");
    Doskerfile::parse_str(&source)
        .expect("the Doskerfile parses")
        .instructions
        .pop()
        .expect("the instruction exists")
}

fn only(text: &str) -> Instruction {
    Doskerfile::parse_str(text)
        .expect("the Doskerfile parses")
        .instructions
        .pop()
        .expect("the instruction exists")
}

fn sample_config() -> ImageConfig {
    ImageConfig {
        schema_version: 1,
        dos_flavour: DosFlavour::Other,
        booter: false,
        boot: sample_boot(),
        machine: MachineProfile::default(),
        env: BTreeMap::new(),
        workdir: r"C:\".to_owned(),
        cmd: None,
        exposed_ports: Vec::new(),
        volumes: Vec::new(),
        engine_pin: None,
        engine_config: BTreeMap::new(),
        requires: Vec::new(),
        agent_version: None,
        created_by: "test".to_owned(),
        history: Vec::new(),
    }
}

fn sample_boot() -> BootMetadata {
    BootMetadata {
        layout: BootLayout::MbrPartitioned,
        disk_size: 16 * 1024 * 1024,
        geometry: Geometry {
            cylinders: 16,
            heads: 4,
            sectors: 32,
            sector_size: 512,
        },
        partition_start: Some(1),
        partition_sectors: Some(32_768),
        mbr: Some(BlobRef::from_bytes(&[0u8; 512])),
        boot_sector: Some(BlobRef::from_bytes(&[0u8; 512])),
        fat_type: Some(FatType::Fat16),
        volume_label: Some("DOSKER".to_owned()),
        media_descriptor: Some(0xf8),
    }
}

#[test]
fn render_tokens_quotes_whitespace_and_escapes() {
    assert_eq!(render_tokens(&["APP".to_owned()]), "APP");
    assert_eq!(render_tokens(&[String::new()]), r#""""#);
    assert_eq!(render_tokens(&["A B".to_owned()]), r#""A B""#);
    assert_eq!(
        render_tokens(&[r#"say "hi""#.to_owned()]),
        r#""say \"hi\"""#
    );
    assert_eq!(render_tokens(&[r"C:\APP".to_owned()]), r#""C:\\APP""#);
    assert_eq!(
        render_tokens(&["A".to_owned(), "B C".to_owned()]),
        r#"A "B C""#
    );
}

#[test]
fn normalize_run_host_keeps_the_host_flag() {
    let instruction = after_from(r#"RUN --host ECHO "hello world""#);

    assert_eq!(
        normalize_instruction(&instruction, &workdir()).expect("the instruction renders"),
        r#"RUN --host ECHO "hello world""#
    );
}

#[test]
fn normalize_run_host_keeps_quoted_and_unquoted_commands_distinct() {
    let quoted = after_from(r#"RUN --host printf '%s' "*.TXT""#);
    let unquoted = after_from(r"RUN --host printf '%s' *.TXT");

    let quoted = normalize_instruction(&quoted, &workdir()).expect("the instruction renders");
    let unquoted = normalize_instruction(&unquoted, &workdir()).expect("the instruction renders");

    assert_eq!(quoted, r#"RUN --host printf '%s' "*.TXT""#);
    assert_eq!(unquoted, r"RUN --host printf '%s' *.TXT");
    assert_ne!(quoted, unquoted, "the cache keys must not collide");
}

#[test]
fn normalize_cmd_keeps_the_raw_text() {
    let instruction = after_from(r"CMD C:\APP\BIN\APP.EXE /FAST");

    assert_eq!(
        normalize_instruction(&instruction, &workdir()).expect("the instruction renders"),
        r"CMD C:\APP\BIN\APP.EXE /FAST"
    );
}

#[test]
fn normalize_workdir_resolves_a_relative_path() {
    let instruction = after_from("WORKDIR sub");

    assert_eq!(
        normalize_instruction(&instruction, &workdir()).expect("the instruction renders"),
        r#"WORKDIR "C:\\PROJECT\\SUB""#
    );
}

#[test]
fn normalize_machine_sorts_keys() {
    let instruction = after_from("MACHINE video=vga cpu=386 cpu=pentium memory=32M");

    assert_eq!(
        normalize_instruction(&instruction, &workdir()).expect("the instruction renders"),
        "MACHINE cpu=pentium memory=32M video=vga"
    );
}

#[test]
fn normalize_requires_is_sorted_and_deduplicated() {
    let instruction = Instruction {
        line: 2,
        kind: InstructionKind::Requires {
            capabilities: vec![
                "gdbstub".to_owned(),
                "headless".to_owned(),
                "gdbstub".to_owned(),
            ],
        },
    };

    assert_eq!(
        normalize_instruction(&instruction, &workdir()).expect("the instruction renders"),
        "REQUIRES headless,gdbstub"
    );
}

#[test]
fn normalize_expose_makes_tcp_explicit() {
    let instruction = after_from("EXPOSE 80");

    assert_eq!(
        normalize_instruction(&instruction, &workdir()).expect("the instruction renders"),
        "EXPOSE 80/tcp"
    );
}

#[test]
fn normalize_env_quotes_a_value_with_spaces() {
    let instruction = after_from("ENV GREETING=hello world");

    assert_eq!(
        normalize_instruction(&instruction, &workdir()).expect("the instruction renders"),
        r#"ENV "GREETING=hello world""#
    );
}

#[test]
fn normalize_from_quotes_a_path_with_spaces() {
    let instruction = only(r#"FROM "./base image.img""#);

    assert_eq!(
        normalize_instruction(&instruction, &workdir()).expect("the instruction renders"),
        r#"FROM "./base image.img""#
    );
}

#[test]
fn resolve_copy_destination_requires_c_drive() {
    let error =
        resolve_copy_destination(&workdir(), r"D:\APP").expect_err("a non-C drive is rejected");

    assert!(matches!(error, BuildError::Context { .. }));
}

#[test]
fn resolve_copy_destination_rejects_traversal() {
    assert!(resolve_copy_destination(&workdir(), r"..\APP").is_err());
}

#[test]
fn resolve_copy_destination_marks_a_trailing_separator() {
    let path = dosker_image::DosPath::parse(r"C:\PROJECT\APP").unwrap();
    assert_eq!(
        resolve_copy_destination(&workdir(), r"APP\").expect("the destination resolves"),
        (path, true)
    );

    let root = dosker_image::DosPath::parse("").unwrap();
    assert_eq!(
        resolve_copy_destination(&workdir(), r"C:\").expect("the root resolves"),
        (root, true)
    );
}

#[test]
fn build_config_digest_changes_for_each_effective_field() {
    let base = build_config_digest(&sample_config()).expect("the digest computes");
    assert!(base.starts_with("sha256:"));
    let mut seen = BTreeSet::new();
    seen.insert(base.clone());

    let mut configs = Vec::new();

    let mut env = sample_config();
    env.env.insert("GREETING".to_owned(), "hello".to_owned());
    configs.push(env);

    let mut workdir = sample_config();
    workdir.workdir = r"C:\APP".to_owned();
    configs.push(workdir);

    let mut machine = sample_config();
    machine.machine.cpu = "pentium".to_owned();
    configs.push(machine);

    let mut engine = sample_config();
    engine.engine_pin = Some("qemu".to_owned());
    configs.push(engine);

    let mut requires = sample_config();
    requires.requires.push("headless".to_owned());
    configs.push(requires);

    let mut ports = sample_config();
    ports.exposed_ports.push(PortDeclaration {
        port: 80,
        protocol: PortProtocol::Tcp,
    });
    configs.push(ports);

    let mut volumes = sample_config();
    volumes.volumes.push(VolumeDeclaration {
        drive: "D:".to_owned(),
    });
    configs.push(volumes);

    let mut cmd = sample_config();
    cmd.cmd = Some("APP.EXE".to_owned());
    configs.push(cmd);

    for config in configs {
        let digest = build_config_digest(&config).expect("the digest computes");
        assert_ne!(digest, base);
        assert!(seen.insert(digest));
    }
}

#[test]
fn build_config_digest_ignores_history_and_boot() {
    let config = sample_config();
    let before = build_config_digest(&config).expect("the digest computes");

    let mut changed = config;
    changed.boot.disk_size *= 2;
    changed.created_by = "someone else".to_owned();
    changed.agent_version = Some("1.2.3".to_owned());
    changed
        .engine_config
        .insert("qemu".to_owned(), vec!["-m 16".to_owned()]);
    changed.history.push(HistoryEntry {
        created_by: "later".to_owned(),
        created_at: fixed_timestamp(),
        empty_layer: true,
        comment: None,
    });

    assert_eq!(
        before,
        build_config_digest(&changed).expect("the digest computes")
    );
}
