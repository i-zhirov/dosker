//! Console output.

use std::io::{self, Write};

/// Writes user-facing output to stdout and stderr.
pub struct Console {
    stdout: Box<dyn Write + Send>,
    stderr: Box<dyn Write + Send>,
}

impl Console {
    /// Creates a console that writes to the process standard streams.
    pub fn std() -> Self {
        Self::new(Box::new(io::stdout()), Box::new(io::stderr()))
    }

    /// Creates a console from explicit writers.
    pub fn new(stdout: Box<dyn Write + Send>, stderr: Box<dyn Write + Send>) -> Self {
        Self { stdout, stderr }
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
