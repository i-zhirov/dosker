//! Control transport between the CLI and a container supervisor.
//!
//! A Unix domain socket carries the control stream on Linux and macOS. A
//! named pipe carries it on Windows.

use std::io::{self, Read, Write};
use std::time::Duration;

use crate::endpoint::Endpoint;
use crate::error::PlatformError;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::UnixConnection;

#[cfg(windows)]
pub use windows::PipeConnection;

/// A connected control endpoint.
pub trait ControlConnection: Read + Write + Send {
    /// Sets the read timeout, or clears it with `None`.
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;

    /// Sets the write timeout, or clears it with `None`.
    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;

    /// Shuts the connection down in both directions.
    fn shutdown(&mut self) -> io::Result<()>;
}

/// A listener that hands out control connections.
pub trait ControlListener: Send {
    /// Waits up to `timeout` for a client and returns `None` when none arrives.
    fn accept(&self, timeout: Duration) -> io::Result<Option<Box<dyn ControlConnection>>>;
}

/// Binds a listener to an endpoint.
#[cfg(unix)]
pub fn bind(endpoint: &Endpoint) -> Result<Box<dyn ControlListener>, PlatformError> {
    unix::bind(endpoint)
}

/// Connects to an endpoint, retrying until the timeout expiry.
#[cfg(unix)]
pub fn connect(
    endpoint: &Endpoint,
    timeout: Duration,
) -> Result<Box<dyn ControlConnection>, PlatformError> {
    unix::connect(endpoint, timeout)
}

/// Reports whether a listener answers at the endpoint.
#[cfg(unix)]
pub fn endpoint_alive(endpoint: &Endpoint) -> bool {
    unix::endpoint_alive(endpoint)
}

/// Removes the endpoint's on-disk artifact, if it has one.
#[cfg(unix)]
pub fn remove_endpoint(endpoint: &Endpoint) -> Result<(), PlatformError> {
    unix::remove_endpoint(endpoint)
}

/// Binds a listener to an endpoint.
#[cfg(windows)]
pub fn bind(endpoint: &Endpoint) -> Result<Box<dyn ControlListener>, PlatformError> {
    windows::bind(endpoint)
}

/// Connects to an endpoint, retrying until the timeout expiry.
#[cfg(windows)]
pub fn connect(
    endpoint: &Endpoint,
    timeout: Duration,
) -> Result<Box<dyn ControlConnection>, PlatformError> {
    windows::connect(endpoint, timeout)
}

/// Reports whether a listener answers at the endpoint.
#[cfg(windows)]
pub fn endpoint_alive(endpoint: &Endpoint) -> bool {
    windows::endpoint_alive(endpoint)
}

/// Removes the endpoint's on-disk artifact, if it has one.
#[cfg(windows)]
pub fn remove_endpoint(endpoint: &Endpoint) -> Result<(), PlatformError> {
    windows::remove_endpoint(endpoint)
}
