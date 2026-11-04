//! QEMU binary discovery and version parsing.

use std::env;
use std::ffi::OsString;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use dosker_engine::EngineError;

/// The environment variable that overrides QEMU discovery.
pub const QEMU_ENV: &str = "DOSKER_QEMU";

/// The QEMU program name used when no override is set.
pub const DEFAULT_QEMU: &str = "qemu-system-i386";

/// The maximum time a version probe may take.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

const PROBE_POLL: Duration = Duration::from_millis(20);
const VERSION_MARKER: &str = "QEMU emulator version ";

/// A three-part QEMU version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct QemuVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl QemuVersion {
    /// Creates a version from its three parts.
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parses the first `QEMU emulator version X.Y.Z` token in a line.
    pub fn parse(line: &str) -> Option<Self> {
        let rest = line.split_once(VERSION_MARKER)?.1;
        let token = rest.split_whitespace().next()?;
        let mut parts = token.split('.');

        Some(Self {
            major: parse_component(parts.next()?)?,
            minor: parse_component(parts.next()?)?,
            patch: parse_component(parts.next()?)?,
        })
    }
}

impl fmt::Display for QemuVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_component(value: &str) -> Option<u32> {
    let digits: String = value
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

/// A discovered QEMU binary and its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QemuBinary {
    pub path: PathBuf,
    pub version: QemuVersion,
}

impl QemuBinary {
    /// Finds QEMU through the environment override or `PATH`.
    pub fn discover() -> Result<Self, EngineError> {
        let requested = match env::var_os(QEMU_ENV) {
            Some(value) if !value.is_empty() => value,
            _ => OsString::from(DEFAULT_QEMU),
        };
        let path = resolve_program(&requested)?;

        Self::from_path(path)
    }

    /// Probes one binary path for its version.
    pub fn from_path(path: PathBuf) -> Result<Self, EngineError> {
        let output = run_version(&path, PROBE_TIMEOUT)?;
        let version = QemuVersion::parse(&output).ok_or_else(|| {
            EngineError::Setup(format!(
                "could not parse the QEMU version from {}",
                path.display()
            ))
        })?;

        Ok(Self { path, version })
    }

    /// Returns the version as text.
    pub fn version_string(&self) -> String {
        self.version.to_string()
    }
}

fn resolve_program(requested: &OsString) -> Result<PathBuf, EngineError> {
    let value = requested.to_string_lossy();
    if has_path_separator(&value) {
        let path = PathBuf::from(requested);
        if path.is_absolute() {
            Ok(path)
        } else {
            let current_dir = env::current_dir().map_err(EngineError::Io)?;
            Ok(current_dir.join(path))
        }
    } else {
        let program = Path::new(requested);
        let search = env::var_os("PATH").unwrap_or_default();
        for directory in env::split_paths(&search) {
            let candidate = directory.join(program);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }

        Err(EngineError::NotFound(PathBuf::from(requested)))
    }
}

fn has_path_separator(value: &str) -> bool {
    value.contains('/') || value.contains('\\')
}

fn run_version(path: &Path, timeout: Duration) -> Result<String, EngineError> {
    let mut child = match Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(EngineError::NotFound(path.to_path_buf()));
        }
        Err(error) => {
            return Err(EngineError::Setup(format!(
                "failed to start {}: {error}",
                path.display()
            )));
        }
    };

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(error) => return Err(EngineError::Io(error)),
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(EngineError::Setup("qemu --version timed out".to_owned()));
        }

        std::thread::sleep(PROBE_POLL);
    }

    let output = child.wait_with_output().map_err(EngineError::Io)?;
    if !output.status.success() {
        return Err(EngineError::Setup(format!(
            "{} --version exited with {}",
            path.display(),
            output.status
        )));
    }

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}
