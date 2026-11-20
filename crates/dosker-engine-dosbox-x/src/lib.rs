//! DOSBox-X engine adapter.

pub mod adapter;
pub mod config;
pub mod discovery;
pub mod endpoints;
mod serial_log;
pub mod session;
mod stream;

pub use adapter::{DosboxXAdapter, ENGINE_NAME, HEADLESS_FLAGS, capabilities};
pub use config::{CONFIG_FILE_NAME, render_config, write_config};
pub use discovery::{
    DEFAULT_DOSBOX_X, DOSBOX_X_ENV, DOSBOX_X_SHA256_ENV, DOSBOX_X_VERSION_ENV, DosboxXBinary,
};
pub use endpoints::DosboxXEndpoints;
pub use session::DosboxXSession;
