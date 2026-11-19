//! Reading, validating, and writing a host directory as a DOS tree.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::ImageError;
use crate::layer::fixed_timestamp;
use crate::path::{DosName, DosPath};
use crate::tree::{Attributes, LayerDelta, Tree, TreeDir, TreeFile};

/// Checks that a host directory can be mounted as a DOS drive.
///
/// The check never reads file contents.
pub fn validate_host_tree(root: &Path) -> Result<(), ImageError> {
    walk(root, false).map(|_| ())
}

/// Reads a host directory into a tree rooted at `C:\`.
pub fn read_host_tree(root: &Path) -> Result<Tree, ImageError> {
    walk(root, true)
}

/// Mirrors `tree` into a host directory rooted at `root`.
///
/// Every entry of the destination that the tree does not contain is
/// removed. Entries are replaced without following symlinks, and file
/// attributes and timestamps are left to the host defaults.
pub fn write_host_tree(root: &Path, tree: &Tree) -> Result<(), ImageError> {
    let expected = expected_paths(tree);
    remove_unexpected(root, root, &expected)?;

    for dir in tree.dirs() {
        let path = root.join(relative_path(&dir.path));
        if let Some(metadata) = symlink_metadata_if_exists(&path)?
            && !metadata.is_dir()
        {
            remove_entry(&path, &metadata)?;
        }
        fs::create_dir_all(&path).map_err(|source| io_error(&path, source))?;
    }

    for file in tree.files() {
        let path = root.join(relative_path(&file.path));
        if let Some(metadata) = symlink_metadata_if_exists(&path)? {
            remove_entry(&path, &metadata)?;
        }
        fs::write(&path, &file.bytes).map_err(|source| io_error(&path, source))?;
    }
    Ok(())
}

fn expected_paths(tree: &Tree) -> BTreeSet<PathBuf> {
    let mut expected = BTreeSet::new();
    for path in tree
        .dirs()
        .map(|dir| &dir.path)
        .chain(tree.files().map(|file| &file.path))
    {
        let mut relative = PathBuf::new();
        for component in path.components() {
            relative.push(component);
            expected.insert(relative.clone());
        }
    }
    expected
}

fn remove_unexpected(
    dir: &Path,
    root: &Path,
    expected: &BTreeSet<PathBuf>,
) -> Result<(), ImageError> {
    for entry in read_dir_sorted(dir)? {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| io_error(&path, source))?;
        let relative = path
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.clone());
        if expected.contains(&relative) {
            if metadata.is_dir() {
                remove_unexpected(&path, root, expected)?;
            }
        } else {
            remove_entry(&path, &metadata)?;
        }
    }
    Ok(())
}

fn relative_path(path: &DosPath) -> PathBuf {
    path.components().collect()
}

fn symlink_metadata_if_exists(path: &Path) -> Result<Option<fs::Metadata>, ImageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error(path, source)),
    }
}

fn remove_entry(path: &Path, metadata: &fs::Metadata) -> Result<(), ImageError> {
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|source| io_error(path, source))
}

fn walk(root: &Path, read_contents: bool) -> Result<Tree, ImageError> {
    let metadata = fs::symlink_metadata(root).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            invalid_host_path(root, "not found")
        } else {
            ImageError::Io {
                path: root.to_path_buf(),
                source,
            }
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(invalid_host_path(root, "a symlink is not allowed"));
    }
    if !metadata.is_dir() {
        return Err(invalid_host_path(root, "not a directory"));
    }

    let mut delta = LayerDelta::new();
    let mut hosts = BTreeMap::new();
    let prefix = DosPath::parse_relative("")?;
    walk_entries(root, &prefix, read_contents, &mut delta, &mut hosts)?;

    let mut tree = Tree::new();
    tree.apply_delta(&delta)?;
    Ok(tree)
}

fn walk_entries(
    dir: &Path,
    prefix: &DosPath,
    read_contents: bool,
    delta: &mut LayerDelta,
    hosts: &mut BTreeMap<DosPath, PathBuf>,
) -> Result<(), ImageError> {
    for entry in read_dir_sorted(dir)? {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| ImageError::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_symlink() {
            return Err(invalid_host_path(&path, "a symlink is not allowed"));
        }

        let file_name = entry.file_name();
        let file_name = file_name
            .to_str()
            .ok_or_else(|| invalid_host_path(&path, "the name is not valid UTF-8"))?;
        let name = DosName::normalize(file_name)
            .map_err(|error| invalid_host_path(&path, &error.to_string()))?;
        let dos_path = prefix.join(&name);
        if let Some(first) = hosts.get(&dos_path) {
            return Err(invalid_host_path(
                &path,
                &format!("duplicate of {}", first.display()),
            ));
        }
        hosts.insert(dos_path.clone(), path.clone());

        if file_type.is_dir() {
            delta.add_dir(TreeDir {
                path: dos_path.clone(),
                attributes: Attributes::default(),
                timestamp: fixed_timestamp(),
            });
            walk_entries(&path, &dos_path, read_contents, delta, hosts)?;
        } else if file_type.is_file() {
            let bytes = if read_contents {
                fs::read(&path).map_err(|source| ImageError::Io {
                    path: path.clone(),
                    source,
                })?
            } else {
                Vec::new()
            };
            delta.add_file(TreeFile {
                path: dos_path,
                attributes: Attributes::default(),
                timestamp: fixed_timestamp(),
                bytes,
            });
        } else {
            return Err(invalid_host_path(&path, "not a regular file or directory"));
        }
    }
    Ok(())
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<fs::DirEntry>, ImageError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|source| ImageError::Io {
        path: dir.to_path_buf(),
        source,
    })? {
        entries.push(entry.map_err(|source| ImageError::Io {
            path: dir.to_path_buf(),
            source,
        })?);
    }
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn io_error(path: &Path, source: io::Error) -> ImageError {
    ImageError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid_host_path(path: &Path, reason: &str) -> ImageError {
    ImageError::InvalidHostPath {
        path: path.to_path_buf(),
        reason: reason.to_owned(),
    }
}
