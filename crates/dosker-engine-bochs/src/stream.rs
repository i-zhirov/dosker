//! Host engine transport.
//!
//! Bochs serial endpoints are loopback TCP sockets. This module keeps the
//! connect behavior in one place so the agent client has one transport type.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use dosker_agent_proto::Transport;
use dosker_engine::{EngineEndpointKind, EngineError};

/// How long one TCP connection attempt may take before it is retried.
const CONNECT_RETRY: Duration = Duration::from_millis(20);

/// A connected engine channel.
#[derive(Debug)]
pub(crate) struct BochsStream(TcpStream);

impl Read for BochsStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for BochsStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Transport for BochsStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_read_timeout(timeout)
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_write_timeout(timeout)
    }
}

/// Connects to an engine endpoint, finishing before `deadline`.
///
/// A TCP endpoint retries `TcpStream::connect_timeout` every 20 milliseconds
/// until the deadline, so a server that is still starting does not fail the
/// attempt early. Bochs listens on the serial line, so a Unix socket endpoint
/// cannot be used for the agent.
pub(crate) fn connect(
    kind: &EngineEndpointKind,
    deadline: Instant,
) -> Result<BochsStream, EngineError> {
    match kind {
        EngineEndpointKind::UnixSocket(_) => Err(EngineError::Setup(
            "the Bochs agent serial endpoint must be TCP".to_owned(),
        )),
        EngineEndpointKind::Tcp { host, port } => {
            tcp_connect(SocketAddr::from((*host, *port)), deadline).map(BochsStream)
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

fn connect_timeout() -> EngineError {
    EngineError::Io(io::Error::new(
        io::ErrorKind::TimedOut,
        "the engine connection timed out",
    ))
}
