//! QEMU engine adapter.

pub mod adapter;
pub mod command;
pub mod config;
pub mod discovery;
pub mod endpoints;
pub mod qmp;
pub mod serial_log;
pub mod session;
pub mod stream;

pub use adapter::{QemuAdapter, capabilities};
pub use config::split_engine_config;
pub use discovery::{
    DEFAULT_QEMU, MIN_QEMU_VERSION, QEMU_ENV, QEMU_SHA256_ENV, QEMU_VERSION_ENV, QemuBinary,
    QemuVersion, engine_source,
};
pub use endpoints::QemuEndpoints;
pub use qmp::QMP_CONNECT_TIMEOUT;
pub use session::QemuSession;
pub use stream::EngineStream;
