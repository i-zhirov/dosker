//! Unix domain socket control transport.

use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::endpoint::Endpoint;
use crate::error::PlatformError;
use crate::fs::ensure_dir_mode;

use super::{ControlConnection, ControlListener};

const RETRY_INTERVAL: Duration = Duration::from_millis(20);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const STALE_PROBE_BUDGET: Duration = Duration::from_millis(100);
const ALIVE_PROBE_BUDGET: Duration = Duration::from_millis(200);
const SOCKET_FILE_MODE: u32 = 0o600;
const SOCKET_DIR_MODE: u32 = 0o700;

/// A control connection backed by a Unix domain socket.
pub struct UnixConnection {
    stream: UnixStream,
}

impl UnixConnection {
    fn new(stream: UnixStream) -> Self {
        Self { stream }
    }
}

impl Read for UnixConnection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl Write for UnixConnection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl ControlConnection for UnixConnection {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_read_timeout(timeout)
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_write_timeout(timeout)
    }

    fn shutdown(&mut self) -> io::Result<()> {
        self.stream.shutdown(Shutdown::Both)
    }
}

struct UnixListenerHandle {
    connections: Receiver<io::Result<Box<dyn ControlConnection>>>,
}

impl ControlListener for UnixListenerHandle {
    fn accept(&self, timeout: Duration) -> io::Result<Option<Box<dyn ControlConnection>>> {
        match self.connections.recv_timeout(timeout) {
            Ok(connection) => connection.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "control acceptor stopped",
            )),
        }
    }
}

pub fn bind(endpoint: &Endpoint) -> Result<Box<dyn ControlListener>, PlatformError> {
    let path = socket_path(endpoint)?.to_path_buf();
    let parent = path
        .parent()
        .ok_or_else(|| PlatformError::InvalidEndpoint(path.display().to_string()))?
        .to_path_buf();
    ensure_dir_mode(&parent, SOCKET_DIR_MODE)?;

    if path.exists() && connect(endpoint, STALE_PROBE_BUDGET).is_err() {
        remove_socket_file(&path)?;
    }

    let listener = UnixListener::bind(&path).map_err(|source| PlatformError::Io {
        path: path.clone(),
        source,
    })?;
    fs::set_permissions(&path, fs::Permissions::from_mode(SOCKET_FILE_MODE)).map_err(|source| {
        PlatformError::Io {
            path: path.clone(),
            source,
        }
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|source| PlatformError::Io {
            path: path.clone(),
            source,
        })?;

    let (sender, connections) = mpsc::channel();
    thread::spawn(move || accept_loop(listener, sender));

    Ok(Box::new(UnixListenerHandle { connections }))
}

pub fn connect(
    endpoint: &Endpoint,
    timeout: Duration,
) -> Result<Box<dyn ControlConnection>, PlatformError> {
    let path = socket_path(endpoint)?;
    let deadline = Instant::now() + timeout;

    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(Box::new(UnixConnection::new(stream))),
            Err(source) if is_retryable(source.kind()) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(PlatformError::Io {
                        path: path.to_path_buf(),
                        source,
                    });
                }
                let remaining = deadline.saturating_duration_since(now);
                thread::sleep(RETRY_INTERVAL.min(remaining));
            }
            Err(source) => {
                return Err(PlatformError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
    }
}

pub fn endpoint_alive(endpoint: &Endpoint) -> bool {
    connect(endpoint, ALIVE_PROBE_BUDGET).is_ok()
}

pub fn remove_endpoint(endpoint: &Endpoint) -> Result<(), PlatformError> {
    remove_socket_file(socket_path(endpoint)?)
}

fn accept_loop(listener: UnixListener, sender: Sender<io::Result<Box<dyn ControlConnection>>>) {
    loop {
        match listener.accept() {
            Ok((stream, _address)) => {
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let connection: Box<dyn ControlConnection> = Box::new(UnixConnection::new(stream));
                if sender.send(Ok(connection)).is_err() {
                    break;
                }
            }
            Err(source) => match source.kind() {
                io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL_INTERVAL),
                io::ErrorKind::Interrupted => {}
                _ => {
                    let _ = sender.send(Err(source));
                    break;
                }
            },
        }
    }
}

fn remove_socket_file(path: &Path) -> Result<(), PlatformError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(PlatformError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn is_retryable(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn socket_path(endpoint: &Endpoint) -> Result<&Path, PlatformError> {
    match endpoint {
        Endpoint::UnixSocket(path) => Ok(path),
        Endpoint::NamedPipe(name) => Err(PlatformError::InvalidEndpoint(format!(
            "named pipe {name} on a unix host"
        ))),
    }
}
