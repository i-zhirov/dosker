//! Atomic persistence of container records.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use dosker_core::{ContainerId, ContainerName, CoreError};
use dosker_platform::PlatformError;
use dosker_platform::control::remove_endpoint;
use dosker_platform::fs::{ensure_dir_mode, write_atomic};

use crate::paths::RuntimePaths;
use crate::record::{ContainerRecord, RECORD_SCHEMA_VERSION};

const DIR_MODE: u32 = 0o700;

/// A failure in the state store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("container not found: {0}")]
    NotFound(ContainerId),
    #[error("container already exists: {0}")]
    AlreadyExists(ContainerId),
    #[error("container name already in use: {0}")]
    NameInUse(ContainerName),
    #[error("i/o error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("corrupt state at {path}: {message}")]
    Corrupt { path: PathBuf, message: String },
    #[error(transparent)]
    Platform(#[from] PlatformError),
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// Stores container records under a data root.
#[derive(Clone, Debug)]
pub struct StateStore {
    paths: RuntimePaths,
}

impl StateStore {
    /// Builds a store for a data root.
    pub fn new(root: PathBuf) -> Self {
        Self {
            paths: RuntimePaths::new(root),
        }
    }

    /// Borrows the store layout.
    pub fn paths(&self) -> &RuntimePaths {
        &self.paths
    }

    /// Creates a container directory, its logs, and its state.
    pub fn create(&self, record: &ContainerRecord) -> Result<(), StoreError> {
        self.ensure_layout()?;

        let state_file = self.paths.state_file(&record.id);
        if state_file.exists() {
            return Err(StoreError::AlreadyExists(record.id.clone()));
        }

        self.lock_name(&record.name, &record.id)?;

        let container_dir = self.paths.container_dir(&record.id);
        let result = (|| -> Result<(), StoreError> {
            ensure_dir_mode(&container_dir, DIR_MODE)?;
            for path in [
                self.paths.engine_log(&record.id),
                self.paths.serial_log(&record.id),
                self.paths.container_log(&record.id),
            ] {
                fs::File::create(&path).map_err(|source| io_error(&path, source))?;
            }
            self.write_state(record)
        })();

        if let Err(error) = result {
            let _ = fs::remove_dir_all(self.paths.name_dir(&record.name));
            let _ = fs::remove_dir_all(&container_dir);
            return Err(error);
        }

        Ok(())
    }

    /// Loads a container record.
    pub fn load(&self, id: &ContainerId) -> Result<ContainerRecord, StoreError> {
        let path = self.paths.state_file(id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::NotFound(id.clone()));
            }
            Err(source) => return Err(io_error(&path, source)),
        };

        let record: ContainerRecord =
            serde_json::from_slice(&bytes).map_err(|error| StoreError::Corrupt {
                path: path.clone(),
                message: error.to_string(),
            })?;

        if record.schema_version != RECORD_SCHEMA_VERSION {
            return Err(StoreError::Corrupt {
                path,
                message: format!("unsupported schema version {}", record.schema_version),
            });
        }

        Ok(record)
    }

    /// Replaces a saved container record.
    pub fn save(&self, record: &ContainerRecord) -> Result<(), StoreError> {
        self.write_state(record)
    }

    /// Loads every container record, oldest first.
    pub fn list(&self) -> Result<Vec<ContainerRecord>, StoreError> {
        let directory = self.paths.containers_dir();
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(io_error(&directory, source)),
        };

        let mut records = Vec::new();
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
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Ok(id) = ContainerId::parse(file_name) else {
                continue;
            };
            if !self.paths.state_file(&id).is_file() {
                continue;
            }
            match self.load(&id) {
                Ok(record) => records.push(record),
                Err(StoreError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }

        records.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });

        Ok(records)
    }

    /// Finds a container by name.
    pub fn find_by_name(
        &self,
        name: &ContainerName,
    ) -> Result<Option<ContainerRecord>, StoreError> {
        let name_dir = self.paths.name_dir(name);
        let id_file = name_dir.join("id");
        let id_text = match fs::read_to_string(&id_file) {
            Ok(text) => text,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                let _ = fs::remove_dir_all(&name_dir);
                return Ok(None);
            }
            Err(source) => return Err(io_error(&id_file, source)),
        };

        let Ok(id) = ContainerId::parse(id_text.trim()) else {
            let _ = fs::remove_dir_all(&name_dir);
            return Ok(None);
        };

        if !self.paths.state_file(&id).is_file() {
            let _ = fs::remove_dir_all(&name_dir);
            return Ok(None);
        }

        match self.load(&id) {
            Ok(record) => Ok(Some(record)),
            Err(StoreError::NotFound(_)) => {
                let _ = fs::remove_dir_all(&name_dir);
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Removes a container's endpoints, name lock, and directory.
    ///
    /// A missing endpoint, name lock, or directory is success.
    pub fn remove(&self, record: &ContainerRecord) -> Result<(), StoreError> {
        for recorded in record.sockets.values() {
            if let Some(endpoint) = recorded.local_endpoint() {
                remove_endpoint(&endpoint).map_err(StoreError::from)?;
            }
        }

        let name_dir = self.paths.name_dir(&record.name);
        let id_file = name_dir.join("id");
        match fs::read_to_string(&id_file) {
            Ok(id_text) => {
                if id_text.trim() == record.id.as_str() {
                    match fs::remove_dir_all(&name_dir) {
                        Ok(()) => {}
                        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                        Err(source) => return Err(io_error(&name_dir, source)),
                    }
                }
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(&id_file, source)),
        }

        let container_dir = self.paths.container_dir(&record.id);
        match fs::remove_dir_all(&container_dir) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(&container_dir, source)),
        }

        Ok(())
    }

    fn ensure_layout(&self) -> Result<(), StoreError> {
        ensure_dir_mode(self.paths.root(), DIR_MODE)?;
        ensure_dir_mode(&self.paths.containers_dir(), DIR_MODE)?;
        ensure_dir_mode(&self.paths.names_dir(), DIR_MODE)?;
        Ok(())
    }

    fn lock_name(&self, name: &ContainerName, id: &ContainerId) -> Result<(), StoreError> {
        let name_dir = self.paths.name_dir(name);
        match fs::create_dir(&name_dir) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                if !self.name_lock_is_stale(&name_dir)? {
                    return Err(StoreError::NameInUse(name.clone()));
                }
                fs::remove_dir_all(&name_dir).map_err(|source| io_error(&name_dir, source))?;
                match fs::create_dir(&name_dir) {
                    Ok(()) => {}
                    Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                        return Err(StoreError::NameInUse(name.clone()));
                    }
                    Err(source) => return Err(io_error(&name_dir, source)),
                }
            }
            Err(source) => return Err(io_error(&name_dir, source)),
        }

        let id_file = name_dir.join("id");
        if let Err(error) = write_atomic(&id_file, format!("{id}\n").as_bytes()) {
            let _ = fs::remove_dir_all(&name_dir);
            return Err(error.into());
        }

        Ok(())
    }

    fn name_lock_is_stale(&self, name_dir: &Path) -> Result<bool, StoreError> {
        let id_file = name_dir.join("id");
        let id_text = match fs::read_to_string(&id_file) {
            Ok(text) => text,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(source) => return Err(io_error(&id_file, source)),
        };

        let Ok(id) = ContainerId::parse(id_text.trim()) else {
            return Ok(true);
        };

        Ok(!self.paths.state_file(&id).is_file())
    }

    fn write_state(&self, record: &ContainerRecord) -> Result<(), StoreError> {
        let path = self.paths.state_file(&record.id);
        let mut bytes = serde_json::to_vec_pretty(record).map_err(|error| StoreError::Corrupt {
            path: path.clone(),
            message: error.to_string(),
        })?;
        bytes.push(b'\n');
        write_atomic(&path, &bytes)?;
        Ok(())
    }
}

fn io_error(path: &Path, source: io::Error) -> StoreError {
    StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}
