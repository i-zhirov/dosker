//! Console output.

use std::io::{self, Write};

/// Writes user-facing output to stdout and stderr.
pub struct Console {
    stdout: Box<dyn Write + Send>,
    stderr: Box<dyn Write + Send>,
    #[cfg(unix)]
    stdout_fd: Option<std::os::unix::io::RawFd>,
}

impl Console {
    /// Creates a console that writes to the process standard streams.
    pub fn std() -> Self {
        #[cfg(unix)]
        {
            Self {
                stdout: Box::new(io::stdout()),
                stderr: Box::new(io::stderr()),
                stdout_fd: Some(libc::STDOUT_FILENO),
            }
        }
        #[cfg(not(unix))]
        {
            Self::new(Box::new(io::stdout()), Box::new(io::stderr()))
        }
    }

    /// Creates a console from explicit writers.
    pub fn new(stdout: Box<dyn Write + Send>, stderr: Box<dyn Write + Send>) -> Self {
        Self {
            stdout,
            stderr,
            #[cfg(unix)]
            stdout_fd: None,
        }
    }

    /// Reports whether stdout has hung up because its reader closed.
    ///
    /// Only a console created by [`Console::std`] has a pollable descriptor.
    /// Consoles built from explicit writers always report `false`, and
    /// non-Unix builds have no hang-up probe; in both cases a closed reader
    /// is still noticed by the write path when new bytes are flushed.
    ///
    /// A process started with stdout already closed (`>&-`) is not
    /// detectable here: the standard library reopens descriptors 0, 1, and 2
    /// on `/dev/null` before `main` runs, so the polled descriptor is valid.
    /// That case is out of scope; a reader that closes after startup is
    /// still detected.
    #[cfg(unix)]
    pub fn stdout_hung_up(&self) -> bool {
        let Some(fd) = self.stdout_fd else {
            return false;
        };
        let mut descriptor = libc::pollfd {
            fd,
            // macOS only reports a hang-up when it is requested, while Linux
            // reports a closed reader as `POLLERR`; ask for both.
            events: libc::POLLHUP | libc::POLLERR,
            revents: 0,
        };
        // SAFETY: `descriptor` points to one initialized `pollfd`.
        let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
        ready > 0 && descriptor.revents & (libc::POLLHUP | libc::POLLERR) != 0
    }

    /// Non-Unix builds cannot poll the standard output descriptor.
    #[cfg(not(unix))]
    pub fn stdout_hung_up(&self) -> bool {
        false
    }

    /// Writes one line to stdout and flushes.
    pub fn line(&mut self, text: &str) -> io::Result<()> {
        let result = (|| {
            self.stdout.write_all(text.as_bytes())?;
            self.stdout.write_all(b"\n")?;
            self.stdout.flush()
        })();

        ignore_broken_pipe(result)
    }

    /// Writes one line to stderr and flushes.
    pub fn error_line(&mut self, text: &str) -> io::Result<()> {
        let result = (|| {
            self.stderr.write_all(text.as_bytes())?;
            self.stderr.write_all(b"\n")?;
            self.stderr.flush()
        })();

        ignore_broken_pipe(result)
    }

    /// Writes raw bytes to stdout and flushes.
    pub fn bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        ignore_broken_pipe(self.try_bytes(bytes))
    }

    /// Writes raw bytes to stdout and flushes, reporting every failure.
    ///
    /// Unlike [`Console::bytes`], a broken pipe is returned to the caller so
    /// long-running writers such as `logs --follow` can stop.
    pub fn try_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stdout.write_all(bytes)?;
        self.stdout.flush()
    }

    /// Writes text to stdout without a newline and flushes.
    pub fn raw(&mut self, text: &str) -> io::Result<()> {
        let result = (|| {
            self.stdout.write_all(text.as_bytes())?;
            self.stdout.flush()
        })();

        ignore_broken_pipe(result)
    }
}

fn ignore_broken_pipe(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}
