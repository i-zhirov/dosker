use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use dosker_build::{BuildError, BuildState, normalize_instruction};
use dosker_core::{Doskerfile, EngineName, Instruction, InstructionKind};
use dosker_image::{
    Attributes, BlobRef, BootLayout, BootMetadata, Descriptor, DosFlavour, DosPath, FatType,
    Geometry, HistoryEntry, ImageConfig, Layer, LayerDelta, MachineProfile, PortDeclaration,
    PortProtocol, Tree, TreeFile, TreeNode, VolumeDeclaration, fixed_timestamp,
};

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

fn tree_config() -> ImageConfig {
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
        created_by: "FROM base:latest".to_owned(),
        history: vec![HistoryEntry {
            created_by: "FROM base:latest".to_owned(),
            created_at: fixed_timestamp(),
            empty_layer: false,
            comment: None,
        }],
    }
}

fn booter_config() -> ImageConfig {
    ImageConfig {
        schema_version: 1,
        dos_flavour: DosFlavour::None,
        booter: true,
        boot: BootMetadata::opaque(1024),
        machine: MachineProfile::default(),
        env: BTreeMap::new(),
        workdir: String::new(),
        cmd: None,
        exposed_ports: Vec::new(),
        volumes: Vec::new(),
        engine_pin: None,
        engine_config: BTreeMap::new(),
        requires: Vec::new(),
        agent_version: None,
        created_by: "FROM base:latest".to_owned(),
        history: vec![HistoryEntry {
            created_by: "FROM base:latest".to_owned(),
            created_at: fixed_timestamp(),
            empty_layer: false,
            comment: None,
        }],
    }
}

fn base_descriptor() -> Descriptor {
    Descriptor {
        media_type: "application/vnd.oci.image.layer.v1.tar".to_owned(),
        digest: "sha256:head".to_owned(),
        size: 1,
        annotations: None,
    }
}

fn tree_state() -> BuildState {
    BuildState::new(tree_config(), Some(Tree::new()), &[], EngineName::Qemu)
}

fn booter_state() -> BuildState {
    BuildState::new(booter_config(), None, &[], EngineName::Qemu)
}

fn instruction(text: &str) -> Instruction {
    Doskerfile::parse_str(&format!("FROM base:latest\n{text}\n"))
        .expect("the Doskerfile parses")
        .instructions
        .into_iter()
        .nth(1)
        .expect("the instruction exists")
}

fn workdir_of(state: &BuildState) -> DosPath {
    DosPath::parse(&state.config.workdir).expect("the workdir parses")
}

fn apply_text(state: &mut BuildState, text: &str, now: DateTime<Utc>) -> Result<(), BuildError> {
    let instruction = instruction(text);
    let normalized = normalize_instruction(&instruction, &workdir_of(state))?;
    state.apply_config(&instruction, &normalized, now)
}

fn apply_kind(
    state: &mut BuildState,
    kind: InstructionKind,
    now: DateTime<Utc>,
) -> Result<(), BuildError> {
    let instruction = Instruction { line: 2, kind };
    let normalized = normalize_instruction(&instruction, &workdir_of(state))?;
    state.apply_config(&instruction, &normalized, now)
}

#[test]
fn env_updates_the_config_and_history() {
    let mut state = tree_state();
    let now = Utc::now();

    apply_text(&mut state, "ENV GREETING=hello world", now).expect("ENV applies");

    assert_eq!(
        state.config.env.get("GREETING"),
        Some(&"hello world".to_owned())
    );
    let history = state.config.history.last().expect("a history entry exists");
    assert_eq!(history.created_by, r#"ENV "GREETING=hello world""#);
    assert!(history.empty_layer);
    assert_eq!(history.created_at, now);
    assert_eq!(state.config.created_by, history.created_by);
    assert!(state.layers.is_empty());
    assert_eq!(state.head_layer_digest, None);
}

#[test]
fn workdir_resolves_and_creates_the_directory() {
    let mut state = tree_state();
    let now = Utc::now();

    apply_text(&mut state, r"WORKDIR APP\SUB", now).expect("WORKDIR applies");

    assert_eq!(state.config.workdir, r"C:\APP\SUB");
    for value in [r"APP", r"APP\SUB"] {
        let path = DosPath::parse(value).expect("the path parses");
        let node = state
            .tree
            .as_ref()
            .expect("the tree exists")
            .get(&path)
            .expect("the directory exists");
        assert!(matches!(node, TreeNode::Dir(_)));
        assert_eq!(node.attributes(), &Attributes::default());
        assert_eq!(node.timestamp(), fixed_timestamp());
    }
}

#[test]
fn workdir_adds_history_but_no_layer() {
    let mut state = tree_state();
    let now = Utc::now();

    apply_text(&mut state, "WORKDIR APP", now).expect("WORKDIR applies");

    assert!(state.layers.is_empty());
    let history = state.config.history.last().expect("a history entry exists");
    assert!(history.empty_layer);
    assert_eq!(history.created_by, r#"WORKDIR "C:\\APP""#);
    assert_eq!(history.created_at, now);
}

#[test]
fn workdir_does_not_change_the_head_layer() {
    let mut state = BuildState::new(
        tree_config(),
        Some(Tree::new()),
        &[base_descriptor()],
        EngineName::Qemu,
    );
    let head = state.head_layer_digest.clone();

    apply_text(&mut state, "WORKDIR APP", Utc::now()).expect("WORKDIR applies");

    assert_eq!(state.head_layer_digest, head);
    assert_eq!(state.head_layer_digest.as_deref(), Some("sha256:head"));
    assert!(state.layers.is_empty());
}

#[test]
fn filesystem_layer_updates_the_head_and_tree() {
    let base = base_descriptor();
    let mut state = BuildState::new(tree_config(), Some(Tree::new()), &[base], EngineName::Qemu);
    let path = DosPath::parse(r"APP\FOO.TXT").expect("the path parses");
    let mut delta = LayerDelta::new();
    delta.add_file(TreeFile {
        path: path.clone(),
        attributes: Attributes {
            archive: true,
            ..Attributes::default()
        },
        timestamp: fixed_timestamp(),
        bytes: b"hello".to_vec(),
    });
    let layer = Layer::from_delta(
        &delta,
        "COPY FOO.TXT C:\\APP\\",
        Some("sha256:head".to_owned()),
        None,
    )
    .expect("the layer builds");
    let descriptor = Descriptor {
        media_type: "application/vnd.oci.image.layer.v1.tar".to_owned(),
        digest: layer.digest().expect("the layer digests"),
        size: 128,
        annotations: None,
    };
    let now = Utc::now();

    state
        .apply_layer(layer, descriptor.clone(), "COPY FOO.TXT C:\\APP\\", now)
        .expect("the layer applies");

    assert_eq!(state.layers, vec![descriptor.clone()]);
    assert_eq!(state.head_layer_digest, Some(descriptor.digest));
    assert!(matches!(
        state.tree.as_ref().expect("the tree exists").get(&path),
        Some(TreeNode::File(_))
    ));
    let history = state.config.history.last().expect("a history entry exists");
    assert!(!history.empty_layer);
    assert_eq!(history.created_at, now);
    assert_eq!(state.config.created_by, "COPY FOO.TXT C:\\APP\\");
}

#[test]
fn config_only_instructions_do_not_change_the_head_layer() {
    let mut state = BuildState::new(
        tree_config(),
        Some(Tree::new()),
        &[base_descriptor()],
        EngineName::Qemu,
    );
    let now = Utc::now();
    let base_history = state.config.history.len();

    apply_text(&mut state, "ENV A=1", now).expect("ENV applies");
    apply_text(&mut state, "WORKDIR APP", now).expect("WORKDIR applies");
    apply_text(&mut state, "CMD ECHO HI", now).expect("CMD applies");
    apply_text(&mut state, "MACHINE video=ega", now).expect("MACHINE applies");
    apply_text(&mut state, "ENGINE qemu", now).expect("ENGINE applies");
    apply_text(&mut state, "ENGINE-CONFIG qemu -m 32", now).expect("ENGINE-CONFIG applies");
    apply_text(&mut state, "REQUIRES headless", now).expect("REQUIRES applies");
    apply_text(&mut state, "EXPOSE 80", now).expect("EXPOSE applies");
    apply_text(&mut state, "VOLUME D:", now).expect("VOLUME applies");

    assert_eq!(state.head_layer_digest.as_deref(), Some("sha256:head"));
    assert!(state.layers.is_empty());
    assert_eq!(state.config.history.len(), base_history + 9);
    assert!(
        state.config.history[base_history..]
            .iter()
            .all(|entry| entry.empty_layer)
    );
}

#[test]
fn cmd_replaces_an_earlier_command() {
    let mut state = tree_state();
    let now = Utc::now();

    apply_text(&mut state, "CMD ECHO FIRST", now).expect("the first CMD applies");
    apply_text(&mut state, "CMD ECHO SECOND", now).expect("the second CMD applies");

    assert_eq!(state.config.cmd.as_deref(), Some("ECHO SECOND"));
}

#[test]
fn machine_merges_and_validates() {
    let mut state = tree_state();
    let now = Utc::now();

    apply_text(
        &mut state,
        "MACHINE cpu=386 memory=32M video=ega sound=none net=none serial=3 control_port=3 disk=ide baud=9600",
        now,
    )
    .expect("MACHINE applies");

    assert_eq!(state.config.machine.cpu, "386");
    assert_eq!(state.config.machine.memory_mb, 32);
    assert_eq!(state.config.machine.video, "ega");
    assert_eq!(state.config.machine.sound, "none");
    assert_eq!(state.config.machine.net, "none");
    assert_eq!(state.config.machine.serial_ports, 3);
    assert_eq!(state.config.machine.control_port, 3);
    assert_eq!(state.config.machine.disk_interface, "ide");
    assert_eq!(state.config.machine.baud, 9_600);

    apply_text(&mut state, "MACHINE cpu=pentium video=svga", now).expect("MACHINE merges");

    assert_eq!(state.config.machine.cpu, "pentium");
    assert_eq!(state.config.machine.video, "svga");
    assert_eq!(state.config.machine.memory_mb, 32);
    assert_eq!(state.config.machine.baud, 9_600);
}

#[test]
fn machine_rejects_an_out_of_range_value() {
    let mut state = tree_state();
    let original = state.config.machine.clone();

    let error = apply_text(&mut state, "MACHINE memory=512M", Utc::now())
        .expect_err("an out-of-range memory value is rejected");

    assert!(matches!(
        error,
        BuildError::Instruction {
            instruction,
            ..
        } if instruction == "MACHINE"
    ));
    assert_eq!(state.config.machine, original);
}

#[test]
fn engine_sets_the_pin() {
    let mut state = tree_state();

    apply_text(&mut state, "ENGINE dosbox-x", Utc::now()).expect("ENGINE applies");

    assert_eq!(state.config.engine_pin.as_deref(), Some("dosbox-x"));
    assert_eq!(state.selected_engine, EngineName::DosboxX);
}

#[test]
fn engine_config_appends_lines_in_order() {
    let mut state = tree_state();

    apply_text(&mut state, "ENGINE-CONFIG qemu -m 32", Utc::now()).expect("the first line applies");
    apply_text(&mut state, "ENGINE-CONFIG qemu -vga std", Utc::now())
        .expect("the second line applies");

    assert_eq!(
        state.config.engine_config.get("qemu"),
        Some(&vec!["-m 32".to_owned(), "-vga std".to_owned()])
    );
}

#[test]
fn requires_merges_and_sorts() {
    let mut state = tree_state();
    state.config.requires = vec!["serial".to_owned()];

    apply_text(&mut state, "REQUIRES gdbstub,headless,gdbstub", Utc::now())
        .expect("REQUIRES applies");

    assert_eq!(
        state.config.requires,
        vec![
            "serial".to_owned(),
            "headless".to_owned(),
            "gdbstub".to_owned()
        ]
    );
}

#[test]
fn requires_rejects_an_unknown_capability() {
    let mut state = tree_state();
    state.config.requires = vec!["serial".to_owned()];

    let error = apply_text(&mut state, "REQUIRES teleport", Utc::now())
        .expect_err("an unknown capability is rejected");

    assert!(matches!(error, BuildError::Instruction { .. }));
    assert_eq!(state.config.requires, vec!["serial".to_owned()]);
}

#[test]
fn expose_merges_and_sorts() {
    let mut config = tree_config();
    config.exposed_ports = vec![PortDeclaration {
        port: 80,
        protocol: PortProtocol::Tcp,
    }];
    let mut state = BuildState::new(config, Some(Tree::new()), &[], EngineName::Qemu);

    apply_text(&mut state, "EXPOSE 22", Utc::now()).expect("EXPOSE applies");

    assert_eq!(
        state
            .config
            .exposed_ports
            .iter()
            .map(|declaration| (declaration.port, declaration.protocol))
            .collect::<Vec<_>>(),
        vec![(22, PortProtocol::Tcp), (80, PortProtocol::Tcp)]
    );
}

#[test]
fn expose_merges_duplicate_ports() {
    let mut config = tree_config();
    config.exposed_ports = vec![PortDeclaration {
        port: 80,
        protocol: PortProtocol::Tcp,
    }];
    let mut state = BuildState::new(config, Some(Tree::new()), &[], EngineName::Qemu);

    apply_text(&mut state, "EXPOSE 80/tcp", Utc::now()).expect("a duplicate EXPOSE applies");
    apply_text(&mut state, "EXPOSE 80", Utc::now()).expect("a duplicate EXPOSE applies");

    assert_eq!(
        state.config.exposed_ports,
        vec![PortDeclaration {
            port: 80,
            protocol: PortProtocol::Tcp,
        }]
    );
}

#[test]
fn volume_rejects_c_and_duplicates() {
    let mut config = tree_config();
    config.volumes = vec![VolumeDeclaration {
        drive: "D:".to_owned(),
    }];
    let mut state = BuildState::new(config, Some(Tree::new()), &[], EngineName::Qemu);

    let reserved = apply_kind(
        &mut state,
        InstructionKind::Volume {
            drive: "C:".to_owned(),
        },
        Utc::now(),
    );
    assert!(reserved.is_err());

    let duplicate =
        apply_text(&mut state, "VOLUME D:", Utc::now()).expect_err("a duplicate drive is rejected");
    assert!(matches!(duplicate, BuildError::Instruction { .. }));

    assert_eq!(
        state
            .config
            .volumes
            .iter()
            .map(|volume| volume.drive.as_str())
            .collect::<Vec<_>>(),
        vec!["D:"]
    );
}

#[test]
fn a_booter_rejects_tree_config_instructions() {
    for text in [
        "ENV A=1",
        "WORKDIR APP",
        "CMD ECHO HI",
        "EXPOSE 80",
        "VOLUME D:",
    ] {
        let mut state = booter_state();
        let history = state.config.history.len();

        let error = apply_text(&mut state, text, Utc::now())
            .expect_err("a tree config instruction is rejected on a booter");

        assert!(matches!(error, BuildError::Instruction { .. }));
        assert!(state.config.env.is_empty());
        assert_eq!(state.config.workdir, "");
        assert_eq!(state.config.cmd, None);
        assert!(state.config.exposed_ports.is_empty());
        assert!(state.config.volumes.is_empty());
        assert_eq!(state.config.history.len(), history);
    }
}

#[test]
fn a_booter_accepts_machine_engine_engine_config_and_requires() {
    let mut state = booter_state();
    let now = Utc::now();
    let history = state.config.history.len();

    apply_text(&mut state, "MACHINE cpu=pentium memory=32M", now).expect("MACHINE applies");
    apply_text(&mut state, "ENGINE bochs", now).expect("ENGINE applies");
    apply_text(&mut state, "ENGINE-CONFIG bochs key=value", now).expect("ENGINE-CONFIG applies");
    apply_text(&mut state, "REQUIRES headless", now).expect("REQUIRES applies");

    assert_eq!(state.config.machine.cpu, "pentium");
    assert_eq!(state.config.machine.memory_mb, 32);
    assert_eq!(state.config.engine_pin.as_deref(), Some("bochs"));
    assert_eq!(state.selected_engine, EngineName::Bochs);
    assert_eq!(
        state.config.engine_config.get("bochs"),
        Some(&vec!["key=value".to_owned()])
    );
    assert_eq!(state.config.requires, vec!["headless".to_owned()]);
    assert_eq!(state.config.history.len(), history + 4);
    assert!(
        state.config.history[history..]
            .iter()
            .all(|entry| entry.empty_layer)
    );
}
