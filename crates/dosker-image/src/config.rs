//! The DOSker image config.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::boot::{BootLayout, BootMetadata};
use crate::canonical::canonical_digest;
use crate::error::ImageError;
use crate::path::DosPath;

/// The config format version.
pub const IMAGE_CONFIG_SCHEMA_VERSION: u32 = 1;

/// The DOS flavour recorded by the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DosFlavour {
    Freedos,
    Msdos,
    Drdos,
    Other,
    None,
}

/// Machine settings validated against an engine capability set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineProfile {
    pub cpu: String,
    pub memory_mb: u16,
    pub video: String,
    pub sound: String,
    pub net: String,
    pub serial_ports: u8,
    pub control_port: u8,
    pub disk_interface: String,
    pub baud: u32,
}

impl Default for MachineProfile {
    fn default() -> Self {
        Self {
            cpu: "486dx2".to_owned(),
            memory_mb: 16,
            video: "vga".to_owned(),
            sound: "sb16".to_owned(),
            net: "ne2000".to_owned(),
            serial_ports: 2,
            control_port: 2,
            disk_interface: "ide".to_owned(),
            baud: 115_200,
        }
    }
}

impl MachineProfile {
    /// Validates the machine values.
    pub fn validate(&self) -> Result<(), ImageError> {
        if !matches!(
            self.cpu.as_str(),
            "8086" | "386" | "486" | "486dx2" | "pentium"
        ) {
            return Err(invalid("unknown CPU"));
        }
        if !(1..=256).contains(&self.memory_mb) {
            return Err(invalid("memory must be 1 to 256 MiB"));
        }
        if !matches!(self.video.as_str(), "cga" | "ega" | "vga" | "svga") {
            return Err(invalid("unknown video mode"));
        }
        if !matches!(self.sound.as_str(), "none" | "sb16") {
            return Err(invalid("unknown sound device"));
        }
        if !matches!(self.net.as_str(), "none" | "ne2000") {
            return Err(invalid("unknown network device"));
        }
        if !(1..=4).contains(&self.serial_ports) {
            return Err(invalid("serial ports must be 1 to 4"));
        }
        if !(1..=4).contains(&self.control_port) || self.control_port > self.serial_ports {
            return Err(invalid("control port must be an enabled serial port"));
        }
        if !matches!(self.disk_interface.as_str(), "ide" | "floppy") {
            return Err(invalid("unknown disk interface"));
        }
        if !(9_600..=921_600).contains(&self.baud) {
            return Err(invalid("baud must be 9600 to 921600"));
        }
        Ok(())
    }
}

/// One history entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub empty_layer: bool,
    pub comment: Option<String>,
}

/// A protocol declared by an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Tcp,
}

/// One exposed guest port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortDeclaration {
    pub port: u16,
    pub protocol: PortProtocol,
}

/// One volume declaration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeDeclaration {
    pub drive: String,
}

/// The image config blob.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageConfig {
    pub schema_version: u32,
    pub dos_flavour: DosFlavour,
    pub booter: bool,
    pub boot: BootMetadata,
    pub machine: MachineProfile,
    pub env: BTreeMap<String, String>,
    pub workdir: String,
    pub cmd: Option<String>,
    pub exposed_ports: Vec<PortDeclaration>,
    pub volumes: Vec<VolumeDeclaration>,
    pub engine_pin: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub engine_config: BTreeMap<String, Vec<String>>,
    pub requires: Vec<String>,
    pub agent_version: Option<String>,
    pub created_by: String,
    pub history: Vec<HistoryEntry>,
}

impl ImageConfig {
    /// Validates the config fields.
    pub fn validate(&self) -> Result<(), ImageError> {
        if self.schema_version != IMAGE_CONFIG_SCHEMA_VERSION {
            return Err(invalid("unsupported config schema version"));
        }
        self.machine.validate()?;
        for (engine, lines) in &self.engine_config {
            if !valid_engine(engine) {
                return Err(invalid("unknown engine"));
            }
            if lines.iter().any(|line| line.trim().is_empty()) {
                return Err(invalid("engine config line must not be empty"));
            }
        }
        if self.booter {
            if self.boot.layout != BootLayout::Opaque {
                return Err(invalid("a booter image needs the opaque layout"));
            }
            if !self.env.is_empty()
                || !self.workdir.is_empty()
                || self.cmd.is_some()
                || !self.exposed_ports.is_empty()
                || !self.volumes.is_empty()
            {
                return Err(invalid("a booter image has no tree settings"));
            }
        } else {
            if self.boot.layout == BootLayout::Opaque {
                return Err(invalid("a tree image cannot use the opaque layout"));
            }
            self.boot.validate()?;
            let workdir = DosPath::parse(&self.workdir)?;
            if !workdir.is_root() && !workdir.to_dos_string().starts_with(r"C:\") {
                return Err(invalid("workdir must be on C:"));
            }
            let expected = match self.boot.layout {
                BootLayout::MbrPartitioned => "ide",
                BootLayout::Superfloppy => "floppy",
                BootLayout::Opaque => unreachable!("opaque was rejected above"),
            };
            if self.machine.disk_interface != expected {
                return Err(invalid("disk interface does not match the boot layout"));
            }
        }
        for port in &self.exposed_ports {
            if port.port == 0 {
                return Err(invalid("port zero is invalid"));
            }
        }
        for volume in &self.volumes {
            if !valid_drive(&volume.drive) {
                return Err(invalid("volume drive must be one letter and a colon"));
            }
        }
        Ok(())
    }

    /// Returns the canonical config digest.
    pub fn digest(&self) -> Result<String, ImageError> {
        config_digest(self)
    }
}

/// Returns the canonical config digest.
pub fn config_digest(config: &ImageConfig) -> Result<String, ImageError> {
    canonical_digest(config)
}

fn valid_engine(value: &str) -> bool {
    matches!(value, "qemu" | "dosbox-x" | "bochs")
}

fn valid_drive(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_uppercase() && bytes[1] == b':'
}

fn invalid(reason: &str) -> ImageError {
    ImageError::InvalidImageConfig {
        reason: reason.to_owned(),
    }
}
