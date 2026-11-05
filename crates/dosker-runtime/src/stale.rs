//! Detection and recovery of containers whose supervisor is gone.

use chrono::{DateTime, Utc};
use dosker_core::{ContainerId, Status};
use dosker_platform::control::endpoint_alive as control_endpoint_alive;
use dosker_platform::process::{ProcessSupervisor, SystemProcesses, system};
use dosker_platform::{EndpointRecord, PlatformError};

use crate::record::ContainerRecord;
use crate::store::{StateStore, StoreError};

/// Answers questions about live host resources.
pub trait HostProbe: Send + Sync {
    /// Reports whether a process is still running.
    fn process_alive(&self, pid: u32) -> bool;

    /// Reports whether a control endpoint answers.
    fn endpoint_alive(&self, endpoint: &EndpointRecord) -> bool;

    /// Kills every process in a group.
    fn kill_group(&self, pgid: u32) -> Result<(), PlatformError>;
}

/// The host probe backed by the operating system.
pub struct SystemProbe {
    processes: SystemProcesses,
}

impl SystemProbe {
    /// Builds a probe over the host process table.
    pub fn new() -> Self {
        Self {
            processes: system(),
        }
    }
}

impl Default for SystemProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl HostProbe for SystemProbe {
    fn process_alive(&self, pid: u32) -> bool {
        self.processes.process_alive(pid)
    }

    fn endpoint_alive(&self, endpoint: &EndpointRecord) -> bool {
        match endpoint.local_endpoint() {
            Some(endpoint) => control_endpoint_alive(&endpoint),
            None => false,
        }
    }

    fn kill_group(&self, pgid: u32) -> Result<(), PlatformError> {
        self.processes.kill_group(pgid)
    }
}

/// Marks stale active containers as exited.
///
/// An active record is stale when its supervisor PID is missing or dead.
/// Running and stopping records are also stale when their control endpoint
/// is missing or dead; starting records are judged only by the supervisor
/// PID.
///
/// Returns the IDs of the records that changed. A failure to kill an
/// orphaned engine group is returned before the record is saved.
pub fn recover_stale(
    store: &StateStore,
    probe: &dyn HostProbe,
    now: DateTime<Utc>,
) -> Result<Vec<ContainerId>, StoreError> {
    let mut changed = Vec::new();

    for mut record in store.list()? {
        if !record.status.is_active() || !is_stale(&record, probe) {
            continue;
        }

        kill_engine_group(&record, probe)?;

        record.transition(Status::Exited, now)?;
        record.pid = None;
        record.engine_pid = None;
        record.engine_pgid = None;
        record.exit_code = None;
        record.dirty = true;
        store.save(&record)?;

        changed.push(record.id.clone());
    }

    Ok(changed)
}

fn is_stale(record: &ContainerRecord, probe: &dyn HostProbe) -> bool {
    let supervisor_alive = record.pid.is_some_and(|pid| probe.process_alive(pid));
    if !supervisor_alive {
        return true;
    }

    match record.status {
        Status::Running | Status::Stopping => match record.control_endpoint() {
            Some(endpoint) => !probe.endpoint_alive(endpoint),
            None => true,
        },
        _ => false,
    }
}

fn kill_engine_group(record: &ContainerRecord, probe: &dyn HostProbe) -> Result<(), PlatformError> {
    if let Some(pgid) = record.engine_pgid
        && probe.process_alive(pgid)
    {
        probe.kill_group(pgid)?;
    }
    Ok(())
}
