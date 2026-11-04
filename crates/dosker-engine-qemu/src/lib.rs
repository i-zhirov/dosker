//! QEMU engine adapter.

pub mod adapter;
pub mod discovery;

#[cfg(unix)]
pub mod command;
#[cfg(unix)]
pub mod paths;
#[cfg(unix)]
pub mod qmp;
#[cfg(unix)]
pub mod serial_log;
#[cfg(unix)]
pub mod session;

pub use adapter::{QemuAdapter, capabilities};
pub use discovery::{DEFAULT_QEMU, QEMU_ENV, QemuBinary, QemuVersion};
#[cfg(unix)]
pub use paths::QemuPaths;
#[cfg(unix)]
pub use qmp::QMP_CONNECT_TIMEOUT;
#[cfg(unix)]
pub use session::QemuSession;
