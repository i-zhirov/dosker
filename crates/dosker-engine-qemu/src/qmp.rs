//! A minimal QEMU machine protocol client.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use dosker_engine::EngineError;
use serde_json::{Map, Value};

/// The time allowed for a QMP connection and handshake.
pub const QMP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A JSON line client for the QEMU machine protocol.
pub struct QmpClient {
    reader: BufReader<UnixStream>,
}

impl QmpClient {
    /// Connects to a QMP socket, reads the greeting, and enables capabilities.
    ///
    /// The connect itself is non-blocking so a QEMU that stops accepting
    /// connections cannot hold the caller past `timeout`. Blocking mode and
    /// the read and write timeouts are restored before the handshake.
    pub fn connect(path: &Path, timeout: Duration) -> Result<Self, EngineError> {
        Self::connect_until(path, Instant::now() + timeout)
    }

    /// Connects to a QMP socket, finishing the handshake before `deadline`.
    ///
    /// This is the deadline-aware form used by a caller that has already
    /// captured an overall deadline, such as `QemuSession::stop`.
    pub fn connect_until(path: &Path, deadline: Instant) -> Result<Self, EngineError> {
        let stream = connect_stream(path, deadline)?;

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(connect_timeout());
        }

        // Keep the handshake inside the same deadline, but never pass a
        // sub-millisecond duration that the OS may round down to "no timeout".
        let handshake_timeout = remaining.max(Duration::from_millis(1));
        stream.set_read_timeout(Some(handshake_timeout))?;
        stream.set_write_timeout(Some(handshake_timeout))?;

        let mut client = Self {
            reader: BufReader::new(stream),
        };

        match client.read_object()? {
            Some(_greeting) => {}
            None => {
                return Err(EngineError::Protocol(
                    "QMP closed the connection before the greeting".to_owned(),
                ));
            }
        }

        client.execute("qmp_capabilities", None)?;
        Ok(client)
    }

    /// Restricts subsequent reads and writes to `timeout`.
    ///
    /// Connecting leaves the socket at the handshake deadline, so a command
    /// that runs later (for example `quit` after an agent shutdown) must
    /// refresh the I/O budget before it runs.
    pub fn set_io_timeout(&mut self, timeout: Duration) -> Result<(), EngineError> {
        let timeout = timeout.max(Duration::from_millis(1));
        let stream = self.reader.get_mut();
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(())
    }

    /// Runs one QMP command and returns its `return` value.
    pub fn execute(
        &mut self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<Value, EngineError> {
        self.send_command(command, arguments)?;

        loop {
            let value = self.read_object()?.ok_or_else(|| {
                EngineError::Protocol(format!("QMP closed the connection during {command}"))
            })?;

            if value.get("event").is_some() {
                continue;
            }

            if let Some(error) = value.get("error") {
                return Err(EngineError::Protocol(format!(
                    "QMP {command} failed: {}",
                    describe_error(error)
                )));
            }

            if let Some(result) = value.get("return") {
                return Ok(result.clone());
            }

            return Err(EngineError::Protocol(format!(
                "unexpected QMP response to {command}: {value}"
            )));
        }
    }

    /// Sends `quit` and accepts an EOF as a successful exit.
    pub fn quit(&mut self) -> Result<(), EngineError> {
        if let Err(error) = self.send_command("quit", None) {
            return if is_closed(&error) {
                Ok(())
            } else {
                Err(error)
            };
        }

        loop {
            match self.read_object() {
                Ok(Some(value)) => {
                    if value.get("event").is_some() {
                        continue;
                    }

                    if let Some(error) = value.get("error") {
                        return Err(EngineError::Protocol(format!(
                            "QMP quit failed: {}",
                            describe_error(error)
                        )));
                    }

                    return Ok(());
                }
                Ok(None) => return Ok(()),
                Err(error) if is_closed(&error) => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    }

    /// Captures the guest screen to a file.
    pub fn screendump(&mut self, path: &Path, format: Option<&str>) -> Result<(), EngineError> {
        let mut arguments = Map::new();
        arguments.insert(
            "filename".to_owned(),
            Value::String(path.to_string_lossy().into_owned()),
        );
        if let Some(format) = format {
            arguments.insert("format".to_owned(), Value::String(format.to_owned()));
        }

        self.execute("screendump", Some(Value::Object(arguments)))?;
        Ok(())
    }

    fn send_command(&mut self, command: &str, arguments: Option<Value>) -> Result<(), EngineError> {
        let mut object = Map::new();
        object.insert("execute".to_owned(), Value::String(command.to_owned()));
        if let Some(arguments) = arguments {
            object.insert("arguments".to_owned(), arguments);
        }

        let mut encoded = serde_json::to_vec(&Value::Object(object))
            .map_err(|error| EngineError::Protocol(format!("encode QMP command: {error}")))?;
        encoded.push(b'\n');

        let stream = self.reader.get_mut();
        stream.write_all(&encoded)?;
        stream.flush()?;
        Ok(())
    }

    fn read_object(&mut self) -> Result<Option<Value>, EngineError> {
        let mut line = String::new();
        let count = self.reader.read_line(&mut line)?;
        if count == 0 {
            return Ok(None);
        }

        let value = serde_json::from_str(line.trim())
            .map_err(|error| EngineError::Protocol(format!("invalid QMP response: {error}")))?;
        Ok(Some(value))
    }
}

/// Connects to a Unix socket without blocking past `deadline`.
fn connect_stream(path: &Path, deadline: Instant) -> Result<UnixStream, EngineError> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };

    if bytes.len() >= address.sun_path.len() {
        return Err(EngineError::Setup(format!(
            "the QMP socket path is too long: {}",
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
        "the QMP connection timed out",
    ))
}

fn describe_error(error: &Value) -> String {
    error
        .get("desc")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string())
}

fn is_closed(error: &EngineError) -> bool {
    let EngineError::Io(error) = error else {
        return false;
    };

    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    )
}

#[cfg(test)]
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
