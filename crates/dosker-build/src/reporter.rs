//! Progress reporting for builds.

use std::io;

/// Receives build progress lines and command output.
pub trait BuildReporter {
    /// Writes one progress line.
    fn line(&mut self, text: &str) -> io::Result<()>;

    /// Writes raw command output.
    fn output(&mut self, bytes: &[u8]) -> io::Result<()>;
}
