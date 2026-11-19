//! Named volume images, their metadata, and their exclusive attachment lock.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::{ContainerId, ContainerName};
use dosker_image::{FatType, ImageError, VolumeLayout, create_volume};
use dosker_platform::PlatformError;
use dosker_platform::fs::{ensure_dir_mode, write_atomic};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::paths::RuntimePaths;

const VOLUME_SCHEMA_VERSION: u32 = 1;
const DIR_MODE: u32 = 0o700;
const HOLDER_READ_RETRIES: usize = 20;
const HOLDER_READ_RETRY_DELAY: Duration = Duration::from_millis(10);

/// The persisted metadata of a volume.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeMeta {
    pub schema_version: u32,
    pub name: String,
    pub size: u64,
    pub layout: VolumeLayout,
    pub fat_type: FatType,
    pub created_at: DateTime<Utc>,
    pub dirty: bool,
}

/// The container that currently holds a volume.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeHolder {
    pub container_id: ContainerId,
    pub container_name: String,
    pub supervisor_pid: u32,
}

/// A volume as reported by the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub meta: VolumeMeta,
    pub path: PathBuf,
    pub attached: Option<VolumeHolder>,
}

/// An exclusive attachment of one volume to one container.
#[derive(Debug)]
pub struct VolumeLock {
    file: File,
    #[allow(dead_code)]
    holder: VolumeHolder,
}

impl Drop for VolumeLock {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
        let _ = self.file.flush();
        let _ = FileExt::unlock(&self.file);
    }
}

/// A failure in a volume operation.
#[derive(Debug, thiserror::Error)]
pub enum VolumeError {
    #[error("volume not found: {0}")]
    NotFound(String),
    #[error("volume already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid volume name: {0}")]
    InvalidName(String),
    #[error("volume {name} is attached to container {}", holder.container_name)]
    InUse { name: String, holder: VolumeHolder },
    #[error("invalid volume metadata at {}: {message}", path.display())]
    InvalidMeta { path: PathBuf, message: String },
    #[error("i/o error at {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error(transparent)]
    Platform(#[from] PlatformError),
}

/// Stores volume images under a data root.
#[derive(Clone, Debug)]
pub struct VolumeStore {
    paths: RuntimePaths,
}

impl VolumeStore {
    /// Builds a store for a data root.
    pub fn new(root: PathBuf) -> Self {
        Self {
            paths: RuntimePaths::new(root),
        }
    }

    /// Creates a volume image and its metadata.
    pub fn create(&self, name: &str, size: u64, now: DateTime<Utc>) -> Result<Volume, VolumeError> {
        let parsed = check_name(name)?;
        let volumes_dir = self.paths.volumes_dir();
        ensure_dir_mode(&volumes_dir, DIR_MODE)?;

        let dir = self.paths.volume_dir(parsed.as_str());
        match fs::create_dir(&dir) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                return Err(VolumeError::AlreadyExists(name.to_owned()));
            }
            Err(source) => return Err(io_error(&dir, source)),
        }

        let result = (|| -> Result<VolumeMeta, VolumeError> {
            let image = create_volume(
                &self.paths.volume_image(parsed.as_str()),
                size,
                parsed.as_str(),
            )?;
            let meta = VolumeMeta {
                schema_version: VOLUME_SCHEMA_VERSION,
                name: parsed.as_str().to_owned(),
                size: image.disk_size,
                layout: image.layout,
                fat_type: image.fat_type,
                created_at: now,
                dirty: false,
            };
            self.write_meta(parsed.as_str(), &meta)?;
            Ok(meta)
        })();

        match result {
            Ok(meta) => Ok(Volume {
                meta,
                path: self.paths.volume_image(parsed.as_str()),
                attached: None,
            }),
            Err(error) => {
                let _ = fs::remove_dir_all(&dir);
                Err(error)
            }
        }
    }

    /// Loads a volume and reports its current holder.
    pub fn load(&self, name: &str) -> Result<Volume, VolumeError> {
        let parsed = check_name(name)?;
        let meta = self.read_meta(parsed.as_str())?;
        let attached = self.attached_holder(parsed.as_str())?;
        Ok(Volume {
            meta,
            path: self.paths.volume_image(parsed.as_str()),
            attached,
        })
    }

    /// Loads every volume, sorted by name.
    pub fn list(&self) -> Result<Vec<Volume>, VolumeError> {
        let directory = self.paths.volumes_dir();
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(io_error(&directory, source)),
        };

        let mut volumes = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| io_error(&directory, source))?;
            let path = entry.path();
            if !entry
                .file_type()
                .map_err(|source| io_error(&path, source))?
                .is_dir()
            {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if ContainerName::parse(name).is_err() {
                continue;
            }
            match self.load(name) {
                Ok(volume) => volumes.push(volume),
                Err(VolumeError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }

        volumes.sort_by(|left, right| left.meta.name.cmp(&right.meta.name));
        Ok(volumes)
    }

    /// Removes a volume that is not attached to a container.
    pub fn remove(&self, name: &str) -> Result<(), VolumeError> {
        let parsed = check_name(name)?;
        let dir = self.paths.volume_dir(parsed.as_str());
        if !dir.is_dir() {
            return Err(VolumeError::NotFound(name.to_owned()));
        }

        let lock_path = self.paths.volume_lock(parsed.as_str());
        match OpenOptions::new().read(true).write(true).open(&lock_path) {
            Ok(mut file) => match FileExt::try_lock_exclusive(&file) {
                Ok(()) => {
                    let _ = FileExt::unlock(&file);
                    drop(file);
                }
                Err(_) => {
                    let holder = read_holder(&mut file, &lock_path)?;
                    return Err(VolumeError::InUse {
                        name: parsed.as_str().to_owned(),
                        holder,
                    });
                }
            },
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(&lock_path, source)),
        }

        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                Err(VolumeError::NotFound(name.to_owned()))
            }
            Err(source) => Err(io_error(&dir, source)),
        }
    }

    /// Attaches a volume to a container.
    pub fn acquire(&self, name: &str, holder: VolumeHolder) -> Result<VolumeLock, VolumeError> {
        let parsed = check_name(name)?;
        let path = self.paths.volume_lock(parsed.as_str());
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| {
                if source.kind() == io::ErrorKind::NotFound {
                    VolumeError::NotFound(name.to_owned())
                } else {
                    io_error(&path, source)
                }
            })?;

        if FileExt::try_lock_exclusive(&file).is_err() {
            let holder = read_holder(&mut file, &path)?;
            return Err(VolumeError::InUse {
                name: parsed.as_str().to_owned(),
                holder,
            });
        }

        if let Err(error) = write_holder(&mut file, &path, &holder) {
            let _ = FileExt::unlock(&file);
            return Err(error);
        }

        Ok(VolumeLock { file, holder })
    }

    /// Replaces the dirty flag of a volume.
    pub fn set_dirty(&self, name: &str, dirty: bool) -> Result<(), VolumeError> {
        let parsed = check_name(name)?;
        let mut meta = self.read_meta(parsed.as_str())?;
        meta.dirty = dirty;
        self.write_meta(parsed.as_str(), &meta)
    }

    fn read_meta(&self, name: &str) -> Result<VolumeMeta, VolumeError> {
        let path = self.paths.volume_meta(name);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(VolumeError::NotFound(name.to_owned()));
            }
            Err(source) => return Err(io_error(&path, source)),
        };

        let meta: VolumeMeta =
            serde_json::from_slice(&bytes).map_err(|error| VolumeError::InvalidMeta {
                path: path.clone(),
                message: error.to_string(),
            })?;
        if meta.schema_version != VOLUME_SCHEMA_VERSION {
            return Err(VolumeError::InvalidMeta {
                path,
                message: format!("unsupported schema version {}", meta.schema_version),
            });
        }

        Ok(meta)
    }

    fn write_meta(&self, name: &str, meta: &VolumeMeta) -> Result<(), VolumeError> {
        let path = self.paths.volume_meta(name);
        let mut bytes =
            serde_json::to_vec_pretty(meta).map_err(|error| VolumeError::InvalidMeta {
                path: path.clone(),
                message: error.to_string(),
            })?;
        bytes.push(b'\n');
        write_atomic(&path, &bytes)?;
        Ok(())
    }

    fn attached_holder(&self, name: &str) -> Result<Option<VolumeHolder>, VolumeError> {
        let path = self.paths.volume_lock(name);
        let mut file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io_error(&path, source)),
        };

        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                let _ = FileExt::unlock(&file);
                Ok(None)
            }
            Err(_) => Ok(Some(read_holder(&mut file, &path)?)),
        }
    }
}

fn check_name(name: &str) -> Result<ContainerName, VolumeError> {
    ContainerName::parse(name).map_err(|_| VolumeError::InvalidName(name.to_owned()))
}

fn read_holder(file: &mut File, path: &Path) -> Result<VolumeHolder, VolumeError> {
    let mut message = String::from("the lock file is empty");
    for attempt in 0..=HOLDER_READ_RETRIES {
        if attempt > 0 {
            thread::sleep(HOLDER_READ_RETRY_DELAY);
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|source| io_error(path, source))?;
        let mut text = String::new();
        file.read_to_string(&mut text)
            .map_err(|source| io_error(path, source))?;
        let text = text.trim();
        if text.is_empty() {
            message = "the lock file is empty".to_owned();
            continue;
        }
        match serde_json::from_str(text) {
            Ok(holder) => return Ok(holder),
            Err(error) => message = error.to_string(),
        }
    }

    Err(VolumeError::InvalidMeta {
        path: path.to_path_buf(),
        message: format!("invalid volume lock: {message}"),
    })
}

fn write_holder(file: &mut File, path: &Path, holder: &VolumeHolder) -> Result<(), VolumeError> {
    file.set_len(0).map_err(|source| io_error(path, source))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| io_error(path, source))?;
    let mut bytes = serde_json::to_vec(holder).map_err(|error| VolumeError::InvalidMeta {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .map_err(|source| io_error(path, source))?;
    file.flush().map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: io::Error) -> VolumeError {
    VolumeError::Io {
        path: path.to_path_buf(),
        source,
    }
}
