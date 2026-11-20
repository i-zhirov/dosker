//! The DOSBox-X engine adapter.

use std::sync::Arc;

use dosker_engine::{
    CapabilitySet, DisplayMode, EngineAdapter, EngineCommand, EngineError, EngineSession,
    EngineSpec,
};
use dosker_platform::process::ProcessSupervisor;

use crate::config::write_config;
use crate::discovery::DosboxXBinary;
use crate::endpoints::DosboxXEndpoints;
use crate::session::DosboxXSession;

/// The canonical DOSBox-X engine name.
pub const ENGINE_NAME: &str = "dosbox-x";

/// The flags that run DOSBox-X without a window.
pub const HEADLESS_FLAGS: &[&str] = &["-set", "sdl videodriver=dummy"];

/// Drives the DOSBox-X binary.
pub struct DosboxXAdapter {
    binary: DosboxXBinary,
    version: String,
    processes: Arc<dyn ProcessSupervisor>,
}

impl DosboxXAdapter {
    /// Creates an adapter for a discovered binary.
    pub fn new(binary: DosboxXBinary, processes: Arc<dyn ProcessSupervisor>) -> Self {
        let version = binary.version_string().to_owned();
        Self {
            binary,
            version,
            processes,
        }
    }

    /// Discovers DOSBox-X and creates an adapter for it.
    pub fn discover(processes: Arc<dyn ProcessSupervisor>) -> Result<Self, EngineError> {
        let binary = DosboxXBinary::discover()?;
        Ok(Self::new(binary, processes))
    }
}

/// Returns the capabilities the DOSBox-X adapter declares.
pub fn capabilities() -> CapabilitySet {
    dosker_engine::declared_capabilities(ENGINE_NAME).expect("dosbox-x is an implemented engine")
}

impl EngineAdapter for DosboxXAdapter {
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
        let endpoints = DosboxXEndpoints::for_spec(spec)?;
        if spec.display == DisplayMode::Vnc {
            return Err(EngineError::Unsupported("vnc"));
        }

        let config = write_config(spec, &endpoints)?;
        let mut args = vec!["-conf".to_owned(), config.to_string_lossy().into_owned()];
        if spec.display == DisplayMode::None {
            args.extend(HEADLESS_FLAGS.iter().map(|flag| (*flag).to_owned()));
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
        let session = DosboxXSession::spawn(spec, self.processes.as_ref(), command.clone())?;
        Ok(Box::new(session))
    }
}
