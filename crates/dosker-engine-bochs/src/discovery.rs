//! Bochs binary discovery and version probing.

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use dosker_engine::EngineError;

/// The environment variable that overrides Bochs discovery.
pub const BOCHS_ENV: &str = "DOSKER_BOCHS";

/// The environment variable that pins the exact Bochs version.
pub const BOCHS_VERSION_ENV: &str = "DOSKER_BOCHS_VERSION";

/// The environment variable that pins the Bochs executable checksum.
pub const BOCHS_SHA256_ENV: &str = "DOSKER_BOCHS_SHA256";

/// The Bochs program name used when no override is set.
pub const DEFAULT_BOCHS: &str = "bochs";

/// The maximum time a version probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

const PROBE_POLL: Duration = Duration::from_millis(20);
const CHECKSUM_BUFFER_BYTES: usize = 64 * 1024;
const VERSION_MARKER: &str = "Bochs x86 Emulator ";
const INTERNAL_DEBUGGER_MARKER: &str = "Bochs internal debugger";
const PROBE_CONFIG: &str = "\
romimage: file=$BXSHARE/BIOS-bochs-latest
vgaromimage: file=$BXSHARE/VGABIOS-lgpl-latest.bin
cpu: model=i486dx4
megs: 16
display_library: nogui
log: -
panic: action=fatal
error: action=report
info: action=report
debug: action=ignore
";
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Which debugger a Bochs build includes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Debugger {
    Internal,
    None,
}

/// A discovered Bochs binary and its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BochsBinary {
    pub path: PathBuf,
    pub version: String,
    pub debugger: Debugger,
}

impl BochsBinary {
    /// Finds Bochs through the environment override or `PATH`.
    pub fn discover() -> Result<Self, EngineError> {
        let requested = match env::var_os(BOCHS_ENV) {
            Some(value) if !value.is_empty() => value,
            _ => OsString::from(DEFAULT_BOCHS),
        };
        let path = resolve_program(&requested)?;

        Self::from_path(&path)
    }

    /// Probes one binary path for its version and debugger.
    pub fn from_path(path: &Path) -> Result<Self, EngineError> {
        if !path.is_file() {
            return Err(EngineError::NotFound(path.to_path_buf()));
        }

        if let Some(expected) = env_pin(BOCHS_SHA256_ENV) {
            verify_checksum(path, &expected)?;
        }

        let (version, debugger) = probe(path, PROBE_TIMEOUT)?;

        let pinned = env_pin(BOCHS_VERSION_ENV);
        check_version(&version, pinned.as_deref())?;

        Ok(Self {
            path: path.to_path_buf(),
            version,
            debugger,
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

    /// Reports whether the build includes the internal debugger.
    pub fn has_internal_debugger(&self) -> bool {
        self.debugger == Debugger::Internal
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
            "Bochs checksum mismatch for {}: expected {}, found {}",
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
            "Bochs version mismatch: expected {pinned}, found {version}"
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

fn probe(path: &Path, timeout: Duration) -> Result<(String, Debugger), EngineError> {
    let config = write_probe_config()?;
    let result = run_probe(path, &config, timeout);
    let _ = std::fs::remove_file(&config);
    result
}

fn write_probe_config() -> Result<PathBuf, EngineError> {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = env::temp_dir().join(format!(
        "dosker-bochs-probe-{}-{id}.bochsrc",
        std::process::id()
    ));
    std::fs::write(&path, PROBE_CONFIG).map_err(EngineError::Io)?;

    Ok(path)
}

fn run_probe(
    path: &Path,
    config: &Path,
    timeout: Duration,
) -> Result<(String, Debugger), EngineError> {
    let mut child = match Command::new(path)
        .arg("-dbg")
        .arg("-q")
        .arg("-f")
        .arg(config)
        .stdin(Stdio::piped())
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

    let stdout = child.stdout.take().map(spawn_reader);
    let stderr = child.stderr.take().map(spawn_reader);

    if let Some(mut stdin) = child.stdin.take() {
        // The child may have exited before the write; a broken pipe is not a
        // probe failure.
        let _ = stdin.write_all(b"quit\n");
    }

    let deadline = Instant::now() + timeout;
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(_)) => break Ok(false),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(PROBE_POLL),
            Ok(None) => break Ok(true),
            Err(error) => break Err(error),
        }
    };

    let timed_out = match outcome {
        Ok(timed_out) => timed_out,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = read_all(stdout);
            let _ = read_all(stderr);
            return Err(EngineError::Io(error));
        }
    };

    if timed_out {
        let _ = child.kill();
        // Reap the child so that its pipe ends close.
        let _ = child.wait();
    }

    let stdout = read_all(stdout);
    let stderr = read_all(stderr);
    let mut text = String::from_utf8_lossy(&stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&stderr));

    let Some(version) = parse_version(&text) else {
        if timed_out {
            return Err(EngineError::Setup(
                "bochs version probe timed out".to_owned(),
            ));
        }

        return Err(EngineError::Setup(format!(
            "could not parse the Bochs version from {}",
            path.display()
        )));
    };

    // A build without the internal debugger rejects `-dbg`, but its banner
    // still identifies the version.
    let debugger = if text.contains(INTERNAL_DEBUGGER_MARKER) {
        Debugger::Internal
    } else {
        Debugger::None
    };

    Ok((version, debugger))
}

/// Reads one child pipe to the end on a separate thread, so that a verbose
/// child cannot fill the pipe and block before it reads `quit`.
fn spawn_reader<R>(mut reader: R) -> std::thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        // A read error keeps the bytes collected so far for parsing.
        let _ = reader.read_to_end(&mut buffer);
        buffer
    })
}

fn read_all(handle: Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default()
}

#[cfg(all(test, unix))]
mod tests {
    use super::probe;
    use dosker_engine::EngineError;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn write_script(dir: &Path, body: &str) -> PathBuf {
        let script = dir.join("bochs");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("write script");
        let mut permissions = std::fs::metadata(&script)
            .expect("read script metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).expect("set script permissions");

        script
    }

    #[test]
    fn probe_times_out_when_the_binary_hangs() {
        let dir = tempfile::tempdir().expect("create temp directory");
        let script = write_script(dir.path(), "exec sleep 30");

        let error =
            probe(&script, Duration::from_millis(200)).expect_err("a hanging probe must time out");

        match error {
            EngineError::Setup(message) => {
                assert_eq!(message, "bochs version probe timed out");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn probe_drains_verbose_output() {
        let dir = tempfile::tempdir().expect("create temp directory");
        let script = write_script(
            dir.path(),
            "head -c 65536 /dev/zero | tr '\\0' 'x'\n\
             head -c 65536 /dev/zero | tr '\\0' 'y' >&2\n\
             echo 'Bochs x86 Emulator 3.1'",
        );

        let (version, _debugger) =
            probe(&script, Duration::from_secs(2)).expect("a verbose probe must not time out");

        assert_eq!(version, "3.1");
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
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
        program_candidates(OsStr::new("bochs"))
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
        assert_eq!(candidate_names(), ["bochs.COM", "bochs.EXE", "bochs.BAT"]);

        unsafe {
            env::set_var("PATHEXT", ".COM;.BAT");
        }
        assert_eq!(candidate_names(), ["bochs.COM", "bochs.BAT", "bochs.EXE"]);
    }
}
