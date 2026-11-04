//! Control endpoints.

use std::path::{Path, PathBuf};

use dosker_core::ContainerId;
use serde::{Deserialize, Serialize};

/// A host-native control endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix domain socket path.
    UnixSocket(PathBuf),
    /// A Windows named-pipe name.
    NamedPipe(String),
}

/// The serialized form of a control endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EndpointRecord {
    /// A Unix domain socket path.
    UnixSocket { path: PathBuf },
    /// A Windows named-pipe name.
    NamedPipe { name: String },
}

/// The maximum length in bytes of a Unix socket path.
#[cfg(unix)]
pub const MAX_UNIX_SOCKET_PATH_BYTES: usize = 100;

/// Returns the fallback directory for Unix control sockets.
///
/// The directory sits under `/tmp` because a long data root can push the
/// default socket path past the platform limit.
#[cfg(unix)]
pub fn short_socket_dir() -> PathBuf {
    let euid = unsafe { libc::geteuid() };
    let tmp = Path::new("/tmp");
    if tmp.is_dir() {
        tmp.join(format!("dosker-{euid}"))
    } else {
        std::env::temp_dir().join(format!("dosker-{euid}"))
    }
}

/// Returns the fallback control socket path for a container.
#[cfg(unix)]
pub fn short_socket_path(id: &ContainerId) -> PathBuf {
    short_socket_dir().join(format!("{id}.sock"))
}

/// Returns the path of a named engine socket for a container.
///
/// The path sits in the container directory when it fits the platform limit
/// and falls back to the short socket directory otherwise. `name` must be one
/// of `agent`, `qmp`, `monitor`, or `gdb`.
#[cfg(unix)]
pub fn engine_socket_path(root: &Path, id: &ContainerId, name: &str) -> PathBuf {
    debug_assert!(
        matches!(name, "agent" | "qmp" | "monitor" | "gdb"),
        "engine socket name must be one of agent, qmp, monitor, gdb: {name}"
    );

    let default = root
        .join("containers")
        .join(id.as_str())
        .join(format!("{name}.sock"));
    if unix_path_len(&default) <= MAX_UNIX_SOCKET_PATH_BYTES {
        default
    } else {
        short_socket_dir().join(format!("{}-{name}.sock", id.as_str()))
    }
}

/// Returns the control endpoint for a container.
pub fn control_endpoint(root: &Path, id: &ContainerId) -> Endpoint {
    #[cfg(unix)]
    {
        let default = root
            .join("containers")
            .join(id.as_str())
            .join("control.sock");
        if unix_path_len(&default) <= MAX_UNIX_SOCKET_PATH_BYTES {
            Endpoint::UnixSocket(default)
        } else {
            Endpoint::UnixSocket(short_socket_path(id))
        }
    }

    #[cfg(windows)]
    {
        let _ = root;
        Endpoint::NamedPipe(format!(r"\\.\pipe\dosker-{id}-control"))
    }
}

#[cfg(unix)]
fn unix_path_len(path: &Path) -> usize {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().len()
}

impl Endpoint {
    /// Converts the endpoint to its serialized form.
    pub fn record(&self) -> EndpointRecord {
        match self {
            Self::UnixSocket(path) => EndpointRecord::UnixSocket { path: path.clone() },
            Self::NamedPipe(name) => EndpointRecord::NamedPipe { name: name.clone() },
        }
    }

    /// Renders the endpoint for a person.
    pub fn display(&self) -> String {
        match self {
            Self::UnixSocket(path) => path.display().to_string(),
            Self::NamedPipe(name) => name.clone(),
        }
    }
}

impl From<EndpointRecord> for Endpoint {
    fn from(record: EndpointRecord) -> Self {
        match record {
            EndpointRecord::UnixSocket { path } => Self::UnixSocket(path),
            EndpointRecord::NamedPipe { name } => Self::NamedPipe(name),
        }
    }
}
