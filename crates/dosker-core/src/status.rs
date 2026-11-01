//! Container lifecycle states.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The lifecycle state of a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Created,
    Starting,
    Running,
    Stopping,
    Stopped,
    Exited,
    Failed,
    Removed,
}

impl Status {
    /// Reports whether the lifecycle permits a move from `self` to `next`.
    pub fn can_transition_to(self, next: Status) -> bool {
        matches!(
            (self, next),
            (Status::Created, Status::Starting)
                | (Status::Created, Status::Removed)
                | (Status::Starting, Status::Running)
                | (Status::Starting, Status::Exited)
                | (Status::Starting, Status::Failed)
                | (Status::Running, Status::Stopping)
                | (Status::Running, Status::Exited)
                | (Status::Stopping, Status::Stopped)
                | (Status::Stopping, Status::Exited)
                | (Status::Stopped, Status::Starting)
                | (Status::Stopped, Status::Removed)
                | (Status::Exited, Status::Starting)
                | (Status::Exited, Status::Removed)
                | (Status::Failed, Status::Removed)
        )
    }

    /// Reports whether the container is expected to have a live supervisor.
    pub fn is_active(self) -> bool {
        matches!(self, Status::Starting | Status::Running | Status::Stopping)
    }

    /// Reports whether the container has reached its final state.
    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Removed)
    }

    /// Returns the wire name of the state.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Created => "created",
            Status::Starting => "starting",
            Status::Running => "running",
            Status::Stopping => "stopping",
            Status::Stopped => "stopped",
            Status::Exited => "exited",
            Status::Failed => "failed",
            Status::Removed => "removed",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
