//! QEMU binary discovery and version parsing.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use dosker_engine::EngineError;

/// The environment variable that overrides QEMU discovery.
pub const QEMU_ENV: &str = "DOSKER_QEMU";

/// The environment variable that pins the QEMU executable checksum.
pub const QEMU_SHA256_ENV: &str = "DOSKER_QEMU_SHA256";

/// The environment variable that pins the exact QEMU version.
pub const QEMU_VERSION_ENV: &str = "DOSKER_QEMU_VERSION";

/// The QEMU program name used when no override is set.
pub const DEFAULT_QEMU: &str = "qemu-system-i386";

/// The oldest QEMU build supported on Windows.
pub const MIN_QEMU_VERSION: QemuVersion = QemuVersion {
    major: 7,
    minor: 1,
    patch: 0,
};

/// The maximum time a version probe may take.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

const PROBE_POLL: Duration = Duration::from_millis(20);
const CHECKSUM_BUFFER_BYTES: usize = 64 * 1024;
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

    /// Parses an exact `major.minor.patch` version pin.
    ///
    /// Every component is decimal and the value has no prefix or suffix.
    pub fn parse_pin(value: &str) -> Option<Self> {
        let mut parts = value.split('.');
        let major = strict_component(parts.next()?)?;
        let minor = strict_component(parts.next()?)?;
        let patch = strict_component(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }

        Some(Self {
            major,
            minor,
            patch,
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

fn strict_component(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    value.parse().ok()
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
        if !path.is_file() {
            return Err(EngineError::NotFound(path));
        }

        if let Some(expected) = env_pin(QEMU_SHA256_ENV) {
            verify_checksum(&path, &expected)?;
        }

        let output = run_version(&path, PROBE_TIMEOUT)?;
        let version = QemuVersion::parse(&output).ok_or_else(|| {
            EngineError::Setup(format!(
                "could not parse the QEMU version from {}",
                path.display()
            ))
        })?;

        let pinned = env_pin(QEMU_VERSION_ENV);
        check_version(version, pinned.as_deref())?;

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

        #[cfg(windows)]
        {
            for candidate in windows_install_candidates(requested) {
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

#[cfg(windows)]
fn windows_install_candidates(name: &OsStr) -> Vec<PathBuf> {
    const ROOTS: [(&str, &str); 3] = [
        ("ProgramFiles", "qemu"),
        ("ProgramFiles(x86)", "qemu"),
        ("LOCALAPPDATA", r"Programs\qemu"),
    ];

    let programs = program_candidates(name);
    let mut candidates = Vec::new();
    for (variable, subdirectory) in ROOTS {
        let Some(root) = env::var_os(variable).filter(|root| !root.is_empty()) else {
            continue;
        };
        let directory = PathBuf::from(root).join(subdirectory);
        for program in &programs {
            candidates.push(directory.join(program));
        }
    }

    candidates
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
            "QEMU checksum mismatch for {}: expected {}, found {}",
            path.display(),
            expected.to_ascii_lowercase(),
            actual
        )))
    }
}

fn check_version(version: QemuVersion, pinned: Option<&str>) -> Result<(), EngineError> {
    if let Some(pinned) = pinned {
        let expected = QemuVersion::parse_pin(pinned).ok_or_else(|| {
            EngineError::Setup(format!(
                "invalid {QEMU_VERSION_ENV} value {pinned:?}: expected major.minor.patch"
            ))
        })?;

        if version != expected {
            return Err(EngineError::Setup(format!(
                "QEMU version mismatch: expected {expected}, found {version}"
            )));
        }
    }

    #[cfg(windows)]
    {
        if version < MIN_QEMU_VERSION {
            return Err(EngineError::Setup(format!(
                "QEMU {version} is older than the minimum supported version {MIN_QEMU_VERSION}"
            )));
        }
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

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::ffi::OsStr;

    use sha2::{Digest, Sha256};

    use super::*;

    #[cfg(windows)]
    #[test]
    fn minimum_version_rejects_an_older_build() {
        let error =
            check_version(QemuVersion::new(7, 0, 9), None).expect_err("an older build must fail");
        let EngineError::Setup(message) = error else {
            panic!("unexpected error: {error}");
        };
        assert!(message.contains("7.1.0"), "message: {message}");
    }

    #[cfg(windows)]
    #[test]
    fn minimum_version_accepts_the_floor() {
        check_version(QemuVersion::new(7, 1, 0), None).expect("the floor must pass");
    }

    #[test]
    fn version_pin_parses_a_bare_three_part_version() {
        assert_eq!(
            QemuVersion::parse_pin("8.2.0"),
            Some(QemuVersion {
                major: 8,
                minor: 2,
                patch: 0,
            })
        );
    }

    #[test]
    fn version_pin_rejects_a_prefix_or_suffix() {
        assert_eq!(QemuVersion::parse_pin("QEMU 8.2.0"), None);
        assert_eq!(QemuVersion::parse_pin("8.2.0-debian"), None);
    }

    #[test]
    fn version_pin_rejects_a_mismatch() {
        let error = check_version(QemuVersion::new(8, 2, 1), Some("8.2.0"))
            .expect_err("a pinned mismatch must fail");
        let EngineError::Setup(message) = error else {
            panic!("unexpected error: {error}");
        };
        assert!(message.contains("8.2.0"), "message: {message}");
        assert!(message.contains("8.2.1"), "message: {message}");
    }

    #[test]
    fn checksum_accepts_a_matching_file() {
        let dir = tempfile::tempdir().expect("create temp directory");
        let path = dir.path().join("qemu.bin");
        std::fs::write(&path, b"compiled qemu").expect("write test file");
        let expected = lowercase_hex(&Sha256::digest(b"compiled qemu"));

        verify_checksum(&path, &expected).expect("a matching checksum must pass");
    }

    #[test]
    fn checksum_rejects_a_modified_file() {
        let dir = tempfile::tempdir().expect("create temp directory");
        let path = dir.path().join("qemu.bin");
        std::fs::write(&path, b"compiled qemu").expect("write test file");
        let expected = lowercase_hex(&Sha256::digest(b"compiled qemu"));
        std::fs::write(&path, b"patched qemu!").expect("modify test file");

        let error = verify_checksum(&path, &expected).expect_err("a modified file must fail");
        assert!(
            matches!(error, EngineError::Setup(_)),
            "unexpected error: {error}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_program_name_gets_an_exe_suffix() {
        let candidates = program_candidates(OsStr::new("qemu-system-i386"));

        assert!(
            candidates.iter().any(|candidate| candidate
                .to_string_lossy()
                .eq_ignore_ascii_case("qemu-system-i386.exe")),
            "candidates: {candidates:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_candidates_include_the_install_roots() {
        struct EnvRestore {
            values: Vec<(&'static str, Option<OsString>)>,
        }

        impl EnvRestore {
            fn capture(names: &[&'static str]) -> Self {
                Self {
                    values: names
                        .iter()
                        .map(|name| (*name, env::var_os(name)))
                        .collect(),
                }
            }
        }

        impl Drop for EnvRestore {
            fn drop(&mut self) {
                for (name, value) in &self.values {
                    unsafe {
                        match value {
                            Some(value) => env::set_var(name, value),
                            None => env::remove_var(name),
                        }
                    }
                }
            }
        }

        let _restore = EnvRestore::capture(&["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]);
        let dir = tempfile::tempdir().expect("create temp directory");
        let program_files = dir.path().join("ProgramFiles");
        let program_files_x86 = dir.path().join("ProgramFilesX86");
        let local_app_data = dir.path().join("LocalAppData");

        unsafe {
            env::set_var("ProgramFiles", &program_files);
            env::set_var("ProgramFiles(x86)", &program_files_x86);
            env::set_var("LOCALAPPDATA", &local_app_data);
        }

        let candidates = windows_install_candidates(OsStr::new("qemu-system-i386"));
        let roots = [
            program_files.join("qemu"),
            program_files_x86.join("qemu"),
            local_app_data.join("Programs").join("qemu"),
        ];

        let mut remaining = candidates.iter();
        for root in roots {
            assert!(
                remaining.any(|candidate| candidate.starts_with(&root)),
                "install root {root:?} is missing from {candidates:?}"
            );
        }
    }

    fn lowercase_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
