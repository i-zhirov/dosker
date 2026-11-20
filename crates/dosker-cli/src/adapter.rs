//! Selecting and building engine adapters.

use std::path::PathBuf;

use dosker_engine::{CapabilitySet, EngineAdapter, EngineError};

use crate::error::CliError;

/// A discovered engine binary and the capabilities it declares.
pub struct DiscoveredEngine {
    /// The canonical engine name.
    pub name: &'static str,
    /// The version reported by the binary.
    pub version: String,
    /// The binary path.
    pub path: PathBuf,
    /// The capabilities the engine declares.
    pub capabilities: CapabilitySet,
}

/// Returns the engine chosen when the command line names none.
pub fn default_engine() -> Option<&'static str> {
    #[cfg(feature = "qemu-engine")]
    {
        Some("qemu")
    }
    #[cfg(all(not(feature = "qemu-engine"), feature = "mock-engine"))]
    {
        Some("mock")
    }
    #[cfg(all(not(feature = "qemu-engine"), not(feature = "mock-engine")))]
    {
        Some("qemu")
    }
}

/// Reports whether the CLI can build an adapter for an engine name.
pub fn is_known(engine: &str) -> bool {
    if dosker_engine::is_known(engine) {
        return true;
    }

    #[cfg(feature = "mock-engine")]
    {
        if engine == "mock" {
            return true;
        }
    }

    false
}

/// Reports whether a string is an engine name, implemented or not.
pub fn is_engine_name(engine: &str) -> bool {
    dosker_core::EngineName::parse(engine).is_some()
}

/// Returns the list of implemented engines for an error message.
pub fn supported_engines_message() -> String {
    dosker_engine::supported_engines_message()
}

/// Returns the capabilities an engine declares without discovering a binary.
pub fn declared_capabilities(engine: &str) -> Option<CapabilitySet> {
    if let Some(capabilities) = dosker_engine::declared_capabilities(engine) {
        return Some(capabilities);
    }

    #[cfg(feature = "mock-engine")]
    {
        use dosker_engine::Capability;

        if engine == "mock" {
            return Some(
                [
                    Capability::Boot,
                    Capability::Serial,
                    Capability::Stop,
                    Capability::Nat,
                    Capability::Vnc,
                ]
                .into_iter()
                .collect(),
            );
        }
    }

    None
}

/// Discovers an engine binary and returns its name, version, path, and
/// capabilities.
pub fn discover(engine: &str) -> Result<DiscoveredEngine, EngineError> {
    let capabilities = dosker_engine::declared_capabilities(engine)
        .ok_or_else(|| EngineError::NotFound(PathBuf::from(engine)))?;

    match engine {
        "qemu" => {
            let binary = dosker_engine_qemu::QemuBinary::discover()?;
            Ok(DiscoveredEngine {
                name: "qemu",
                version: binary.version_string(),
                path: binary.path.clone(),
                capabilities,
            })
        }
        "dosbox-x" => {
            let binary = dosker_engine_dosbox_x::DosboxXBinary::discover()?;
            Ok(DiscoveredEngine {
                name: "dosbox-x",
                version: binary.version_string().to_owned(),
                path: binary.path().to_path_buf(),
                capabilities,
            })
        }
        "bochs" => {
            let binary = dosker_engine_bochs::BochsBinary::discover()?;
            Ok(DiscoveredEngine {
                name: "bochs",
                version: binary.version_string().to_owned(),
                path: binary.path().to_path_buf(),
                capabilities,
            })
        }
        _ => Err(EngineError::NotFound(PathBuf::from(engine))),
    }
}

/// Builds the adapter for a known engine.
pub fn build(engine: &str) -> Result<Box<dyn EngineAdapter>, CliError> {
    #[cfg(feature = "mock-engine")]
    {
        use std::sync::Arc;

        use dosker_platform::process::{ProcessSupervisor, system};
        use dosker_test_support::fixtures::mock_engine_path;
        use dosker_test_support::mock_adapter::MockAdapter;

        if engine == "mock" {
            let path = mock_engine_path().map_err(CliError::Message)?;
            let processes: Arc<dyn ProcessSupervisor> = Arc::new(system());
            return Ok(Box::new(MockAdapter::new(path, processes)));
        }
    }

    if engine == "qemu" {
        use std::sync::Arc;

        use dosker_engine_qemu::QemuAdapter;
        use dosker_platform::process::{ProcessSupervisor, system};

        let processes: Arc<dyn ProcessSupervisor> = Arc::new(system());
        let adapter = QemuAdapter::discover(processes)
            .map_err(|error| CliError::Message(error.to_string()))?;
        return Ok(Box::new(adapter));
    }

    if engine == "dosbox-x" {
        use std::sync::Arc;

        use dosker_engine_dosbox_x::DosboxXAdapter;
        use dosker_platform::process::{ProcessSupervisor, system};

        let processes: Arc<dyn ProcessSupervisor> = Arc::new(system());
        let adapter = DosboxXAdapter::discover(processes)
            .map_err(|error| CliError::Message(error.to_string()))?;
        return Ok(Box::new(adapter));
    }

    if engine == "bochs" {
        use std::sync::Arc;

        use dosker_engine_bochs::BochsAdapter;
        use dosker_platform::process::{ProcessSupervisor, system};

        let processes: Arc<dyn ProcessSupervisor> = Arc::new(system());
        let adapter = BochsAdapter::discover(processes)
            .map_err(|error| CliError::Message(error.to_string()))?;
        return Ok(Box::new(adapter));
    }

    Err(CliError::Usage(format!("unknown engine: {engine}")))
}
