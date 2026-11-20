//! The Bochs engine adapter.

use std::sync::Arc;

use dosker_engine::{
    CapabilitySet, DisplayMode, EngineAdapter, EngineCommand, EngineError, EngineSession,
    EngineSpec,
};
use dosker_platform::process::ProcessSupervisor;

use crate::config::write_config;
use crate::discovery::BochsBinary;
use crate::endpoints::BochsEndpoints;
use crate::session::BochsSession;

/// The canonical Bochs engine name.
pub const ENGINE_NAME: &str = "bochs";

/// Drives the Bochs binary.
pub struct BochsAdapter {
    binary: BochsBinary,
    version: String,
    processes: Arc<dyn ProcessSupervisor>,
}

impl BochsAdapter {
    /// Creates an adapter for a discovered binary.
    pub fn new(binary: BochsBinary, processes: Arc<dyn ProcessSupervisor>) -> Self {
        let version = binary.version_string().to_owned();
        Self {
            binary,
            version,
            processes,
        }
    }

    /// Discovers Bochs and creates an adapter for it.
    pub fn discover(processes: Arc<dyn ProcessSupervisor>) -> Result<Self, EngineError> {
        let binary = BochsBinary::discover()?;
        Ok(Self::new(binary, processes))
    }
}

/// Returns the capabilities the Bochs adapter declares.
pub fn capabilities() -> CapabilitySet {
    dosker_engine::declared_capabilities(ENGINE_NAME).expect("bochs is an implemented engine")
}

impl EngineAdapter for BochsAdapter {
    fn name(&self) -> &str {
        ENGINE_NAME
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

    fn build_command(&self, spec: &EngineSpec) -> Result<EngineCommand, EngineError> {
        let endpoints = BochsEndpoints::for_spec(spec)?;
        match spec.display {
            DisplayMode::None => {}
            DisplayMode::Vnc => return Err(EngineError::Unsupported("vnc")),
            DisplayMode::Window => return Err(EngineError::Unsupported("window display")),
        }

        let config = write_config(spec, &endpoints)?;
        let mut args = vec![
            "-q".to_owned(),
            "-f".to_owned(),
            config.to_string_lossy().into_owned(),
        ];
        if cfg!(unix) && self.binary.has_internal_debugger() {
            args.push("-dbg".to_owned());
        }

        Ok(EngineCommand {
            program: self.binary.path.clone(),
            args,
            env: Vec::new(),
            endpoints: endpoints.engine_endpoints(),
        })
    }

    fn start(&mut self, spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError> {
        let command = self.build_command(spec)?;
        self.start_with_command(spec, &command)
    }

    fn start_with_command(
        &mut self,
        spec: &EngineSpec,
        command: &EngineCommand,
    ) -> Result<Box<dyn EngineSession>, EngineError> {
        let session = BochsSession::spawn(spec, self.processes.as_ref(), command.clone())?;
        Ok(Box::new(session))
    }
}
