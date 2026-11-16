//! The build context: source resolution, ignore matching, and hashes.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use dosker_core::{DOSKERFILE_NAME, LOCK_FILE_NAME};
use dosker_image::{canonical_json, sha256_digest};
use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use sha2::{Digest, Sha256};

use crate::error::BuildError;

const DOSKERIGNORE_NAME: &str = ".doskerignore";

/// A directory that contains a Doskerfile and its build inputs.
#[derive(Debug)]
pub struct BuildContext {
    root: PathBuf,
    ignore: Gitignore,
    digest: String,
}

/// A source path resolved inside the build context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSource {
    /// The path on the host.
    pub host_path: PathBuf,
    /// The context-relative path with `/` separators.
    pub relative: String,
    /// Whether the source is a file or a directory.
    pub kind: SourceKind,
}

/// The kind of a resolved source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// A regular file.
    File,
    /// A directory whose contents are the source.
    Directory,
}

/// One file read from a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    /// The context-relative path with `/` separators.
    pub relative: String,
    /// The file contents.
    pub bytes: Vec<u8>,
}

/// One entry read from a source directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceEntry {
    /// A regular file.
    File(SourceFile),
    /// A directory below the source root, as a context-relative path.
    Directory(String),
}

impl SourceEntry {
    /// Returns the context-relative path with `/` separators.
    pub fn relative(&self) -> &str {
        match self {
            Self::File(file) => &file.relative,
            Self::Directory(relative) => relative,
        }
    }
}

impl BuildContext {
    /// Opens the context rooted at `root` and computes its digest.
    pub fn open(root: &Path) -> Result<Self, BuildError> {
        let root = std::path::absolute(root).map_err(|source| BuildError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let metadata = fs::metadata(&root).map_err(|source| BuildError::Io {
            path: root.clone(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(BuildError::Context {
                reason: format!("build context is not a directory: {}", root.display()),
            });
        }
        let ignore = build_ignore(&root)?;
        let digest = compute_digest(&root, &ignore)?;
        Ok(Self {
            root,
            ignore,
            digest,
        })
    }

    /// Returns the context root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the context digest.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Returns the Doskerfile path.
    pub fn doskerfile_path(&self) -> PathBuf {
        self.root.join(DOSKERFILE_NAME)
    }

    /// Returns the lock file path.
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(LOCK_FILE_NAME)
    }

    /// Resolves a host source path inside the context.
    pub fn resolve_source(&self, source: &str) -> Result<ResolvedSource, BuildError> {
        if source.is_empty() {
            return Err(context_error("source path is empty"));
        }
        if has_glob_character(source) {
            return Err(context_error(format!(
                "source path contains a glob character: {source}"
            )));
        }

        let raw = Path::new(source);
        let relative_input = if raw.is_absolute() {
            raw.strip_prefix(&self.root)
                .map_err(|_| {
                    context_error(format!("source is outside the build context: {source}"))
                })?
                .to_path_buf()
        } else {
            raw.to_path_buf()
        };

        let mut normalized = PathBuf::new();
        for component in relative_input.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(context_error(format!(
                        "source path may not contain '..': {source}"
                    )));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(context_error(format!(
                        "source is outside the build context: {source}"
                    )));
                }
            }
        }

        self.reject_symlink_components(&normalized, source)?;
        let host_path = if normalized.as_os_str().is_empty() {
            self.root.clone()
        } else {
            self.root.join(&normalized)
        };
        let metadata = fs::metadata(&host_path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => context_error(format!("source not found: {source}")),
            _ => BuildError::Io {
                path: host_path.clone(),
                source: error,
            },
        })?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(context_error(format!(
                "source is not a file or directory: {source}"
            )));
        }
        let is_dir = metadata.is_dir();
        if !normalized.as_os_str().is_empty() && is_ignored(&self.ignore, &normalized, is_dir) {
            return Err(context_error(format!(
                "source is excluded by .doskerignore: {source}"
            )));
        }

        Ok(ResolvedSource {
            host_path,
            relative: slash_path(&normalized),
            kind: if is_dir {
                SourceKind::Directory
            } else {
                SourceKind::File
            },
        })
    }

    /// Reads every file selected by a resolved source.
    pub fn read_source_files(
        &self,
        source: &ResolvedSource,
    ) -> Result<Vec<SourceFile>, BuildError> {
        match source.kind {
            SourceKind::File => {
                let metadata =
                    fs::symlink_metadata(&source.host_path).map_err(|error| BuildError::Io {
                        path: source.host_path.clone(),
                        source: error,
                    })?;
                if metadata.file_type().is_symlink() {
                    return Err(context_error(format!(
                        "source path contains a symlink: {}",
                        source.relative
                    )));
                }
                let bytes = fs::read(&source.host_path).map_err(|error| BuildError::Io {
                    path: source.host_path.clone(),
                    source: error,
                })?;
                Ok(vec![SourceFile {
                    relative: source.relative.clone(),
                    bytes,
                }])
            }
            SourceKind::Directory => Ok(self
                .read_source_entries(source)?
                .into_iter()
                .filter_map(|entry| match entry {
                    SourceEntry::File(file) => Some(file),
                    SourceEntry::Directory(_) => None,
                })
                .collect()),
        }
    }

    /// Reads every file and directory selected by a resolved source.
    pub fn read_source_entries(
        &self,
        source: &ResolvedSource,
    ) -> Result<Vec<SourceEntry>, BuildError> {
        match source.kind {
            SourceKind::File => Ok(self
                .read_source_files(source)?
                .into_iter()
                .map(SourceEntry::File)
                .collect()),
            SourceKind::Directory => {
                let mut entries = Vec::new();
                let at_root = source.host_path == self.root;
                self.collect_source_entries(&source.host_path, at_root, &mut entries)?;
                entries.sort_by(|left, right| left.relative().cmp(right.relative()));
                Ok(entries)
            }
        }
    }

    /// Returns the context digest followed by the source file and directory hashes.
    pub fn source_hashes(&self, sources: &[ResolvedSource]) -> Result<Vec<String>, BuildError> {
        let mut hashes = vec![format!("context:{}", self.digest)];
        for source in sources {
            if source.kind == SourceKind::Directory {
                hashes.push(format!("dir:{}", source.relative));
            }
            for entry in self.read_source_entries(source)? {
                match entry {
                    SourceEntry::Directory(relative) => hashes.push(format!("dir:{relative}")),
                    SourceEntry::File(file) => hashes.push(format!(
                        "file:{}:{}",
                        file.relative,
                        sha256_hex(&file.bytes)
                    )),
                }
            }
        }
        Ok(hashes)
    }

    fn reject_symlink_components(&self, relative: &Path, source: &str) -> Result<(), BuildError> {
        let mut current = self.root.clone();
        for component in relative.components() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(context_error(format!(
                        "source path contains a symlink: {source}"
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(context_error(format!("source not found: {source}")));
                }
                Err(error) => {
                    return Err(BuildError::Io {
                        path: current,
                        source: error,
                    });
                }
            }
        }
        Ok(())
    }

    fn collect_source_entries(
        &self,
        dir: &Path,
        skip_root_metadata: bool,
        entries: &mut Vec<SourceEntry>,
    ) -> Result<(), BuildError> {
        for entry in read_dir_sorted(dir)? {
            let path = entry.path();
            let relative_path = path.strip_prefix(&self.root).map_err(|_| {
                context_error(format!(
                    "source path is outside the build context: {}",
                    path.display()
                ))
            })?;
            let relative = slash_path(relative_path);
            if skip_root_metadata && is_root_metadata(&relative) {
                continue;
            }
            let file_type = entry.file_type().map_err(|source| BuildError::Io {
                path: path.clone(),
                source,
            })?;
            let is_dir = file_type.is_dir();
            if is_ignored(&self.ignore, relative_path, is_dir) {
                continue;
            }
            if file_type.is_symlink() {
                return Err(context_error(format!(
                    "source path contains a symlink: {relative}"
                )));
            }
            if file_type.is_file() {
                let bytes = fs::read(&path).map_err(|source| BuildError::Io {
                    path: path.clone(),
                    source,
                })?;
                entries.push(SourceEntry::File(SourceFile { relative, bytes }));
            } else if is_dir {
                entries.push(SourceEntry::Directory(relative));
                self.collect_source_entries(&path, false, entries)?;
            } else {
                return Err(context_error(format!(
                    "source path contains an unsupported file type: {relative}"
                )));
            }
        }
        Ok(())
    }
}

fn build_ignore(root: &Path) -> Result<Gitignore, BuildError> {
    let mut builder = GitignoreBuilder::new(root);
    let path = root.join(DOSKERIGNORE_NAME);
    if path.is_file()
        && let Some(error) = builder.add(&path)
    {
        return Err(context_error(format!(
            "invalid {DOSKERIGNORE_NAME}: {error}"
        )));
    }
    builder
        .build()
        .map_err(|error| context_error(format!("invalid {DOSKERIGNORE_NAME}: {error}")))
}

fn compute_digest(root: &Path, ignore: &Gitignore) -> Result<String, BuildError> {
    let mut entries = BTreeMap::new();
    collect_context_entries(root, root, ignore, true, &mut entries)?;
    let bytes = canonical_json(&entries)?;
    Ok(sha256_digest(&bytes))
}

fn collect_context_entries(
    dir: &Path,
    root: &Path,
    ignore: &Gitignore,
    at_root: bool,
    entries: &mut BTreeMap<String, String>,
) -> Result<(), BuildError> {
    for entry in read_dir_sorted(dir)? {
        let path = entry.path();
        let relative_path = path.strip_prefix(root).map_err(|_| {
            context_error(format!(
                "context path is outside the build context: {}",
                path.display()
            ))
        })?;
        let relative = slash_path(relative_path);
        if at_root && is_root_metadata(&relative) {
            continue;
        }
        let file_type = entry.file_type().map_err(|source| BuildError::Io {
            path: path.clone(),
            source,
        })?;
        let is_dir = file_type.is_dir();
        if is_ignored(ignore, relative_path, is_dir) {
            continue;
        }
        if file_type.is_symlink() {
            let target = fs::read_link(&path).map_err(|source| BuildError::Io {
                path: path.clone(),
                source,
            })?;
            entries.insert(
                relative,
                format!(
                    "sha256:{}",
                    sha256_hex(target.as_os_str().as_encoded_bytes())
                ),
            );
        } else if file_type.is_file() {
            let bytes = fs::read(&path).map_err(|source| BuildError::Io {
                path: path.clone(),
                source,
            })?;
            entries.insert(relative, format!("sha256:{}", sha256_hex(&bytes)));
        } else if is_dir {
            collect_context_entries(&path, root, ignore, false, entries)?;
        }
    }
    Ok(())
}

fn read_dir_sorted(path: &Path) -> Result<Vec<fs::DirEntry>, BuildError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|source| BuildError::Io {
        path: path.to_path_buf(),
        source,
    })? {
        entries.push(entry.map_err(|source| BuildError::Io {
            path: path.to_path_buf(),
            source,
        })?);
    }
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn is_ignored(ignore: &Gitignore, relative: &Path, is_dir: bool) -> bool {
    if ignore.is_empty() {
        return false;
    }
    matches!(
        ignore.matched_path_or_any_parents(relative, is_dir),
        Match::Ignore(_)
    )
}

fn is_root_metadata(relative: &str) -> bool {
    relative == DOSKERFILE_NAME || relative == LOCK_FILE_NAME || relative == DOSKERIGNORE_NAME
}

fn has_glob_character(source: &str) -> bool {
    source
        .chars()
        .any(|character| matches!(character, '*' | '?' | '[' | ']' | '{' | '}'))
}

fn slash_path(path: &Path) -> String {
    path.iter()
        .map(|component| component.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut text, "{byte:02x}").expect("writing to a String cannot fail");
    }
    text
}

fn context_error(reason: impl Into<String>) -> BuildError {
    BuildError::Context {
        reason: reason.into(),
    }
}
