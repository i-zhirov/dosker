//! The engine adapter contract.

use std::path::Path;
use std::time::Duration;

use crate::capability::CapabilitySet;
use crate::error::EngineError;
use crate::spec::{DebugKind, EngineCommand, EngineEndpoint, EngineSpec, ExecRequest, ExecResult};

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

    /// Builds the engine command line without starting the engine.
    fn build_command(&self, spec: &EngineSpec) -> Result<EngineCommand, EngineError> {
        let _ = spec;
        Err(EngineError::Unsupported("command inspection"))
    }

    /// Starts the engine from an already built command line.
    fn start_with_command(
        &mut self,
        spec: &EngineSpec,
        command: &EngineCommand,
    ) -> Result<Box<dyn EngineSession>, EngineError> {
        let _ = command;
        self.start(spec)
    }

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

    /// Returns the command line the session was started from, when known.
    fn command(&self) -> Option<&EngineCommand> {
        None
    }

    /// Returns the engine endpoints the session exposes.
    fn endpoints(&self) -> Vec<EngineEndpoint> {
        Vec::new()
    }

    /// Captures the guest screen to a file.
    fn screenshot(&mut self, destination: &Path) -> Result<(), EngineError> {
        let _ = destination;
        Err(EngineError::Unsupported("screenshot"))
    }

    /// Returns the debugger endpoint for a debugger kind.
    fn debug_endpoint(&mut self, kind: DebugKind) -> Result<String, EngineError> {
        let _ = kind;
        Err(EngineError::Unsupported("debugger"))
    }

    /// Uploads a file into the guest and returns the written byte count.
    fn put_file(&mut self, path: &str, data: &[u8]) -> Result<u64, EngineError> {
        let _ = (path, data);
        Err(EngineError::Unsupported("file upload"))
    }

    /// Downloads a file from the guest.
    fn get_file(&mut self, path: &str) -> Result<Vec<u8>, EngineError> {
        let _ = path;
        Err(EngineError::Unsupported("file download"))
    }
}
