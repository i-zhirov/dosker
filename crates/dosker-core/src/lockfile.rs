//! The `Doskerfile.lock` model.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The conventional file name of a lock file.
pub const LOCK_FILE_NAME: &str = "Doskerfile.lock";

/// The lock file format version.
pub const LOCK_VERSION: u32 = 1;

/// A resolved base image pin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BasePin {
    /// The image name.
    pub name: String,
    /// The image tag, empty for a digest reference.
    pub tag: String,
    /// The resolved manifest digest.
    pub digest: String,
}

/// A resolved engine pin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnginePin {
    /// The reported engine version.
    pub version: String,
    /// How the engine binary was found.
    pub source: String,
}

/// A resolved agent pin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPin {
    /// The agent version recorded by the image config.
    pub version: String,
}

/// The resolved pins recorded for a build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockFile {
    /// The lock file format version.
    pub version: u32,
    /// The resolved base image pins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base: Vec<BasePin>,
    /// The resolved engine pins, keyed by engine name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub engines: BTreeMap<String, EnginePin>,
    /// The resolved agent pin, absent when the base has no agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentPin>,
}

impl LockFile {
    /// Returns an empty lock file for the current format version.
    pub fn new() -> Self {
        Self {
            version: LOCK_VERSION,
            base: Vec::new(),
            engines: BTreeMap::new(),
            agent: None,
        }
    }

    /// Parses a lock file, rejecting unknown fields and unsupported versions.
    pub fn from_toml(input: &str) -> Result<Self, LockFileError> {
        let probe: VersionProbe = toml::from_str(input).map_err(invalid_de)?;
        if probe.version != LOCK_VERSION {
            return Err(LockFileError::UnsupportedVersion(probe.version));
        }
        toml::from_str(input).map_err(invalid_de)
    }

    /// Serializes the lock file as TOML with empty sections omitted.
    pub fn to_toml(&self) -> Result<String, LockFileError> {
        if self.version != LOCK_VERSION {
            return Err(LockFileError::Invalid {
                reason: format!("unsupported lock file version: {}", self.version),
            });
        }
        toml::to_string(self).map_err(invalid_ser)
    }
}

fn invalid_de(error: toml::de::Error) -> LockFileError {
    LockFileError::Invalid {
        reason: error.to_string(),
    }
}

fn invalid_ser(error: toml::ser::Error) -> LockFileError {
    LockFileError::Invalid {
        reason: error.to_string(),
    }
}

/// Reads only the format version before the strict parse, so an unsupported
/// version is reported even when the file uses fields this model does not know.
#[derive(Deserialize)]
struct VersionProbe {
    version: u32,
}

impl Default for LockFile {
    fn default() -> Self {
        Self::new()
    }
}

/// A failure while reading or writing a lock file.
#[derive(Debug, thiserror::Error)]
pub enum LockFileError {
    /// The lock file is not valid TOML or does not match the model.
    #[error("invalid lock file: {reason}")]
    Invalid { reason: String },
    /// The lock file declares a format version this build cannot read.
    #[error("unsupported lock file version: {0}")]
    UnsupportedVersion(u32),
}
