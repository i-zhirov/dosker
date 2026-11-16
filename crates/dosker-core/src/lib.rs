//! Core domain types: container IDs, names, lifecycle statuses, and errors.

pub mod doskerfile;
pub mod error;
pub mod id;
pub mod lockfile;
pub mod name;
pub mod status;

pub use doskerfile::{
    DOSKERFILE_NAME, Doskerfile, DoskerfileError, EngineName, FromSource, Instruction,
    InstructionKind,
};
pub use error::CoreError;
pub use id::ContainerId;
pub use lockfile::{
    AgentPin, BasePin, EnginePin, LOCK_FILE_NAME, LOCK_VERSION, LockFile, LockFileError,
};
pub use name::ContainerName;
pub use status::Status;
