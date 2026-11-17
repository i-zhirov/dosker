//! Agent injection planning for imported DOS images.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use fatfs::FileSystem;

use crate::error::ImageError;
use crate::fat::{self, PartitionFile};
use crate::layer::fixed_timestamp;
use crate::path::DosPath;
use crate::tree::{Attributes, LayerDelta, Tree, TreeDir, TreeFile, TreeNode};

/// The agent directory name at the image root.
pub const AGENT_DIR: &str = "DOSKER";
/// The agent executable file name.
pub const AGENT_FILE: &str = "DOSKER.EXE";
/// The full DOS path of the agent executable.
pub const AGENT_PATH: &str = r"C:\DOSKER\DOSKER.EXE";
/// The DOS path of the image startup batch file.
pub const AUTOEXEC_PATH: &str = r"C:\AUTOEXEC.BAT";
/// The DOS path of the saved original startup batch file.
pub const AUTOEXEC_OLD_PATH: &str = r"C:\DOSKER\AUTOEXEC.OLD";

/// The startup batch file that runs the saved original, then the agent.
pub const WRAPPED_AUTOEXEC_BYTES: &[u8] =
    b"@ECHO OFF\r\nCALL C:\\DOSKER\\AUTOEXEC.OLD\r\nC:\\DOSKER\\DOSKER.EXE\r\n";

/// The startup batch file written when the image has none.
pub const NEW_AUTOEXEC_BYTES: &[u8] = b"@ECHO OFF\r\nC:\\DOSKER\\DOSKER.EXE\r\n";

/// The agent changes to apply to an imported DOS image.
#[derive(Clone, Debug)]
pub struct AgentInjection {
    pub applied: bool,
    pub delta: LayerDelta,
}

/// Plans the agent files and the startup batch changes for one image tree.
pub fn plan_agent_injection(tree: &Tree, agent: &[u8]) -> Result<AgentInjection, ImageError> {
    let agent_dir = DosPath::parse(AGENT_DIR)?;
    let agent_file = DosPath::parse(AGENT_PATH)?;
    let autoexec = DosPath::parse(AUTOEXEC_PATH)?;
    let autoexec_old = DosPath::parse(AUTOEXEC_OLD_PATH)?;

    reject_file(tree, &agent_dir, "the agent directory path is a file")?;
    reject_dir(
        tree,
        &agent_file,
        "the agent executable path is a directory",
    )?;
    reject_dir(tree, &autoexec, "the startup batch path is a directory")?;
    reject_dir(
        tree,
        &autoexec_old,
        "the saved startup batch path is a directory",
    )?;

    let mut delta = LayerDelta::new();

    if tree.get(&agent_dir).is_none() {
        delta.add_dir(TreeDir {
            path: agent_dir,
            attributes: Attributes::default(),
            timestamp: fixed_timestamp(),
        });
    }

    let existing_agent = match tree.get(&agent_file) {
        Some(TreeNode::File(file)) => Some(file),
        _ => None,
    };
    if existing_agent.is_none_or(|file| file.bytes != agent) {
        delta.add_file(TreeFile {
            path: agent_file,
            attributes: existing_agent
                .map(|file| file.attributes.clone())
                .unwrap_or_default(),
            timestamp: fixed_timestamp(),
            bytes: agent.to_vec(),
        });
    }

    match tree.get(&autoexec) {
        Some(TreeNode::File(file)) => {
            if !starts_agent(&file.bytes) {
                if tree.get(&autoexec_old).is_some() {
                    return Err(conflict(
                        &autoexec_old,
                        "the saved startup batch already exists",
                    ));
                }
                delta.add_file(TreeFile {
                    path: autoexec_old,
                    attributes: Attributes::default(),
                    timestamp: fixed_timestamp(),
                    bytes: file.bytes.clone(),
                });
                delta.add_file(TreeFile {
                    path: autoexec,
                    attributes: file.attributes.clone(),
                    timestamp: fixed_timestamp(),
                    bytes: WRAPPED_AUTOEXEC_BYTES.to_vec(),
                });
            }
        }
        None => {
            delta.add_file(TreeFile {
                path: autoexec,
                attributes: Attributes::default(),
                timestamp: fixed_timestamp(),
                bytes: NEW_AUTOEXEC_BYTES.to_vec(),
            });
        }
        Some(TreeNode::Dir(_)) => {}
    }

    let applied = !delta.dirs.is_empty() || !delta.files.is_empty();
    Ok(AgentInjection { applied, delta })
}

/// Applies the planned agent changes to the raw disk image in place.
pub fn apply_agent_injection(image: &Path, injection: &AgentInjection) -> Result<(), ImageError> {
    if !injection.applied {
        return Ok(());
    }

    let region = fat::fat_region(image)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(image)
        .map_err(|source| ImageError::Io {
            path: image.to_path_buf(),
            source,
        })?;
    let disk = PartitionFile::new(file, region.offset, region.length);
    let fs = FileSystem::new(disk, fat::fixed_fs_options())
        .map_err(|source| corrupt(image, format!("cannot mount FAT volume: {source}")))?;

    for dir in &injection.delta.dirs {
        fs.root_dir()
            .create_dir(dir.path.as_str())
            .map_err(|source| {
                corrupt(
                    image,
                    format!("cannot create {}: {source}", dir.path.to_dos_string()),
                )
            })?;
    }

    for entry in &injection.delta.files {
        let mut handle = fs
            .root_dir()
            .create_file(entry.path.as_str())
            .map_err(|source| {
                corrupt(
                    image,
                    format!("cannot open {}: {source}", entry.path.to_dos_string()),
                )
            })?;
        handle.truncate().map_err(|source| {
            corrupt(
                image,
                format!("cannot truncate {}: {source}", entry.path.to_dos_string()),
            )
        })?;
        handle.write_all(&entry.bytes).map_err(|source| {
            corrupt(
                image,
                format!("cannot write {}: {source}", entry.path.to_dos_string()),
            )
        })?;
    }

    fs.unmount()
        .map_err(|source| corrupt(image, format!("cannot unmount FAT volume: {source}")))?;
    Ok(())
}

/// Returns true when a startup batch file line runs the agent.
pub fn starts_agent(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    text.split('\n').any(|line| {
        let line = line.trim().to_ascii_uppercase();
        let mut tokens = line.split_ascii_whitespace();
        let first = tokens.next();
        let command = match first {
            Some("CALL") => tokens.next(),
            other => other,
        };
        command == Some(AGENT_PATH)
    })
}

fn reject_file(tree: &Tree, path: &DosPath, reason: &str) -> Result<(), ImageError> {
    match tree.get(path) {
        Some(TreeNode::File(_)) => Err(conflict(path, reason)),
        _ => Ok(()),
    }
}

fn reject_dir(tree: &Tree, path: &DosPath, reason: &str) -> Result<(), ImageError> {
    match tree.get(path) {
        Some(TreeNode::Dir(_)) => Err(conflict(path, reason)),
        _ => Ok(()),
    }
}

fn conflict(path: &DosPath, reason: &str) -> ImageError {
    ImageError::AgentInjection {
        reason: format!("{}: {reason}", path.to_dos_string()),
    }
}

fn corrupt(image: &Path, message: String) -> ImageError {
    ImageError::Corrupt {
        path: image.to_path_buf(),
        message,
    }
}
