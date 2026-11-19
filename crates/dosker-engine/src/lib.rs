//! Engine adapter contract and capability model.

pub mod adapter;
pub mod capability;
pub mod error;
pub mod source;
pub mod spec;

pub use adapter::{EngineAdapter, EngineSession};
pub use capability::{Capability, CapabilitySet};
pub use error::EngineError;
pub use source::EngineSource;
pub use spec::{
    DebugKind, DisplayMode, EngineCommand, EngineEndpoint, EngineEndpointKind, EnginePort,
    EngineSpec, EngineVolume, ExecRequest, ExecResult,
};
