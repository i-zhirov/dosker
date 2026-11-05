//! QEMU engine adapter.

pub mod adapter;
pub mod command;
pub mod discovery;
pub mod endpoints;
pub mod qmp;
pub mod serial_log;
pub mod session;
pub mod stream;

pub use adapter::{QemuAdapter, capabilities};
pub use discovery::{DEFAULT_QEMU, QEMU_ENV, QemuBinary, QemuVersion};
pub use endpoints::QemuEndpoints;
pub use qmp::QMP_CONNECT_TIMEOUT;
pub use session::QemuSession;
pub use stream::EngineStream;
