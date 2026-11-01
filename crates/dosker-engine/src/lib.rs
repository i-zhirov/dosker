//! Engine adapter contract and capability model.

pub mod adapter;
pub mod capability;
pub mod error;
pub mod spec;

pub use adapter::{EngineAdapter, EngineSession};
pub use capability::{Capability, CapabilitySet};
pub use error::EngineError;
pub use spec::{EngineSpec, ExecRequest, ExecResult};
