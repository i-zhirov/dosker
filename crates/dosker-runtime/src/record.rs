//! The persisted state of a container.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use dosker_core::{ContainerId, ContainerName, CoreError, Status};
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
        }
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
