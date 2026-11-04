//! A serial transport that records every byte to a log file.
//!
//! Logging is best-effort. The serial link carries the agent protocol, so a
//! failure to append to the log must not fail a read or a write that already
//! moved protocol bytes. The first logging failure is reported to stderr and
//! further logging is skipped, keeping the failure visible without disturbing
//! the protocol.

use std::fs::File;
use std::io::{self, Read, Write};
use std::time::Duration;

use dosker_agent_proto::Transport;

/// A transport wrapper that appends every transferred byte to a log.
pub struct SerialLog<T> {
    inner: T,
    log: File,
    log_failed: bool,
}

impl<T> SerialLog<T> {
    /// Wraps a transport and writes its traffic to `log`.
    pub fn new(inner: T, log: File) -> Self {
        Self {
            inner,
            log,
            log_failed: false,
        }
    }

    fn record(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.log_failed {
            return;
        }

        if let Err(error) = self.log.write_all(bytes).and_then(|()| self.log.flush()) {
            self.log_failed = true;
            eprintln!("dosker: cannot write the serial log: {error}");
        }
    }
}

impl<T: Read> Read for SerialLog<T> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.record(&buffer[..count]);

        Ok(count)
    }
}

impl<T: Write> Write for SerialLog<T> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(buffer)?;
        self.record(&buffer[..count]);

        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()?;

        if !self.log_failed
            && let Err(error) = self.log.flush()
        {
            self.log_failed = true;
            eprintln!("dosker: cannot write the serial log: {error}");
        }

        Ok(())
    }
}

impl<T: Transport> Transport for SerialLog<T> {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.inner.set_read_timeout(timeout)
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.inner.set_write_timeout(timeout)
    }
}
