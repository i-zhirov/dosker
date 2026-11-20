//! Bochs engine adapter.

pub mod adapter;
pub mod config;
pub mod discovery;
pub mod endpoints;
pub mod geometry;

#[cfg(unix)]
mod pty;
mod serial_log;
pub mod session;
mod stream;

pub use adapter::{BochsAdapter, ENGINE_NAME, capabilities};
pub use config::{
    CONFIG_FILE_NAME, CPU_MODEL, HEADLESS_DISPLAY, MEMORY_MB, render_config, write_config,
};
pub use discovery::{
    BOCHS_ENV, BOCHS_SHA256_ENV, BOCHS_VERSION_ENV, BochsBinary, DEFAULT_BOCHS, Debugger,
};
pub use endpoints::BochsEndpoints;
pub use geometry::DiskGeometry;
pub use session::BochsSession;
