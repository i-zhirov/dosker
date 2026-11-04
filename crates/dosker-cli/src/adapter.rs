//! Selecting and building engine adapters.

use dosker_engine::EngineAdapter;

use crate::error::CliError;

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
    if engine == "qemu" {
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

    Err(CliError::Usage(format!("unknown engine: {engine}")))
}
