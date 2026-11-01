//! Layout of the runtime data root.

use std::path::{Path, PathBuf};

use dosker_core::{ContainerId, ContainerName};

const STATE_FILE: &str = "state.json";
const ENGINE_LOG: &str = "engine.log";
const SERIAL_LOG: &str = "serial.log";
const CONTAINER_LOG: &str = "container.log";

/// The paths of the runtime data root and its container entries.
#[derive(Clone, Debug)]
pub struct RuntimePaths {
    root: PathBuf,
}

impl RuntimePaths {
    /// Builds the layout for a data root.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Borrows the data root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the directory that holds the containers.
    pub fn containers_dir(&self) -> PathBuf {
        self.root.join("containers")
    }

    /// Returns the directory that holds the name locks.
    pub fn names_dir(&self) -> PathBuf {
        self.containers_dir().join(".names")
    }

    /// Returns the directory of a container.
    pub fn container_dir(&self, id: &ContainerId) -> PathBuf {
        self.containers_dir().join(id.as_str())
    }

    /// Returns the name-lock directory of a container name.
    pub fn name_dir(&self, name: &ContainerName) -> PathBuf {
        self.names_dir().join(name.as_str())
    }

    /// Returns the state file of a container.
    pub fn state_file(&self, id: &ContainerId) -> PathBuf {
        self.container_dir(id).join(STATE_FILE)
    }

    /// Returns the engine log of a container.
    pub fn engine_log(&self, id: &ContainerId) -> PathBuf {
        self.container_dir(id).join(ENGINE_LOG)
    }

    /// Returns the serial log of a container.
    pub fn serial_log(&self, id: &ContainerId) -> PathBuf {
        self.container_dir(id).join(SERIAL_LOG)
    }

    /// Returns the container log of a container.
    pub fn container_log(&self, id: &ContainerId) -> PathBuf {
        self.container_dir(id).join(CONTAINER_LOG)
    }
}
