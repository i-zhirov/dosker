//! Selecting and building engine adapters.

use dosker_engine::EngineAdapter;

use crate::error::CliError;

/// Returns the engine chosen when the command line names none.
pub fn default_engine() -> Option<&'static str> {
    #[cfg(feature = "mock-engine")]
    {
        Some("mock")
    }
    #[cfg(not(feature = "mock-engine"))]
    {
        None
    }
}

/// Reports whether the CLI can build an adapter for an engine name.
pub fn is_known(engine: &str) -> bool {
    #[cfg(feature = "mock-engine")]
    {
        engine == "mock"
    }
    #[cfg(not(feature = "mock-engine"))]
    {
        let _ = engine;
        false
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

    Err(CliError::Usage(format!("unknown engine: {engine}")))
}
