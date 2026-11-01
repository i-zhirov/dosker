//! Core domain types: container IDs, names, lifecycle statuses, and errors.

pub mod error;
pub mod id;
pub mod name;
pub mod status;

pub use error::CoreError;
pub use id::ContainerId;
pub use name::ContainerName;
pub use status::Status;
