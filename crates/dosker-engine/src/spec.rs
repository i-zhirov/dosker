//! Engine inputs and command results.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use dosker_core::ContainerId;
use serde::{Deserialize, Serialize};

/// How the guest display is exposed to the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    /// No display is attached.
    #[default]
    None,
    /// A local window shows the guest display.
    Window,
    /// A VNC server exposes the guest display.
    Vnc,
}

impl DisplayMode {
    /// Returns the serialized name of the display mode.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Window => "window",
            Self::Vnc => "vnc",
        }
    }

    /// Parses a display mode name.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "window" => Some(Self::Window),
            "vnc" => Some(Self::Vnc),
            _ => None,
        }
    }
}

/// The kind of debugger endpoint an engine can expose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebugKind {
    /// A GDB remote target.
    Gdb,
}

/// The pinned inputs an adapter needs to launch an engine for one container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineSpec {
    pub root: PathBuf,
    pub container_id: ContainerId,
    pub workdir: String,
    pub env: BTreeMap<String, String>,
    pub cmd: Option<String>,
    pub expects_agent: bool,
    pub log_dir: PathBuf,
    pub disk_path: PathBuf,
    pub booter: bool,
    pub display: DisplayMode,
    pub vnc_port: Option<u16>,
}

/// A resolved engine command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl EngineCommand {
    /// Renders the program and arguments as one command line.
    ///
    /// Only empty values, whitespace, and quote characters are quoted. Other
    /// shell metacharacters are left unchanged.
    pub fn render(&self) -> String {
        let mut parts = Vec::with_capacity(self.args.len() + 1);
        parts.push(quote(self.program.to_string_lossy().as_ref()));
        for argument in &self.args {
            parts.push(quote(argument));
        }
        parts.join(" ")
    }
}

fn quote(value: &str) -> String {
    if value.is_empty() {
        return String::from("''");
    }
    if value
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '\'' | '"'))
    {
        format!("'{}'", value.replace('\'', "'\\''"))
    } else {
        value.to_owned()
    }
}

/// How an engine endpoint is reachable from the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineEndpointKind {
    /// A Unix domain socket path.
    UnixSocket(PathBuf),
    /// A TCP address on the loopback interface.
    Tcp { host: Ipv4Addr, port: u16 },
}

impl EngineEndpointKind {
    /// Returns the Unix socket path, when the endpoint is a Unix socket.
    pub fn unix_path(&self) -> Option<&Path> {
        match self {
            Self::UnixSocket(path) => Some(path),
            Self::Tcp { .. } => None,
        }
    }

    /// Returns the TCP address, when the endpoint is TCP.
    pub fn tcp_addr(&self) -> Option<SocketAddr> {
        match self {
            Self::UnixSocket(_) => None,
            Self::Tcp { host, port } => Some(SocketAddr::from((*host, *port))),
        }
    }

    /// Renders the endpoint for a person.
    pub fn display(&self) -> String {
        match self {
            Self::UnixSocket(path) => path.display().to_string(),
            Self::Tcp { host, port } => format!("{host}:{port}"),
        }
    }
}

/// A named endpoint an engine exposes to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineEndpoint {
    pub name: String,
    pub kind: EngineEndpointKind,
}

/// A command to run inside the guest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecRequest {
    pub command: String,
    pub env: BTreeMap<String, String>,
    pub workdir: String,
    pub timeout_ms: u64,
}

/// The outcome of a guest command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecResult {
    pub exit_code: i32,
    pub output: String,
    pub timed_out: bool,
    pub truncated: bool,
}
