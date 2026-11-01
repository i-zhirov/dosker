//! Host platform primitives: data roots, control endpoints, atomic files,
//! console output, process cleanup and spawn flags, and platform errors.

pub mod console;
pub mod control;
pub mod data_root;
pub mod endpoint;
pub mod error;
pub mod fs;
pub mod process;

pub use endpoint::{Endpoint, EndpointRecord};
#[cfg(unix)]
pub use endpoint::{MAX_UNIX_SOCKET_PATH_BYTES, short_socket_dir, short_socket_path};
pub use error::PlatformError;
