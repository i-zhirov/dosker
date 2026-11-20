//! DOSBox-X binary discovery and version parsing.

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use dosker_engine::EngineError;

/// The environment variable that overrides DOSBox-X discovery.
pub const DOSBOX_X_ENV: &str = "DOSKER_DOSBOX_X";

/// The environment variable that pins the exact DOSBox-X version.
pub const DOSBOX_X_VERSION_ENV: &str = "DOSKER_DOSBOX_X_VERSION";

/// The environment variable that pins the DOSBox-X executable checksum.
pub const DOSBOX_X_SHA256_ENV: &str = "DOSKER_DOSBOX_X_SHA256";

/// The DOSBox-X program name used when no override is set.
pub const DEFAULT_DOSBOX_X: &str = "dosbox-x";

/// The maximum time a version probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

const PROBE_POLL: Duration = Duration::from_millis(20);
const CHECKSUM_BUFFER_BYTES: usize = 64 * 1024;
const VERSION_MARKER: &str = "DOSBox-X version ";

/// A discovered DOSBox-X binary and its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DosboxXBinary {
    pub path: PathBuf,
    pub version: String,
}

impl DosboxXBinary {
    /// Finds DOSBox-X through the environment override or `PATH`.
    pub fn discover() -> Result<Self, EngineError> {
        let requested = match env::var_os(DOSBOX_X_ENV) {
            Some(value) if !value.is_empty() => value,
            _ => OsString::from(DEFAULT_DOSBOX_X),
        };
        let path = resolve_program(&requested)?;

        Self::from_path(&path)
    }

    /// Probes one binary path for its version.
    pub fn from_path(path: &Path) -> Result<Self, EngineError> {
        if !path.is_file() {
            return Err(EngineError::NotFound(path.to_path_buf()));
        }

        if let Some(expected) = env_pin(DOSBOX_X_SHA256_ENV) {
            verify_checksum(path, &expected)?;
        }

        let output = run_version(path, PROBE_TIMEOUT)?;
        let version = parse_version(&output).ok_or_else(|| {
            EngineError::Setup(format!(
                "could not parse the DOSBox-X version from {}",
                path.display()
            ))
        })?;

        let pinned = env_pin(DOSBOX_X_VERSION_ENV);
        check_version(&version, pinned.as_deref())?;

        Ok(Self {
            path: path.to_path_buf(),
            version,
        })
    }

    /// Returns the version as text.
    pub fn version_string(&self) -> &str {
        &self.version
    }

    /// Returns the binary path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn parse_version(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        line.split_once(VERSION_MARKER)
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .map(str::to_owned)
    })
}

fn resolve_program(requested: &OsString) -> Result<PathBuf, EngineError> {
    let value = requested.to_string_lossy();
    if has_path_separator(&value) {
        let path = PathBuf::from(requested);
        let file_name = path
            .file_name()
            .map(OsStr::to_os_string)
            .ok_or_else(|| EngineError::NotFound(path.clone()))?;
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let directory = if parent.as_os_str().is_empty() || parent.is_absolute() {
            parent.to_path_buf()
        } else {
            env::current_dir().map_err(EngineError::Io)?.join(parent)
        };

        for candidate in program_candidates(&file_name) {
            let candidate = directory.join(candidate);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }

        Err(EngineError::NotFound(PathBuf::from(requested)))
    } else {
        let programs = program_candidates(requested);
        let search = env::var_os("PATH").unwrap_or_default();
        for directory in env::split_paths(&search) {
            for program in &programs {
                let candidate = directory.join(program);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }

        Err(EngineError::NotFound(PathBuf::from(requested)))
    }
}

#[cfg(windows)]
fn program_candidates(name: &OsStr) -> Vec<OsString> {
    if Path::new(name).extension().is_some() {
        return vec![name.to_os_string()];
    }

    let mut suffixes = Vec::new();
    if let Some(pathext) = env::var_os("PATHEXT") {
        for suffix in pathext.to_string_lossy().split(';') {
            let suffix = suffix.trim();
            if !suffix.is_empty()
                && !suffixes
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(suffix))
            {
                suffixes.push(suffix.to_ascii_uppercase());
            }
        }
    }
    if !suffixes
        .iter()
        .any(|suffix| suffix.eq_ignore_ascii_case(".EXE"))
    {
        suffixes.push(".EXE".to_owned());
    }

    suffixes
        .into_iter()
        .map(|suffix| OsString::from(format!("{}{suffix}", name.to_string_lossy())))
        .collect()
}

#[cfg(not(windows))]
fn program_candidates(name: &OsStr) -> Vec<OsString> {
    vec![name.to_os_string()]
}

fn env_pin(variable: &str) -> Option<String> {
    env::var(variable).ok().filter(|value| !value.is_empty())
}

fn file_sha256(path: &Path) -> Result<String, EngineError> {
    let mut file = std::fs::File::open(path).map_err(EngineError::Io)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; CHECKSUM_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).map_err(EngineError::Io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(lowercase_hex(&hasher.finalize()))
}

fn verify_checksum(path: &Path, expected: &str) -> Result<(), EngineError> {
    let actual = file_sha256(path)?;
    if actual == expected.to_ascii_lowercase() {
        Ok(())
    } else {
        Err(EngineError::Setup(format!(
            "DOSBox-X checksum mismatch for {}: expected {}, found {}",
            path.display(),
            expected.to_ascii_lowercase(),
            actual
        )))
    }
}

fn check_version(version: &str, pinned: Option<&str>) -> Result<(), EngineError> {
    if let Some(pinned) = pinned
        && version != pinned
    {
        return Err(EngineError::Setup(format!(
            "DOSBox-X version mismatch: expected {pinned}, found {version}"
        )));
    }

    Ok(())
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }

    output
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
            return Err(EngineError::Setup(
                "dosbox-x --version timed out".to_owned(),
            ));
        }

        std::thread::sleep(PROBE_POLL);
    }

    let output = child.wait_with_output().map_err(EngineError::Io)?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    // The Homebrew build prints the version line and then exits non-zero, so
    // the status alone does not prove the probe failed.
    if !output.status.success() && !text.contains(VERSION_MARKER) {
        return Err(EngineError::Setup(format!(
            "{} --version exited with {}",
            path.display(),
            output.status
        )));
    }

    Ok(text)
}

#[cfg(all(test, windows))]
mod tests {
    use std::env;
    use std::ffi::{OsStr, OsString};

    use super::program_candidates;

    struct EnvRestore {
        pathext: Option<OsString>,
    }

    impl EnvRestore {
        fn capture() -> Self {
            Self {
                pathext: env::var_os("PATHEXT"),
            }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            unsafe {
                match &self.pathext {
                    Some(value) => env::set_var("PATHEXT", value),
                    None => env::remove_var("PATHEXT"),
                }
            }
        }
    }

    fn candidate_names() -> Vec<String> {
        program_candidates(OsStr::new("dosbox-x"))
            .iter()
            .map(|candidate| candidate.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn program_candidates_pins_the_pathext_list() {
        let _restore = EnvRestore::capture();

        unsafe {
            env::set_var("PATHEXT", ".COM;.EXE;.BAT");
        }
        assert_eq!(
            candidate_names(),
            ["dosbox-x.COM", "dosbox-x.EXE", "dosbox-x.BAT"]
        );

        unsafe {
            env::set_var("PATHEXT", ".COM;.BAT");
        }
        assert_eq!(
            candidate_names(),
            ["dosbox-x.COM", "dosbox-x.BAT", "dosbox-x.EXE"]
        );
    }
}
