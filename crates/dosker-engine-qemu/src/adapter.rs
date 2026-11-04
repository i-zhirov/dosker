//! The QEMU engine adapter.

use std::sync::Arc;

use dosker_engine::{
    Capability, CapabilitySet, EngineAdapter, EngineCommand, EngineError, EngineSession, EngineSpec,
};
use dosker_platform::process::ProcessSupervisor;

use crate::discovery::QemuBinary;

/// Drives the QEMU binary.
pub struct QemuAdapter {
    binary: QemuBinary,
    version: String,
    processes: Arc<dyn ProcessSupervisor>,
}

impl QemuAdapter {
    /// Creates an adapter for a discovered binary.
    pub fn new(binary: QemuBinary, processes: Arc<dyn ProcessSupervisor>) -> Self {
        let version = binary.version_string();
        Self {
            binary,
            version,
            processes,
        }
    }

    /// Discovers QEMU and creates an adapter for it.
    pub fn discover(processes: Arc<dyn ProcessSupervisor>) -> Result<Self, EngineError> {
        let binary = QemuBinary::discover()?;
        Ok(Self::new(binary, processes))
    }

    /// Returns the discovered binary.
    pub fn binary(&self) -> &QemuBinary {
        &self.binary
    }
}

/// Returns the capabilities the QEMU adapter declares.
pub fn capabilities() -> CapabilitySet {
    [
        Capability::Boot,
        Capability::Serial,
        Capability::Stop,
        Capability::Headless,
        Capability::DebugNative,
        Capability::GdbStub,
        Capability::Screenshot,
        Capability::Vnc,
    ]
    .into_iter()
    .collect()
}

impl EngineAdapter for QemuAdapter {
    fn name(&self) -> &str {
        "qemu"
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> CapabilitySet {
        capabilities()
    }

    fn probe(&mut self) -> Result<(), EngineError> {
        if self.binary.path.is_file() {
            Ok(())
        } else {
            Err(EngineError::NotFound(self.binary.path.clone()))
        }
    }

    #[cfg(unix)]
    fn build_command(&self, spec: &EngineSpec) -> Result<EngineCommand, EngineError> {
        crate::command::build(spec, &self.binary)
    }

    #[cfg(windows)]
    fn build_command(&self, spec: &EngineSpec) -> Result<EngineCommand, EngineError> {
        let _ = spec;
        Err(EngineError::Unsupported("QEMU command line"))
    }

    #[cfg(unix)]
    fn start(&mut self, spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError> {
        let command = self.build_command(spec)?;
        self.start_with_command(spec, &command)
    }

    #[cfg(unix)]
    fn start_with_command(
        &mut self,
        spec: &EngineSpec,
        command: &EngineCommand,
    ) -> Result<Box<dyn EngineSession>, EngineError> {
        let session = crate::session::QemuSession::spawn(
            &self.binary,
            spec,
            self.processes.as_ref(),
            command.clone(),
        )?;
        Ok(Box::new(session))
    }

    #[cfg(windows)]
    fn start(&mut self, spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError> {
        let _ = spec;
        Err(EngineError::Unsupported("QEMU runtime on this host"))
    }

    #[cfg(windows)]
    fn start_with_command(
        &mut self,
        spec: &EngineSpec,
        command: &EngineCommand,
    ) -> Result<Box<dyn EngineSession>, EngineError> {
        let _ = (spec, command);
        Err(EngineError::Unsupported("QEMU runtime on this host"))
    }
}
