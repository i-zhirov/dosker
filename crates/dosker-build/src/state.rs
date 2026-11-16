//! Build configuration state for config-only instructions.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use dosker_core::{EngineName, Instruction, InstructionKind};
use dosker_engine::Capability;
use dosker_image::{
    Attributes, Descriptor, DosPath, HistoryEntry, ImageConfig, Layer, LayerDelta, MachineProfile,
    PortDeclaration, PortProtocol, Tree, TreeDir, TreeNode, VolumeDeclaration, fixed_timestamp,
};

use crate::error::BuildError;
use crate::normalize::{build_config_digest, resolve_dos_path, sorted_capabilities};

/// The mutable state of one build step sequence.
pub struct BuildState {
    /// The effective image config seen so far.
    pub config: ImageConfig,
    /// The staged tree, or `None` for a booter image.
    pub tree: Option<Tree>,
    /// Filesystem layers produced by this build.
    pub layers: Vec<Descriptor>,
    /// The digest of the last layer in the image chain.
    pub head_layer_digest: Option<String>,
    /// The engine selected by the build.
    pub selected_engine: EngineName,
}

impl BuildState {
    /// Creates the state from the base image and its layer chain.
    pub fn new(
        config: ImageConfig,
        tree: Option<Tree>,
        base_layers: &[Descriptor],
        selected_engine: EngineName,
    ) -> Self {
        Self {
            config,
            tree,
            layers: Vec::new(),
            head_layer_digest: base_layers.last().map(|layer| layer.digest.clone()),
            selected_engine,
        }
    }

    /// Returns the staged tree digest, or an empty string when there is no tree.
    pub fn parent_tree_digest(&self) -> Result<String, BuildError> {
        match &self.tree {
            Some(tree) => Ok(tree.digest()?),
            None => Ok(String::new()),
        }
    }

    /// Returns the digest of the effective build configuration.
    pub fn config_digest(&self) -> Result<String, BuildError> {
        build_config_digest(&self.config)
    }

    /// Applies a filesystem layer and records its history entry.
    ///
    /// This is the only method that changes the layer list or the head layer.
    pub fn apply_layer(
        &mut self,
        layer: Layer,
        descriptor: Descriptor,
        normalized: &str,
        now: DateTime<Utc>,
    ) -> Result<(), BuildError> {
        let tree = self
            .tree
            .as_mut()
            .ok_or_else(|| instruction_error_from_name(normalized, "a booter image has no tree"))?;
        layer.apply_to(tree)?;
        self.layers.push(descriptor.clone());
        self.head_layer_digest = Some(descriptor.digest);
        self.record(normalized, now, false);
        Ok(())
    }

    /// Applies one config-only instruction and records an empty history entry.
    ///
    /// This never changes the layer list or the head layer.
    pub fn apply_config(
        &mut self,
        instruction: &Instruction,
        normalized: &str,
        now: DateTime<Utc>,
    ) -> Result<(), BuildError> {
        if self.config.booter {
            match &instruction.kind {
                InstructionKind::Machine { .. }
                | InstructionKind::Engine { .. }
                | InstructionKind::EngineConfig { .. }
                | InstructionKind::Requires { .. } => {}
                _ => {
                    return Err(instruction_error(
                        instruction,
                        "a booter image accepts only MACHINE, ENGINE, ENGINE-CONFIG, and REQUIRES",
                    ));
                }
            }
        }

        match &instruction.kind {
            InstructionKind::Env { key, value } => {
                self.config.env.insert(key.clone(), value.clone());
            }
            InstructionKind::Workdir { path } => {
                let workdir = DosPath::parse(&self.config.workdir)?;
                let resolved = resolve_dos_path(&workdir, path)
                    .map_err(|error| instruction_error(instruction, error.to_string()))?;
                let tree = self.tree.get_or_insert_with(Tree::new);
                ensure_directory(tree, &resolved, instruction)?;
                self.config.workdir = resolved.to_dos_string();
            }
            InstructionKind::Cmd { raw, .. } => {
                self.config.cmd = Some(raw.clone());
            }
            InstructionKind::Machine { settings } => {
                let mut machine = self.config.machine.clone();
                apply_machine(&mut machine, settings)
                    .map_err(|reason| instruction_error(instruction, reason))?;
                machine
                    .validate()
                    .map_err(|error| instruction_error(instruction, error.to_string()))?;
                self.config.machine = machine;
            }
            InstructionKind::Engine { name } => {
                self.config.engine_pin = Some(name.as_str().to_owned());
                self.selected_engine = *name;
            }
            InstructionKind::EngineConfig { engine, raw } => {
                self.config
                    .engine_config
                    .entry(engine.as_str().to_owned())
                    .or_default()
                    .push(raw.clone());
            }
            InstructionKind::Requires { capabilities } => {
                let mut merged = self.config.requires.clone();
                for capability in capabilities {
                    if Capability::from_name(capability).is_none() {
                        return Err(instruction_error(
                            instruction,
                            format!("unknown capability {capability}"),
                        ));
                    }
                    merged.push(capability.clone());
                }
                self.config.requires = sorted_capabilities(&merged);
            }
            InstructionKind::Expose { port, protocol } => {
                if !protocol.eq_ignore_ascii_case("tcp") {
                    return Err(instruction_error(
                        instruction,
                        format!("unsupported EXPOSE protocol {protocol}"),
                    ));
                }
                self.config.exposed_ports.push(PortDeclaration {
                    port: *port,
                    protocol: PortProtocol::Tcp,
                });
                self.config
                    .exposed_ports
                    .sort_by_key(|declaration| declaration.port);
                self.config.exposed_ports.dedup();
            }
            InstructionKind::Volume { drive } => {
                let drive = drive.to_ascii_uppercase();
                if !valid_volume_drive(&drive) {
                    return Err(instruction_error(
                        instruction,
                        format!("invalid VOLUME drive {drive}"),
                    ));
                }
                if drive == "C:" {
                    return Err(instruction_error(
                        instruction,
                        "C: is reserved for the boot disk",
                    ));
                }
                if self
                    .config
                    .volumes
                    .iter()
                    .any(|volume| volume.drive == drive)
                {
                    return Err(instruction_error(
                        instruction,
                        format!("duplicate VOLUME drive {drive}"),
                    ));
                }
                self.config.volumes.push(VolumeDeclaration { drive });
                self.config
                    .volumes
                    .sort_by(|left, right| left.drive.cmp(&right.drive));
            }
            _ => {
                return Err(instruction_error(
                    instruction,
                    "not a config-only instruction",
                ));
            }
        }

        self.record(normalized, now, true);
        Ok(())
    }

    fn record(&mut self, normalized: &str, now: DateTime<Utc>, empty_layer: bool) {
        self.config.created_by = normalized.to_owned();
        self.config.history.push(HistoryEntry {
            created_by: normalized.to_owned(),
            created_at: now,
            empty_layer,
            comment: None,
        });
    }
}

fn apply_machine(
    machine: &mut MachineProfile,
    settings: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (key, value) in settings {
        match key.as_str() {
            "cpu" => machine.cpu = value.clone(),
            "memory" => {
                let digits = value
                    .strip_suffix('M')
                    .ok_or_else(|| format!("memory {value} needs an M suffix"))?;
                machine.memory_mb = digits
                    .parse::<u16>()
                    .map_err(|_| format!("invalid memory value {value}"))?;
            }
            "video" => machine.video = value.clone(),
            "sound" => machine.sound = value.clone(),
            "net" => machine.net = value.clone(),
            "serial" => {
                machine.serial_ports = value
                    .parse::<u8>()
                    .map_err(|_| format!("invalid serial value {value}"))?;
            }
            "control_port" => {
                machine.control_port = value
                    .parse::<u8>()
                    .map_err(|_| format!("invalid control_port value {value}"))?;
            }
            "disk" => machine.disk_interface = value.clone(),
            "baud" => {
                machine.baud = value
                    .parse::<u32>()
                    .map_err(|_| format!("invalid baud value {value}"))?;
            }
            other => return Err(format!("unknown MACHINE key {other}")),
        }
    }
    Ok(())
}

fn ensure_directory(
    tree: &mut Tree,
    path: &DosPath,
    instruction: &Instruction,
) -> Result<(), BuildError> {
    if path.is_root() {
        return Ok(());
    }
    match tree.get(path) {
        Some(TreeNode::Dir(_)) => return Ok(()),
        Some(TreeNode::File(_)) => {
            return Err(instruction_error(
                instruction,
                format!("WORKDIR path {} is a file", path.to_dos_string()),
            ));
        }
        None => {}
    }

    let mut delta = LayerDelta::new();
    delta.add_dir(TreeDir {
        path: path.clone(),
        attributes: Attributes::default(),
        timestamp: fixed_timestamp(),
    });
    tree.apply_delta(&delta)
        .map_err(|error| instruction_error(instruction, error.to_string()))
}

fn valid_volume_drive(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_uppercase() && bytes[1] == b':'
}

fn instruction_error(instruction: &Instruction, reason: impl Into<String>) -> BuildError {
    BuildError::Instruction {
        line: instruction.line,
        instruction: instruction.kind.name().to_owned(),
        reason: reason.into(),
    }
}

fn instruction_error_from_name(normalized: &str, reason: impl Into<String>) -> BuildError {
    BuildError::Instruction {
        line: 0,
        instruction: normalized
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned(),
        reason: reason.into(),
    }
}
