//! Digest-keyed license acceptance records.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use dosker_platform::fs::{ensure_dir_mode, write_atomic};
use serde::{Deserialize, Serialize};

use crate::error::ImageError;
use crate::reference::ImageDigest;

/// The acceptance record schema version.
pub const ACCEPTANCE_SCHEMA_VERSION: u32 = 1;

/// One accepted image digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedImage {
    pub schema_version: u32,
    pub digest: String,
    pub config_digest: String,
    pub image: String,
    pub license_summary: String,
    pub accepted_at: DateTime<Utc>,
}

/// The accepted digests under `<data-root>/accepted`.
#[derive(Clone, Debug)]
pub struct AcceptanceStore {
    root: PathBuf,
}

impl AcceptanceStore {
    /// Creates a store under `<root>/accepted`.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: root.join("accepted"),
        }
    }

    /// Returns the record path for one digest.
    pub fn path(&self, digest: &str) -> Result<PathBuf, ImageError> {
        let parsed = ImageDigest::parse(digest)?;
        let hex = parsed
            .as_str()
            .strip_prefix("sha256:")
            .expect("a sha256 digest");
        Ok(self.root.join(format!("{hex}.json")))
    }

    /// Returns whether the digest has an acceptance record.
    pub fn is_accepted(&self, digest: &str) -> Result<bool, ImageError> {
        let path = self.path(digest)?;
        match fs::read(&path) {
            Ok(_) => Ok(self.load_at(&path)?.digest == digest),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(ImageError::Io { path, source }),
        }
    }

    /// Records one acceptance.
    pub fn record(&self, record: &AcceptedImage) -> Result<(), ImageError> {
        ensure_dir_mode(&self.root, 0o700)?;
        let path = self.path(&record.digest)?;
        let mut bytes = serde_json::to_vec_pretty(record).map_err(|error| ImageError::Corrupt {
            path: path.clone(),
            message: error.to_string(),
        })?;
        bytes.push(b'\n');
        write_atomic(&path, &bytes)?;
        Ok(())
    }

    /// Lists every acceptance record, sorted by digest.
    pub fn list(&self) -> Result<Vec<AcceptedImage>, ImageError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(ImageError::Io {
                    path: self.root.clone(),
                    source,
                });
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| ImageError::Io {
                path: self.root.clone(),
                source,
            })?;
            if entry.path().extension().and_then(|value| value.to_str()) == Some("json") {
                records.push(self.load_at(&entry.path())?);
            }
        }
        records.sort_by(|left, right| left.digest.cmp(&right.digest));
        Ok(records)
    }

    fn load_at(&self, path: &Path) -> Result<AcceptedImage, ImageError> {
        let bytes = fs::read(path).map_err(|source| ImageError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let record: AcceptedImage =
            serde_json::from_slice(&bytes).map_err(|error| ImageError::Corrupt {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        if record.schema_version != ACCEPTANCE_SCHEMA_VERSION {
            return Err(ImageError::Corrupt {
                path: path.to_path_buf(),
                message: "unsupported acceptance schema".to_owned(),
            });
        }
        Ok(record)
    }
}
