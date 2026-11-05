//! Control endpoints.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use dosker_core::ContainerId;
use serde::{Deserialize, Serialize};

use crate::PlatformError;

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
    /// A loopback TCP address.
    Tcp { host: Ipv4Addr, port: u16 },
}

impl EndpointRecord {
    /// Creates a loopback TCP endpoint record.
    pub fn tcp(port: u16) -> Self {
        Self::Tcp {
            host: Ipv4Addr::LOCALHOST,
            port,
        }
    }

    /// Returns the host-native control endpoint, when the record has one.
    pub fn local_endpoint(&self) -> Option<Endpoint> {
        match self {
            Self::UnixSocket { path } => Some(Endpoint::UnixSocket(path.clone())),
            Self::NamedPipe { name } => Some(Endpoint::NamedPipe(name.clone())),
            Self::Tcp { .. } => None,
        }
    }
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

impl TryFrom<EndpointRecord> for Endpoint {
    type Error = PlatformError;

    fn try_from(record: EndpointRecord) -> Result<Self, Self::Error> {
        match record {
            EndpointRecord::UnixSocket { path } => Ok(Self::UnixSocket(path)),
            EndpointRecord::NamedPipe { name } => Ok(Self::NamedPipe(name)),
            EndpointRecord::Tcp { host, port } => Err(PlatformError::InvalidEndpoint(format!(
                "tcp endpoint {host}:{port} is not a local control endpoint"
            ))),
        }
    }
}
