//! The engine adapter contract.

use std::time::Duration;

use crate::capability::CapabilitySet;
use crate::error::EngineError;
use crate::spec::{EngineSpec, ExecRequest, ExecResult};

/// Translates an [`EngineSpec`] into a running engine and controls it.
pub trait EngineAdapter: Send {
    /// The adapter's engine name.
    fn name(&self) -> &str;

    /// The engine version the adapter targets.
    fn version(&self) -> &str;

    /// The capabilities the engine declares.
    fn capabilities(&self) -> CapabilitySet;

    /// Checks that the engine is available and usable.
    fn probe(&mut self) -> Result<(), EngineError>;

    /// Starts the engine and returns a handle to the running session.
    fn start(&mut self, spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError>;
}

/// One running engine owned by a supervisor.
pub trait EngineSession: Send {
    /// The engine process identifier, once it is known.
    fn pid(&self) -> Option<u32>;

    /// The engine process group, once it is known.
    fn process_group(&self) -> Option<u32>;

    /// Waits until the engine is ready for guest commands.
    fn ready(&mut self, timeout: Duration) -> Result<(), EngineError>;

    /// Runs a command inside the guest.
    fn exec(&mut self, request: &ExecRequest) -> Result<ExecResult, EngineError>;

    /// Stops the engine gracefully and returns its exit code.
    fn stop(&mut self, grace: Duration) -> Result<i32, EngineError>;

    /// Stops the engine immediately and returns its exit code.
    fn kill(&mut self) -> Result<i32, EngineError>;

    /// Reports the exit code when the engine has already exited.
    fn try_exit(&mut self) -> Result<Option<i32>, EngineError>;
}
