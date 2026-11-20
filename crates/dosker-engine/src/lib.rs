//! Engine adapter contract and capability model.

pub mod adapter;
pub mod capability;
pub mod error;
pub mod provider;
pub mod source;
pub mod spec;

pub use adapter::{EngineAdapter, EngineSession};
pub use capability::{Capability, CapabilitySet};
pub use error::EngineError;
pub use provider::{
    EngineProvider, IMPLEMENTED_ENGINES, capability_message, check_requires, declared_capabilities,
    implemented_engine_names, is_known, providers, supported_engines_message,
};
pub use source::EngineSource;
pub use spec::{
    DebugKind, DisplayMode, EngineCommand, EngineEndpoint, EngineEndpointKind, EnginePort,
    EngineSpec, EngineVolume, EngineVolumeKind, ExecRequest, ExecResult,
};
