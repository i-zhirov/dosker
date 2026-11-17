//! Pinned FreeDOS inputs and the files loaded from them.

use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::canonical::sha256_digest;
use crate::error::ImageError;
use crate::fat;

/// The only supported FreeDOS release.
pub const FREEDOS_VERSION: &str = "1.3";

/// The pinned `KERNEL.SYS` digest.
pub const FREEDOS_KERNEL_SHA256: &str =
    "sha256:f34a7483c575fcf2709d9a7d0bc3db81c6211c279530f9e1bf78576b9233924d";
/// The pinned `COMMAND.COM` digest.
pub const FREEDOS_COMMAND_SHA256: &str =
    "sha256:93ed257d3aa2454821aa305b731544550dcccff898865b590deb972c47b48610";
/// The pinned agent executable digest.
pub const FREEDOS_AGENT_SHA256: &str =
    "sha256:590fcc601e8ad9e85ebd8b3b4575c67460b51f46e0bcf7d89f6bff57f9f05591";
/// The pinned MBR template digest.
pub const FREEDOS_MBR_SHA256: &str =
    "sha256:d2cc0119c6d3caec354658eb822d3b0dfb69884a521d7c8aa28d59cacdc638ff";
/// The pinned boot-sector template digest.
pub const FREEDOS_BOOT_SECTOR_SHA256: &str =
    "sha256:b27479134bbd40f1aa45a12e8540a1d1e9cd1fd73cea8bec9d942372132256b7";

/// The committed MBR template.
pub const MBR_TEMPLATE: &[u8] = include_bytes!("../../../../agent/fixtures/base/mbr.bin");
/// The committed FreeDOS boot-sector template.
pub const BOOT_SECTOR_TEMPLATE: &[u8] =
    include_bytes!("../../../../agent/fixtures/base/boot-sector.bin");

/// The FreeDOS release requested by a `FROM` instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreedosVersion(String);

impl FreedosVersion {
    /// The releases this build supports.
    pub const SUPPORTED: &'static [&'static str] = &[FREEDOS_VERSION];

    /// Parses a release, accepting only the pinned value.
    pub fn parse(value: &str) -> Result<Self, ImageError> {
        if Self::SUPPORTED.contains(&value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(ImageError::UnsupportedFreedosVersion {
                version: value.to_owned(),
                supported: FREEDOS_VERSION.to_owned(),
            })
        }
    }

    /// Returns the release tag.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The digests that pin the FreeDOS and agent inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreedosPins {
    pub kernel_sha256: String,
    pub command_sha256: String,
    pub agent_sha256: String,
}

impl FreedosPins {
    /// The pins for FreeDOS 1.3.
    pub fn freedos_1_3() -> Self {
        Self {
            kernel_sha256: FREEDOS_KERNEL_SHA256.to_owned(),
            command_sha256: FREEDOS_COMMAND_SHA256.to_owned(),
            agent_sha256: FREEDOS_AGENT_SHA256.to_owned(),
        }
    }
}

impl Default for FreedosPins {
    fn default() -> Self {
        Self::freedos_1_3()
    }
}

/// The host paths the FreeDOS base is built from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseInputs {
    pub source: PathBuf,
    pub agent_executable: PathBuf,
    pub pins: FreedosPins,
}

impl BaseInputs {
    /// Reads the source and agent paths from the environment.
    pub fn from_environment(_version: &FreedosVersion) -> Result<Self, ImageError> {
        let source = env::var_os("DOSKER_FREEDOS_SOURCE")
            .map(PathBuf::from)
            .ok_or(ImageError::FreedosSourceNotSet)?;
        Ok(Self {
            source,
            agent_executable: default_agent_executable(),
            pins: FreedosPins::default(),
        })
    }
}

/// Returns the agent executable path, preferring `DOSKER_AGENT_EXE`.
pub fn default_agent_executable() -> PathBuf {
    match env::var_os("DOSKER_AGENT_EXE") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agent/build/DOSKER.EXE"),
    }
}

/// Reads the default agent executable without checking its pin.
pub fn load_agent_executable() -> Result<Vec<u8>, ImageError> {
    read_agent(&default_agent_executable())
}

/// The bytes loaded from the pinned FreeDOS inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreedosBinaries {
    pub kernel: Vec<u8>,
    pub command: Vec<u8>,
    pub agent: Vec<u8>,
    pub mbr: Vec<u8>,
    pub boot_sector: Vec<u8>,
}

/// Loads and verifies every FreeDOS input.
pub fn load_freedos_binaries(inputs: &BaseInputs) -> Result<FreedosBinaries, ImageError> {
    if !inputs.source.is_file() {
        return Err(ImageError::FreedosSourceMissing {
            path: inputs.source.clone(),
        });
    }

    let kernel = read_required(&inputs.source, "\\KERNEL.SYS", "KERNEL.SYS")?;
    let command = match fat::read_file_if_present(&inputs.source, "\\FREEDOS\\BIN\\COMMAND.COM")? {
        Some(bytes) => bytes,
        None => read_required(&inputs.source, "\\COMMAND.COM", "COMMAND.COM")?,
    };
    let agent = read_agent(&inputs.agent_executable)?;

    verify_pin("KERNEL.SYS", &inputs.pins.kernel_sha256, &kernel)?;
    verify_pin("COMMAND.COM", &inputs.pins.command_sha256, &command)?;
    verify_pin("DOSKER.EXE", &inputs.pins.agent_sha256, &agent)?;

    Ok(FreedosBinaries {
        kernel,
        command,
        agent,
        mbr: MBR_TEMPLATE.to_vec(),
        boot_sector: BOOT_SECTOR_TEMPLATE.to_vec(),
    })
}

fn read_required(image: &Path, dos_path: &str, file: &str) -> Result<Vec<u8>, ImageError> {
    fat::read_file_if_present(image, dos_path)?.ok_or_else(|| ImageError::FreedosFileMissing {
        path: image.to_path_buf(),
        file: file.to_owned(),
    })
}

fn read_agent(path: &Path) -> Result<Vec<u8>, ImageError> {
    fs::read(path).map_err(|source| {
        if source.kind() == ErrorKind::NotFound {
            ImageError::FreedosFileMissing {
                path: path.to_path_buf(),
                file: "DOSKER.EXE".to_owned(),
            }
        } else {
            ImageError::Io {
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

fn verify_pin(file: &str, expected: &str, bytes: &[u8]) -> Result<(), ImageError> {
    let actual = sha256_digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(ImageError::FreedosPinMismatch {
            file: file.to_owned(),
            expected: expected.to_owned(),
            actual,
        })
    }
}
