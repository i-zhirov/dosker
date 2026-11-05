//! Host engine transport.
//!
//! Engine channels are Unix sockets on Unix and loopback TCP on Windows. This
//! module keeps the connect, probe, and timeout behavior in one place so the
//! QMP client and the serial agent share it.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use dosker_agent_proto::Transport;
use dosker_engine::{EngineEndpointKind, EngineError};

/// How long one TCP connection attempt may take before it is retried.
const TCP_CONNECT_RETRY: Duration = Duration::from_millis(20);

/// A connected engine channel.
#[derive(Debug)]
pub enum EngineStream {
    /// A Unix domain socket.
    #[cfg(unix)]
    Unix(UnixStream),
    /// A loopback TCP socket.
    Tcp(TcpStream),
}

impl Read for EngineStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
            Self::Tcp(stream) => stream.read(buffer),
        }
    }
}

impl Write for EngineStream {
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

impl Transport for EngineStream {
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

impl EngineStream {
    /// Connects to an engine endpoint, finishing before `deadline`.
    ///
    /// A Unix socket uses the non-blocking connect and writability poll
    /// inherited from the QMP client. A TCP endpoint retries
    /// `TcpStream::connect_timeout` every 20 milliseconds until the deadline,
    /// so a server that is still starting does not fail the attempt early.
    pub fn connect(
        kind: &EngineEndpointKind,
        deadline: Instant,
    ) -> Result<EngineStream, EngineError> {
        match kind {
            EngineEndpointKind::UnixSocket(path) => {
                #[cfg(unix)]
                {
                    unix_connect(path, deadline).map(EngineStream::Unix)
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
                tcp_connect(SocketAddr::from((*host, *port)), deadline).map(EngineStream::Tcp)
            }
        }
    }

    /// Reports whether an engine endpoint accepts a connection right now.
    ///
    /// A Unix endpoint is probed by its socket path. A TCP endpoint is probed
    /// with a bounded connect attempt.
    pub fn probe(kind: &EngineEndpointKind, timeout: Duration) -> bool {
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
}

fn tcp_connect(address: SocketAddr, deadline: Instant) -> Result<TcpStream, EngineError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(connect_timeout());
        }

        let attempt = remaining.min(TCP_CONNECT_RETRY);
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

                std::thread::sleep(TCP_CONNECT_RETRY.min(remaining));
            }
        }
    }
}

/// Connects to a Unix socket without blocking past `deadline`.
#[cfg(unix)]
fn unix_connect(path: &Path, deadline: Instant) -> Result<UnixStream, EngineError> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };

    if bytes.len() >= address.sun_path.len() {
        return Err(EngineError::Setup(format!(
            "the engine socket path is too long: {}",
            path.display()
        )));
    }

    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        address.sun_len = std::mem::size_of::<libc::sockaddr_un>() as u8;
    }

    let descriptor = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if descriptor < 0 {
        return Err(EngineError::Io(io::Error::last_os_error()));
    }
    let stream = unsafe { UnixStream::from_raw_fd(descriptor) };
    stream.set_nonblocking(true)?;

    let result = unsafe {
        libc::connect(
            stream.as_raw_fd(),
            &address as *const libc::sockaddr_un as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };

    if result != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS)
            && error.raw_os_error() != Some(libc::EALREADY)
        {
            return Err(EngineError::Io(error));
        }

        wait_writable(stream.as_raw_fd(), deadline)?;

        let mut socket_error: libc::c_int = 0;
        let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let status = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                &mut socket_error as *mut libc::c_int as *mut libc::c_void,
                &mut length,
            )
        };
        if status != 0 {
            return Err(EngineError::Io(io::Error::last_os_error()));
        }
        if socket_error != 0 {
            return Err(EngineError::Io(io::Error::from_raw_os_error(socket_error)));
        }
    }

    stream.set_nonblocking(false)?;
    Ok(stream)
}

/// Waits until a socket is writable or the deadline passes.
#[cfg(unix)]
fn wait_writable(descriptor: RawFd, deadline: Instant) -> Result<(), EngineError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(connect_timeout());
        }

        let millis = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
        let mut polled = libc::pollfd {
            fd: descriptor,
            events: libc::POLLOUT,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut polled, 1, millis) };

        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(EngineError::Io(error));
        }

        if result > 0 {
            return Ok(());
        }
    }
}

fn connect_timeout() -> EngineError {
    EngineError::Io(io::Error::new(
        io::ErrorKind::TimedOut,
        "the engine connection timed out",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn wait_writable_stops_at_the_deadline() {
        let mut descriptors = [0 as libc::c_int; 2];
        assert_eq!(
            unsafe { libc::pipe(descriptors.as_mut_ptr()) },
            0,
            "create a pipe"
        );

        let started = Instant::now();
        let error = wait_writable(descriptors[0], Instant::now() + Duration::from_millis(50))
            .expect_err("the read end of a pipe never becomes writable");
        let elapsed = started.elapsed();

        unsafe {
            libc::close(descriptors[0]);
            libc::close(descriptors[1]);
        }

        match error {
            EngineError::Io(error) => assert_eq!(error.kind(), io::ErrorKind::TimedOut),
            other => panic!("expected an I/O timeout, got {other:?}"),
        }
        assert!(elapsed >= Duration::from_millis(50));
        assert!(elapsed < Duration::from_secs(1));
    }
}
