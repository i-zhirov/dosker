//! In-DOS `RUN` requests and read-back tree deltas.

pub mod engine;

pub use engine::{EngineInDosRunner, EngineOpener, QemuEngineOpener};

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use dosker_core::EngineName;
use dosker_image::{
    Attributes, BootMetadata, DosPath, LayerDelta, Tree, TreeDir, TreeFile, TreeNode,
    fixed_timestamp,
};

use crate::error::BuildError;
use crate::normalize::is_reserved_path;

/// How long the build waits for the agent to announce itself.
pub const RUN_BOOT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the guest has to shut down after the command.
pub const RUN_STOP_GRACE: Duration = Duration::from_secs(10);

/// Reported when the command produced no captured text.
pub const RUN_EMPTY_OUTPUT_NOTE: &str = "RUN produced no captured output";

/// Reported when the captured output was truncated.
pub const RUN_TRUNCATED_NOTE: &str = "RUN output was truncated";

/// One in-DOS `RUN` command.
pub struct InDosRunRequest<'a> {
    /// The Doskerfile line that holds the instruction.
    pub line: usize,
    /// The command sent to the agent.
    pub command: &'a str,
    /// The image workdir the command runs in.
    pub workdir: &'a DosPath,
    /// The image environment passed to the command.
    pub env: &'a BTreeMap<String, String>,
    /// The maximum time the command may run.
    pub timeout: Duration,
    /// The maximum time to wait for the agent.
    pub boot_timeout: Duration,
    /// The private disk the VM boots.
    pub disk_path: &'a Path,
    /// The disk interface the engine attaches.
    pub disk_interface: &'a str,
    /// The number of serial ports the engine exposes.
    pub serial_ports: u8,
    /// The serial port that carries the agent.
    pub control_port: u8,
    /// The directory for engine logs.
    pub log_dir: &'a Path,
    /// The build root that holds the container sockets.
    pub root: &'a Path,
    /// The engine selected for this step.
    pub engine: EngineName,
}

/// The result of one in-DOS `RUN` command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InDosRunOutcome {
    /// The DOS exit code the command returned.
    pub exit_code: i32,
    /// The text the command captured.
    pub output: String,
    /// True when the command produced more output than was captured.
    pub truncated: bool,
}

/// Runs one command inside a booted image.
pub trait InDosRunner {
    /// Boots the disk, runs the command, and shuts the guest down.
    fn run(&mut self, request: &InDosRunRequest<'_>) -> Result<InDosRunOutcome, BuildError>;
}

/// The tree delta and boot override one in-DOS `RUN` produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunDelta {
    /// The filesystem changes to store in a layer.
    pub delta: LayerDelta,
    /// The effective boot metadata when the boot artifacts changed.
    pub boot: Option<BootMetadata>,
}

/// Folds the read-back tree into a layer delta.
///
/// Timestamps and attributes are ignored for change detection: the FAT
/// writer cannot preserve them. Entries under `C:\DOSKER` are never part of
/// the delta because the builder owns that directory.
pub fn build_delta(
    staged: &Tree,
    read_back: &Tree,
    boot_before: &BootMetadata,
    boot_after: &BootMetadata,
) -> RunDelta {
    let mut delta = LayerDelta::new();

    let changed: BTreeSet<DosPath> = staged
        .iter()
        .filter(|(path, _)| !is_reserved_path(path))
        .filter_map(|(path, node)| match read_back.get(path) {
            Some(read_back_node) if node.kind() == read_back_node.kind() => None,
            _ => Some(path.clone()),
        })
        .collect();

    for path in &changed {
        if topmost_change(path, &changed) {
            delta.whiteout(path.clone());
        }
    }

    for (path, node) in read_back.iter() {
        if is_reserved_path(path) {
            continue;
        }
        match node {
            TreeNode::File(file) => {
                if matches!(
                    staged.get(path),
                    Some(TreeNode::File(staged_file)) if staged_file.bytes == file.bytes
                ) {
                    continue;
                }
                delta.add_file(TreeFile {
                    path: path.clone(),
                    attributes: Attributes {
                        archive: true,
                        ..Attributes::default()
                    },
                    timestamp: fixed_timestamp(),
                    bytes: file.bytes.clone(),
                });
            }
            TreeNode::Dir(_) => {
                if matches!(staged.get(path), Some(TreeNode::Dir(_))) {
                    continue;
                }
                delta.add_dir(TreeDir {
                    path: path.clone(),
                    attributes: Attributes::default(),
                    timestamp: fixed_timestamp(),
                });
            }
        }
    }

    let boot = if boot_before.boot_artifacts_equal(boot_after) {
        None
    } else {
        Some(boot_after.clone())
    };

    RunDelta { delta, boot }
}

fn topmost_change(path: &DosPath, changed: &BTreeSet<DosPath>) -> bool {
    let mut parent = path.parent();
    while let Some(current) = parent {
        if changed.contains(&current) {
            return false;
        }
        parent = current.parent();
    }
    true
}
