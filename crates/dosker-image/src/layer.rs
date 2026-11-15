//! Canonical layer tar codec and parent rule.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use tar::{Archive, Builder, EntryType, Header};

use crate::boot::BootMetadata;
use crate::canonical::{canonical_json, sha256_digest};
use crate::config::ImageConfig;
use crate::error::ImageError;
use crate::path::{DosPath, normalize_tree_member};
use crate::tree::{Attributes, LayerDelta, Tree, TreeDir, TreeFile, TreeKind};

/// The layer sidecar schema version.
pub const LAYER_SCHEMA_VERSION: u32 = 1;
/// The sidecar member name.
pub const LAYER_META_FILE: &str = "dosker-layer.json";
/// The prefix for file members.
pub const LAYER_FILES_PREFIX: &str = "files/";
/// The fixed timestamp for generated layers.
pub const FIXED_TIMESTAMP_SECONDS: i64 = 315_532_800;

/// Returns `1980-01-01T00:00:00Z`.
pub fn fixed_timestamp() -> DateTime<Utc> {
    Utc.timestamp_opt(FIXED_TIMESTAMP_SECONDS, 0)
        .single()
        .expect("the fixed timestamp is valid")
}

/// One entry in the sidecar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerEntry {
    pub path: DosPath,
    pub kind: TreeKind,
    pub attributes: Attributes,
    pub timestamp: DateTime<Utc>,
    pub size: u64,
    pub content: Option<String>,
}

/// The layer sidecar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerMeta {
    pub schema_version: u32,
    pub parent: Option<String>,
    pub created_by: String,
    pub whiteouts: Vec<DosPath>,
    pub entries: Vec<LayerEntry>,
    pub boot: Option<BootMetadata>,
}

/// One canonical layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub meta: LayerMeta,
    files: BTreeMap<DosPath, Vec<u8>>,
}

impl Layer {
    /// Builds a layer from a delta.
    pub fn from_delta(
        delta: &LayerDelta,
        created_by: &str,
        parent: Option<String>,
        boot: Option<BootMetadata>,
    ) -> Result<Self, ImageError> {
        let mut whiteouts = delta.whiteouts.clone();
        whiteouts.sort();
        let mut files = delta.files.clone();
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let mut dirs = delta.dirs.clone();
        dirs.sort_by(|left, right| left.path.cmp(&right.path));

        let mut entries = Vec::with_capacity(files.len() + dirs.len());
        let mut contents = BTreeMap::new();
        for dir in dirs {
            entries.push(LayerEntry {
                path: dir.path,
                kind: TreeKind::Dir,
                attributes: dir.attributes,
                timestamp: truncate_seconds(dir.timestamp),
                size: 0,
                content: None,
            });
        }
        for file in files {
            let path = file.path.clone();
            entries.push(LayerEntry {
                path: file.path.clone(),
                kind: TreeKind::File,
                attributes: file.attributes,
                timestamp: truncate_seconds(file.timestamp),
                size: file.bytes.len() as u64,
                content: Some(file.path.to_tar_member()),
            });
            if contents.insert(path, file.bytes).is_some() {
                return Err(conflict(&file.path, "duplicate file"));
            }
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        for pair in entries.windows(2) {
            if pair[0].path == pair[1].path {
                return Err(conflict(&pair[0].path, "duplicate layer entry"));
            }
        }
        Ok(Self {
            meta: LayerMeta {
                schema_version: LAYER_SCHEMA_VERSION,
                parent,
                created_by: created_by.to_owned(),
                whiteouts,
                entries,
                boot,
            },
            files: contents,
        })
    }

    /// Converts the layer back to a delta.
    pub fn to_delta(&self) -> LayerDelta {
        let mut delta = LayerDelta::new();
        delta.whiteouts = self.meta.whiteouts.clone();
        for entry in &self.meta.entries {
            match entry.kind {
                TreeKind::Dir => delta.dirs.push(TreeDir {
                    path: entry.path.clone(),
                    attributes: entry.attributes.clone(),
                    timestamp: entry.timestamp,
                }),
                TreeKind::File => delta.files.push(TreeFile {
                    path: entry.path.clone(),
                    attributes: entry.attributes.clone(),
                    timestamp: entry.timestamp,
                    bytes: self.files.get(&entry.path).cloned().unwrap_or_default(),
                }),
            }
        }
        delta
    }

    /// Applies the layer to a tree.
    pub fn apply_to(&self, tree: &mut Tree) -> Result<(), ImageError> {
        tree.apply_delta(&self.to_delta())
    }

    /// Writes the canonical tar.
    pub fn write_tar(&self) -> Result<Vec<u8>, ImageError> {
        let mut builder = Builder::new(Vec::new());
        let sidecar = self.meta_json()?;
        append_member(
            &mut builder,
            LAYER_META_FILE,
            &sidecar,
            fixed_timestamp(),
            false,
        )?;
        for entry in &self.meta.entries {
            let path = match entry.kind {
                TreeKind::Dir => format!("{}/", entry.path.to_tar_member()),
                TreeKind::File => entry.path.to_tar_member(),
            };
            match entry.kind {
                TreeKind::Dir => append_member(&mut builder, &path, &[], entry.timestamp, true)?,
                TreeKind::File => append_member(
                    &mut builder,
                    &path,
                    self.files
                        .get(&entry.path)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                    entry.timestamp,
                    false,
                )?,
            }
        }
        builder
            .into_inner()
            .map_err(|error| corrupt("layer tar", error.to_string()))
    }

    /// Reads a canonical tar.
    pub fn read_tar(bytes: &[u8]) -> Result<Self, ImageError> {
        let mut archive = Archive::new(Cursor::new(bytes));
        let mut sidecar: Option<Vec<u8>> = None;
        let mut members: BTreeMap<DosPath, (TreeKind, Vec<u8>)> = BTreeMap::new();
        let entries = archive
            .entries()
            .map_err(|error| corrupt("layer tar", error.to_string()))?;
        for entry in entries {
            let mut entry = entry.map_err(|error| corrupt("layer tar", error.to_string()))?;
            let path = entry
                .path()
                .map_err(|error| corrupt("layer tar", error.to_string()))?;
            let path_text = path.to_string_lossy();
            if path_text == LAYER_META_FILE {
                if sidecar.is_some() {
                    return Err(corrupt("layer tar", "duplicate sidecar"));
                }
                let mut data = Vec::new();
                entry
                    .read_to_end(&mut data)
                    .map_err(|error| corrupt("layer tar", error.to_string()))?;
                sidecar = Some(data);
                continue;
            }
            let path = normalize_tree_member(&path_text)?;
            let file_type = entry.header().entry_type();
            let (kind, data) = if file_type.is_file() {
                let mut data = Vec::new();
                entry
                    .read_to_end(&mut data)
                    .map_err(|error| corrupt("layer tar", error.to_string()))?;
                (TreeKind::File, data)
            } else if file_type.is_dir() {
                (TreeKind::Dir, Vec::new())
            } else {
                return Err(ImageError::UnsafeLayerEntry {
                    path: path_text.into_owned(),
                    reason: "only file and directory members are allowed".to_owned(),
                });
            };
            if members.insert(path.clone(), (kind, data)).is_some() {
                return Err(corrupt("layer tar", "duplicate member"));
            }
        }

        let sidecar = sidecar.ok_or(ImageError::MissingLayerSidecar)?;
        let mut meta: LayerMeta = serde_json::from_slice(&sidecar)
            .map_err(|error| corrupt("layer sidecar", error.to_string()))?;
        if meta.schema_version != LAYER_SCHEMA_VERSION {
            return Err(corrupt("layer sidecar", "unsupported schema version"));
        }
        meta.entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        meta.whiteouts.sort();
        let mut files = BTreeMap::new();
        for entry in &meta.entries {
            let Some((kind, data)) = members.remove(&entry.path) else {
                return Err(corrupt("layer sidecar", "entry has no member"));
            };
            if kind != entry.kind {
                return Err(corrupt(
                    "layer sidecar",
                    "entry kind differs from the member",
                ));
            }
            match entry.kind {
                TreeKind::File => {
                    if entry.content.is_none() || entry.size != data.len() as u64 {
                        return Err(corrupt("layer sidecar", "file size or content is wrong"));
                    }
                    files.insert(entry.path.clone(), data);
                }
                TreeKind::Dir => {
                    if entry.content.is_some() || !data.is_empty() || entry.size != 0 {
                        return Err(corrupt("layer sidecar", "directory entry has content"));
                    }
                }
            }
        }
        if !members.is_empty() {
            return Err(corrupt(
                "layer sidecar",
                "member set differs from the sidecar",
            ));
        }
        Ok(Self { meta, files })
    }

    /// Returns the canonical tar digest.
    pub fn digest(&self) -> Result<String, ImageError> {
        self.write_tar().map(|bytes| sha256_digest(&bytes))
    }

    /// Checks the parent link.
    pub fn validate_parent(&self, previous: Option<&str>) -> Result<(), ImageError> {
        match (previous, self.meta.parent.as_deref()) {
            (None, None) => Ok(()),
            (Some(previous), Some(parent)) if previous == parent => Ok(()),
            _ => Err(corrupt(
                "layer parent",
                "parent does not match the previous layer",
            )),
        }
    }

    /// Returns canonical sidecar JSON.
    pub fn meta_json(&self) -> Result<Vec<u8>, ImageError> {
        canonical_json(&self.meta)
    }
}

/// Returns the effective boot metadata.
pub fn effective_boot(config: &ImageConfig, layers: &[Layer]) -> BootMetadata {
    layers
        .iter()
        .rev()
        .find_map(|layer| layer.meta.boot.clone())
        .unwrap_or_else(|| config.boot.clone())
}

fn append_member(
    builder: &mut Builder<Vec<u8>>,
    path: &str,
    data: &[u8],
    timestamp: DateTime<Utc>,
    directory: bool,
) -> Result<(), ImageError> {
    let mut header = Header::new_gnu();
    header.set_mode(if directory { 0o755 } else { 0o644 });
    header.set_uid(0);
    header.set_gid(0);
    header
        .set_username("")
        .map_err(|error| corrupt(path, error.to_string()))?;
    header
        .set_groupname("")
        .map_err(|error| corrupt(path, error.to_string()))?;
    header.set_mtime(timestamp.timestamp().max(0) as u64);
    header.set_size(data.len() as u64);
    header.set_entry_type(if directory {
        EntryType::Directory
    } else {
        EntryType::Regular
    });
    builder
        .append_data(&mut header, path, data)
        .map_err(|error| corrupt(path, error.to_string()))?;
    Ok(())
}

fn truncate_seconds(value: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp(value.timestamp(), 0).unwrap_or(value)
}

fn conflict(path: &DosPath, reason: &str) -> ImageError {
    ImageError::LayerConflict {
        path: path.to_dos_string(),
        reason: reason.to_owned(),
    }
}

fn corrupt(path: &str, message: impl Into<String>) -> ImageError {
    ImageError::LayerCorrupt {
        path: path.to_owned(),
        message: message.into(),
    }
}
