//! Build-cache records and key computation.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use dosker_platform::fs::{ensure_dir_mode, write_atomic};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::canonical::hex;
use crate::error::ImageError;

/// The cache entry schema version.
pub const CACHE_SCHEMA_VERSION: u32 = 1;

/// Inputs to one cache key.
#[derive(Clone, Copy, Debug)]
pub struct CacheKeyInput<'a> {
    pub parent_tree_digest: &'a str,
    pub config_digest: &'a str,
    pub instruction: &'a str,
    pub input_hashes: &'a [String],
}

/// Computes the bare lowercase hex cache key.
pub fn cache_key(input: &CacheKeyInput<'_>) -> String {
    let mut hashes = input.input_hashes.to_vec();
    hashes.sort();
    let material = format!(
        "{}\n{}\n{}\n{}",
        input.parent_tree_digest,
        input.config_digest,
        input.instruction,
        hashes.join("\n")
    );
    hex(&Sha256::digest(material.as_bytes()))
}

/// One cache record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheRecord {
    pub schema_version: u32,
    pub key: String,
    pub layer_digest: String,
    pub instruction: String,
    pub created_at: DateTime<Utc>,
}

/// The local cache directory.
#[derive(Clone, Debug)]
pub struct CacheStore {
    root: PathBuf,
}

impl CacheStore {
    /// Creates a cache under `<root>/cache`.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Returns the entry path for one bare hex key.
    pub fn entry_path(&self, key: &str) -> PathBuf {
        self.root.join("cache").join(format!("{key}.json"))
    }

    /// Loads one record; a missing directory or entry is `None`.
    pub fn load(&self, key: &str) -> Result<Option<CacheRecord>, ImageError> {
        let path = self.entry_path(key);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(ImageError::Io { path, source }),
        };
        let record: CacheRecord =
            serde_json::from_slice(&bytes).map_err(|error| ImageError::Corrupt {
                path: path.clone(),
                message: error.to_string(),
            })?;
        if record.schema_version != CACHE_SCHEMA_VERSION || record.key != key {
            return Err(ImageError::Corrupt {
                path,
                message: "unsupported cache schema or key".to_owned(),
            });
        }
        Ok(Some(record))
    }

    /// Stores one record, creating the cache directory.
    pub fn store(&self, record: &CacheRecord) -> Result<(), ImageError> {
        let directory = self.root.join("cache");
        ensure_dir_mode(&directory, 0o700)?;
        let mut bytes = serde_json::to_vec_pretty(record).map_err(|error| ImageError::Corrupt {
            path: self.entry_path(&record.key),
            message: error.to_string(),
        })?;
        bytes.push(b'\n');
        write_atomic(&self.entry_path(&record.key), &bytes)?;
        Ok(())
    }

    /// Removes one record; a missing entry is success.
    pub fn remove(&self, key: &str) -> Result<(), ImageError> {
        match fs::remove_file(self.entry_path(key)) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(ImageError::Io {
                path: self.entry_path(key),
                source,
            }),
        }
    }

    /// Lists every valid record.
    pub fn list(&self) -> Result<Vec<CacheRecord>, ImageError> {
        let directory = self.root.join("cache");
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(ImageError::Io {
                    path: directory,
                    source,
                });
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| ImageError::Io {
                path: directory.clone(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(key) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if let Some(record) = self.load(key)? {
                records.push(record);
            }
        }
        records.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(records)
    }
}

/// Creates a record for tests and future callers.
pub fn record(
    key: String,
    layer_digest: String,
    instruction: String,
    created_at: DateTime<Utc>,
) -> CacheRecord {
    CacheRecord {
        schema_version: CACHE_SCHEMA_VERSION,
        key,
        layer_digest,
        instruction,
        created_at,
    }
}

#[allow(dead_code)]
fn _path_type_check(_: &Path) {}
