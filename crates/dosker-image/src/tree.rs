//! The canonical DOS file tree and layer deltas.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::canonical::{canonical_json, sha256_digest};
use crate::error::ImageError;
use crate::path::DosPath;

/// DOS file attributes stored in a tree entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attributes {
    pub read_only: bool,
    pub hidden: bool,
    pub system: bool,
    pub archive: bool,
}

/// The kind of a tree entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeKind {
    File,
    Dir,
}

/// A file in the tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeFile {
    pub path: DosPath,
    pub attributes: Attributes,
    pub timestamp: DateTime<Utc>,
    pub bytes: Vec<u8>,
}

/// A directory in the tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeDir {
    pub path: DosPath,
    pub attributes: Attributes,
    pub timestamp: DateTime<Utc>,
}

/// One tree entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TreeNode {
    File(TreeFile),
    Dir(TreeDir),
}

impl TreeNode {
    /// Returns the entry kind.
    pub fn kind(&self) -> TreeKind {
        match self {
            Self::File(_) => TreeKind::File,
            Self::Dir(_) => TreeKind::Dir,
        }
    }

    /// Returns the entry attributes.
    pub fn attributes(&self) -> &Attributes {
        match self {
            Self::File(file) => &file.attributes,
            Self::Dir(dir) => &dir.attributes,
        }
    }

    /// Returns the entry timestamp.
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::File(file) => file.timestamp,
            Self::Dir(dir) => dir.timestamp,
        }
    }
}

/// The changes one layer applies to a tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerDelta {
    pub whiteouts: Vec<DosPath>,
    pub files: Vec<TreeFile>,
    pub dirs: Vec<TreeDir>,
}

impl LayerDelta {
    /// Creates an empty delta.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a whiteout.
    pub fn whiteout(&mut self, path: DosPath) {
        self.whiteouts.push(path);
    }

    /// Adds a file.
    pub fn add_file(&mut self, file: TreeFile) {
        self.files.push(file);
    }

    /// Adds a directory.
    pub fn add_dir(&mut self, dir: TreeDir) {
        self.dirs.push(dir);
    }
}

/// The canonical tree rooted at `C:\`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    entries: BTreeMap<DosPath, TreeNode>,
}

impl Tree {
    /// Creates an empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Looks up one path.
    pub fn get(&self, path: &DosPath) -> Option<&TreeNode> {
        self.entries.get(path)
    }

    /// Iterates over paths and nodes in path order.
    pub fn iter(&self) -> impl Iterator<Item = (&DosPath, &TreeNode)> {
        self.entries.iter()
    }

    /// Iterates over files in path order.
    pub fn files(&self) -> impl Iterator<Item = &TreeFile> {
        self.entries.values().filter_map(|node| match node {
            TreeNode::File(file) => Some(file),
            TreeNode::Dir(_) => None,
        })
    }

    /// Iterates over directories in path order.
    pub fn dirs(&self) -> impl Iterator<Item = &TreeDir> {
        self.entries.values().filter_map(|node| match node {
            TreeNode::Dir(dir) => Some(dir),
            TreeNode::File(_) => None,
        })
    }

    /// Returns true when the tree has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Converts the whole tree into a delta.
    pub fn to_delta(&self) -> LayerDelta {
        let mut delta = LayerDelta::new();
        delta.dirs = self.dirs().cloned().collect();
        delta.files = self.files().cloned().collect();
        delta
    }

    /// Applies whiteouts first, then directories and files.
    pub fn apply_delta(&mut self, delta: &LayerDelta) -> Result<(), ImageError> {
        for path in &delta.whiteouts {
            self.remove_subtree(path);
        }
        for dir in &delta.dirs {
            self.insert_dir(dir)?;
        }
        for file in &delta.files {
            self.insert_file(file)?;
        }
        Ok(())
    }

    /// Returns the canonical digest of the tree.
    pub fn digest(&self) -> Result<String, ImageError> {
        let entries: Vec<TreeDigestEntry> = self
            .entries
            .iter()
            .map(|(path, node)| TreeDigestEntry {
                path: path.as_str().to_owned(),
                kind: node.kind(),
                attributes: node.attributes().clone(),
                timestamp: truncate_seconds(node.timestamp()),
                size: match node {
                    TreeNode::File(file) => file.bytes.len() as u64,
                    TreeNode::Dir(_) => 0,
                },
                content: match node {
                    TreeNode::File(file) => Some(sha256_digest(&file.bytes)),
                    TreeNode::Dir(_) => None,
                },
            })
            .collect();
        let bytes = canonical_json(&entries)?;
        Ok(sha256_digest(&bytes))
    }

    fn insert_file(&mut self, file: &TreeFile) -> Result<(), ImageError> {
        self.reject_root(&file.path)?;
        self.ensure_parents(&file.path, file.timestamp)?;
        if matches!(self.entries.get(&file.path), Some(TreeNode::Dir(_))) {
            return Err(conflict(&file.path, "a file replaces a directory"));
        }
        self.entries
            .insert(file.path.clone(), TreeNode::File(file.clone()));
        Ok(())
    }

    fn insert_dir(&mut self, dir: &TreeDir) -> Result<(), ImageError> {
        self.reject_root(&dir.path)?;
        self.ensure_parents(&dir.path, dir.timestamp)?;
        if matches!(self.entries.get(&dir.path), Some(TreeNode::File(_))) {
            return Err(conflict(&dir.path, "a directory replaces a file"));
        }
        self.entries
            .insert(dir.path.clone(), TreeNode::Dir(dir.clone()));
        Ok(())
    }

    fn ensure_parents(
        &mut self,
        path: &DosPath,
        timestamp: DateTime<Utc>,
    ) -> Result<(), ImageError> {
        let mut parent = path.parent();
        let mut chain = Vec::new();
        while let Some(path) = parent {
            if path.is_root() {
                break;
            }
            chain.push(path.clone());
            parent = path.parent();
        }
        for path in chain.into_iter().rev() {
            match self.entries.get(&path) {
                Some(TreeNode::File(_)) => {
                    return Err(conflict(&path, "a file is used as a directory"));
                }
                Some(TreeNode::Dir(_)) => {}
                None => {
                    self.entries.insert(
                        path.clone(),
                        TreeNode::Dir(TreeDir {
                            path,
                            attributes: Attributes::default(),
                            timestamp,
                        }),
                    );
                }
            }
        }
        Ok(())
    }

    fn reject_root(&self, path: &DosPath) -> Result<(), ImageError> {
        if path.is_root() {
            return Err(conflict(path, "the tree root cannot be an entry"));
        }
        Ok(())
    }

    fn remove_subtree(&mut self, path: &DosPath) {
        if path.is_root() {
            self.entries.clear();
            return;
        }
        let prefix = format!("{}/", path.as_str());
        self.entries
            .retain(|candidate, _| candidate != path && !candidate.as_str().starts_with(&prefix));
    }
}

#[derive(Serialize)]
struct TreeDigestEntry {
    path: String,
    kind: TreeKind,
    attributes: Attributes,
    timestamp: DateTime<Utc>,
    size: u64,
    content: Option<String>,
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
