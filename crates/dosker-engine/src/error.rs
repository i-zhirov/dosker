//! Engine adapter errors.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

/// An error raised while probing, starting, or controlling an engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("engine not found at {}", .0.display())]
    NotFound(PathBuf),
    #[error("engine setup failed: {0}")]
    Setup(String),
    #[error("engine protocol error: {0}")]
    Protocol(String),
    #[error("engine did not become ready within {0:?}")]
    ReadyTimeout(Duration),
    #[error("command timed out after {0:?}")]
    ExecTimeout(Duration),
    #[error("engine did not stop within {0:?}")]
    StopTimeout(Duration),
    #[error("engine exited with code {0}")]
    Exited(i32),
    #[error("the image has no agent")]
    NoAgent,
    #[error("unsupported engine feature: {0}")]
    Unsupported(&'static str),
    #[error("the guest rebooted during the command")]
    GuestRebooted,
    #[error("agent error {code}: {message}")]
    Agent { code: String, message: String },
}
