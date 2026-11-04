//! The persisted state of a container.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use dosker_core::{ContainerId, ContainerName, CoreError, Status};
use dosker_engine::DisplayMode;
use dosker_platform::EndpointRecord;
use serde::{Deserialize, Serialize};

/// The key of the control endpoint in the socket map.
pub const CONTROL_SOCKET: &str = "control";

/// The version of the state file format.
pub const RECORD_SCHEMA_VERSION: u32 = 1;

/// Everything the runtime persists about one container.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerRecord {
    pub schema_version: u32,
    pub id: ContainerId,
    pub name: ContainerName,
    pub image: String,
    pub engine: String,
    pub engine_version: String,
    pub status: Status,
    pub pid: Option<u32>,
    pub engine_pid: Option<u32>,
    #[serde(default)]
    pub engine_pgid: Option<u32>,
    pub exit_code: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub sockets: BTreeMap<String, EndpointRecord>,
    pub disk_path: Option<PathBuf>,
    pub dirty: bool,
    #[serde(default)]
    pub image_digest: String,
    #[serde(default)]
    pub booter: bool,
    #[serde(default)]
    pub display: DisplayMode,
    #[serde(default)]
    pub vnc_port: Option<u16>,
    #[serde(default)]
    pub cmd: Option<String>,
    #[serde(default)]
    pub engine_command: Option<String>,
}

impl ContainerRecord {
    /// Creates a record in the `created` state.
    pub fn new(
        id: ContainerId,
        name: ContainerName,
        image: String,
        engine: String,
        control: EndpointRecord,
        now: DateTime<Utc>,
    ) -> Self {
        let mut sockets = BTreeMap::new();
        sockets.insert(CONTROL_SOCKET.to_owned(), control);

        Self {
            schema_version: RECORD_SCHEMA_VERSION,
            id,
            name,
            image,
            engine,
            engine_version: String::new(),
            status: Status::Created,
            pid: None,
            engine_pid: None,
            engine_pgid: None,
            exit_code: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            sockets,
            disk_path: None,
            dirty: false,
            image_digest: String::new(),
            booter: false,
            display: DisplayMode::None,
            vnc_port: None,
            cmd: None,
            engine_command: None,
        }
    }

    /// Records the image digest, whether the image is a booter, and the
    /// startup command.
    pub fn with_image(mut self, digest: String, booter: bool, cmd: Option<String>) -> Self {
        self.image_digest = digest;
        self.booter = booter;
        self.cmd = cmd;
        self
    }

    /// Records the display mode and the allocated VNC port.
    pub fn with_display(mut self, display: DisplayMode, vnc_port: Option<u16>) -> Self {
        self.display = display;
        self.vnc_port = vnc_port;
        self
    }

    /// Records the private container disk path.
    pub fn with_disk(mut self, path: PathBuf) -> Self {
        self.disk_path = Some(path);
        self
    }

    /// Returns the recorded control endpoint.
    pub fn control_endpoint(&self) -> Option<&EndpointRecord> {
        self.sockets.get(CONTROL_SOCKET)
    }

    /// Moves the record to another lifecycle state.
    ///
    /// Repeating the current state is accepted and leaves the record
    /// unchanged.
    pub fn transition(&mut self, next: Status, now: DateTime<Utc>) -> Result<(), CoreError> {
        if next == self.status {
            return Ok(());
        }
        if !self.status.can_transition_to(next) {
            return Err(CoreError::InvalidTransition {
                from: self.status,
                to: next,
            });
        }

        self.status = next;

        if next == Status::Running && self.started_at.is_none() {
            self.started_at = Some(now);
        }
        if matches!(next, Status::Stopped | Status::Exited | Status::Failed) {
            self.finished_at = Some(now);
        }

        Ok(())
    }
}
