//! Process cleanup and detached spawn flags.

use std::process::Command;

use crate::error::PlatformError;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::JobObject;

/// Controls how dosker starts and cleans up child processes.
pub trait ProcessSupervisor: Send + Sync {
    /// Starts the child in a session of its own.
    fn prepare_detached(&self, cmd: &mut Command) -> Result<(), PlatformError>;

    /// Starts the child in a process group that belongs to it.
    fn prepare_engine(&self, cmd: &mut Command) -> Result<(), PlatformError>;

    /// Kills every process in the group.
    ///
    /// On Windows this ends the named process. The Job Object ends the engine
    /// when the supervisor exits.
    fn kill_group(&self, pgid: u32) -> Result<(), PlatformError>;

    /// Reports whether a process is still running.
    fn process_alive(&self, pid: u32) -> bool;

    /// Returns the process group of a process, or `None` when it is gone.
    fn process_group(&self, pid: u32) -> Result<Option<u32>, PlatformError>;
}

/// The host process supervisor.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProcesses;

/// Returns the host process supervisor.
pub fn system() -> SystemProcesses {
    SystemProcesses
}

impl ProcessSupervisor for SystemProcesses {
    #[cfg(unix)]
    fn prepare_detached(&self, cmd: &mut Command) -> Result<(), PlatformError> {
        unix::prepare_detached(cmd)
    }

    #[cfg(windows)]
    fn prepare_detached(&self, cmd: &mut Command) -> Result<(), PlatformError> {
        windows::prepare_detached(cmd)
    }

    #[cfg(unix)]
    fn prepare_engine(&self, cmd: &mut Command) -> Result<(), PlatformError> {
        unix::prepare_engine(cmd)
    }

    #[cfg(windows)]
    fn prepare_engine(&self, cmd: &mut Command) -> Result<(), PlatformError> {
        windows::prepare_engine(cmd)
    }

    #[cfg(unix)]
    fn kill_group(&self, pgid: u32) -> Result<(), PlatformError> {
        unix::kill_group(pgid)
    }

    #[cfg(windows)]
    fn kill_group(&self, pgid: u32) -> Result<(), PlatformError> {
        windows::kill_group(pgid)
    }

    #[cfg(unix)]
    fn process_alive(&self, pid: u32) -> bool {
        unix::process_alive(pid)
    }

    #[cfg(windows)]
    fn process_alive(&self, pid: u32) -> bool {
        windows::process_alive(pid)
    }

    #[cfg(unix)]
    fn process_group(&self, pid: u32) -> Result<Option<u32>, PlatformError> {
        unix::process_group(pid)
    }

    #[cfg(windows)]
    fn process_group(&self, pid: u32) -> Result<Option<u32>, PlatformError> {
        windows::process_group(pid)
    }
}
