//! Host engine transport.
//!
//! DOSBox-X serial endpoints are loopback TCP sockets. This module keeps the
//! connect and probe behavior in one place so the agent client has one
//! transport type.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use dosker_agent_proto::Transport;
use dosker_engine::{EngineEndpointKind, EngineError};

/// How long one TCP connection attempt may take before it is retried.
const CONNECT_RETRY: Duration = Duration::from_millis(20);

/// A connected engine channel.
#[derive(Debug)]
pub(crate) enum DosboxXStream {
    /// A Unix domain socket.
    #[cfg(unix)]
    Unix(UnixStream),
    /// A loopback TCP socket.
    Tcp(TcpStream),
}

impl Read for DosboxXStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
            Self::Tcp(stream) => stream.read(buffer),
        }
    }
}

impl Write for DosboxXStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buffer),
            Self::Tcp(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

impl Transport for DosboxXStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_read_timeout(timeout),
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
        }
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_write_timeout(timeout),
            Self::Tcp(stream) => stream.set_write_timeout(timeout),
        }
    }
}

/// Connects to an engine endpoint, finishing before `deadline`.
///
/// A TCP endpoint retries `TcpStream::connect_timeout` every 20 milliseconds
/// until the deadline, so a server that is still starting does not fail the
/// attempt early. A Unix socket retries `UnixStream::connect` the same way.
pub(crate) fn connect(
    kind: &EngineEndpointKind,
    deadline: Instant,
) -> Result<DosboxXStream, EngineError> {
    match kind {
        EngineEndpointKind::UnixSocket(path) => {
            #[cfg(unix)]
            {
                unix_connect(path, deadline).map(DosboxXStream::Unix)
            }
            #[cfg(windows)]
            {
                let _ = path;
                Err(EngineError::Unsupported(
                    "Unix socket engine endpoints on Windows",
                ))
            }
        }
        EngineEndpointKind::Tcp { host, port } => {
            tcp_connect(SocketAddr::from((*host, *port)), deadline).map(DosboxXStream::Tcp)
        }
    }
}

/// Reports whether an engine endpoint accepts a connection right now.
#[allow(dead_code)]
pub(crate) fn probe(kind: &EngineEndpointKind, timeout: Duration) -> bool {
    match kind {
        EngineEndpointKind::UnixSocket(_) => kind.unix_path().is_some_and(Path::exists),
        EngineEndpointKind::Tcp { .. } => {
            let Some(address) = kind.tcp_addr() else {
                return false;
            };
            let timeout = timeout.max(Duration::from_millis(1));

            TcpStream::connect_timeout(&address, timeout).is_ok()
        }
    }
}

fn tcp_connect(address: SocketAddr, deadline: Instant) -> Result<TcpStream, EngineError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(connect_timeout());
        }

        let attempt = remaining.min(CONNECT_RETRY);
        match TcpStream::connect_timeout(&address, attempt) {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                return Ok(stream);
            }
            Err(_) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(connect_timeout());
                }

                thread::sleep(CONNECT_RETRY.min(remaining));
            }
        }
    }
}

#[cfg(unix)]
fn unix_connect(path: &Path, deadline: Instant) -> Result<UnixStream, EngineError> {
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(EngineError::Io(error));
                }

                thread::sleep(CONNECT_RETRY.min(remaining));
            }
        }
    }
}

fn connect_timeout() -> EngineError {
    EngineError::Io(io::Error::new(
        io::ErrorKind::TimedOut,
        "the engine connection timed out",
    ))
}
