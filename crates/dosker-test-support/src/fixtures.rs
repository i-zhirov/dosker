//! Shared helpers for tests that drive real processes.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The environment variable that enables real-engine tests.
pub const INTEGRATION_ENV: &str = "DOSKER_INTEGRATION";

/// The environment variable that names the prepared QEMU image.
pub const QEMU_IMAGE_ENV: &str = "DOSKER_QEMU_IMAGE";

/// The environment variable that names the network QEMU image.
pub const QEMU_NET_IMAGE_ENV: &str = "DOSKER_QEMU_NET_IMAGE";

/// The environment variable that overrides the QEMU binary.
pub const QEMU_ENV: &str = "DOSKER_QEMU";

/// The environment variable that names the FreeDOS source boot floppy image.
pub const FREEDOS_SOURCE_ENV: &str = "DOSKER_FREEDOS_SOURCE";

/// The environment variable that names the FreeDOS base fixture image.
pub const FREEDOS_BASE_IMAGE_ENV: &str = "DOSKER_FREEDOS_BASE_IMAGE";

const DEFAULT_QEMU: &str = "qemu-system-i386";
const DEFAULT_FREEDOS_BASE_IMAGE: &str = "agent/fixtures/freedos-base.img";

/// A temporary directory that removes itself when dropped.
pub struct TempRoot {
    dir: tempfile::TempDir,
}

impl TempRoot {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("create a temporary directory"),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

impl Default for TempRoot {
    fn default() -> Self {
        Self::new()
    }
}

/// The environment variable that overrides the mock engine path.
pub const MOCK_ENGINE_ENV: &str = "DOSKER_MOCK_ENGINE";

/// Reports whether real-engine tests were enabled with the exact value `1`.
pub fn integration_enabled() -> bool {
    matches!(std::env::var(INTEGRATION_ENV).as_deref(), Ok("1"))
}

/// Resolves the prepared image named by [`QEMU_IMAGE_ENV`].
///
/// A relative value is resolved against the workspace root, which is two
/// levels above this crate's manifest directory.
pub fn qemu_image() -> Result<PathBuf, String> {
    let value = std::env::var_os(QEMU_IMAGE_ENV).filter(|value| !value.is_empty());
    let Some(value) = value else {
        return Err(format!(
            "{QEMU_IMAGE_ENV} is not set; set it to the prepared image path"
        ));
    };

    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    };

    if !path.is_file() {
        return Err(format!(
            "{QEMU_IMAGE_ENV} does not name a prepared image: {}",
            path.display()
        ));
    }

    Ok(path)
}

/// Resolves the network image named by [`QEMU_NET_IMAGE_ENV`].
///
/// A relative value is resolved against the workspace root, which is two
/// levels above this crate's manifest directory.
pub fn net_image() -> Result<PathBuf, String> {
    let value = std::env::var_os(QEMU_NET_IMAGE_ENV).filter(|value| !value.is_empty());
    let Some(value) = value else {
        return Err(format!(
            "{QEMU_NET_IMAGE_ENV} is not set; set it to the network image path"
        ));
    };

    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    };

    if !path.is_file() {
        return Err(format!(
            "{QEMU_NET_IMAGE_ENV} does not name a network image: {}",
            path.display()
        ));
    }

    Ok(path)
}

/// Resolves the FreeDOS base fixture named by [`FREEDOS_BASE_IMAGE_ENV`].
///
/// The value defaults to `agent/fixtures/freedos-base.img`. A relative value
/// is resolved against the workspace root, which is two levels above this
/// crate's manifest directory.
pub fn freedos_base_image() -> Result<PathBuf, String> {
    let value = std::env::var_os(FREEDOS_BASE_IMAGE_ENV)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| std::ffi::OsString::from(DEFAULT_FREEDOS_BASE_IMAGE));

    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    };

    if !path.is_file() {
        return Err(format!(
            "the FreeDOS base image is missing: {}; set {FREEDOS_BASE_IMAGE_ENV} to a base disk image",
            path.display()
        ));
    }

    Ok(path)
}

/// Resolves the workspace agent executable at `agent/build/DOSKER.EXE`.
pub fn agent_executable() -> Result<PathBuf, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("agent/build/DOSKER.EXE");
    if !path.is_file() {
        return Err(format!(
            "the agent executable is missing: {}; build it into agent/build/DOSKER.EXE",
            path.display()
        ));
    }
    Ok(path)
}

/// Resolves the workspace boot-sector helper at `agent/build/SETBOOT.EXE`.
pub fn setboot_executable() -> Result<PathBuf, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("agent/build/SETBOOT.EXE");
    if !path.is_file() {
        return Err(format!(
            "the setboot helper is missing: {}; build it into agent/build/SETBOOT.EXE",
            path.display()
        ));
    }
    Ok(path)
}

/// Resolves the workspace reboot helper at `agent/build/REBOOT.EXE`.
pub fn reboot_executable() -> Result<PathBuf, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("agent/build/REBOOT.EXE");
    if !path.is_file() {
        return Err(format!(
            "the reboot helper is missing: {}; build it into agent/build/REBOOT.EXE",
            path.display()
        ));
    }
    Ok(path)
}

/// Resolves the FreeDOS source image named by [`FREEDOS_SOURCE_ENV`].
///
/// A relative value is resolved against the workspace root, which is two
/// levels above this crate's manifest directory.
pub fn freedos_source() -> Result<PathBuf, String> {
    let value = std::env::var_os(FREEDOS_SOURCE_ENV).filter(|value| !value.is_empty());
    let Some(value) = value else {
        return Err(format!(
            "{FREEDOS_SOURCE_ENV} is not set; set it to a FreeDOS source boot image"
        ));
    };

    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    };

    if !path.is_file() {
        return Err(format!(
            "{FREEDOS_SOURCE_ENV} does not name a FreeDOS source image: {}",
            path.display()
        ));
    }

    Ok(path)
}

/// Returns the QEMU program named by [`QEMU_ENV`] or the default program.
pub fn qemu_binary() -> String {
    std::env::var(QEMU_ENV)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_QEMU.to_owned())
}

/// Locates the mock engine binary built beside the test executable.
pub fn mock_engine_path() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os(MOCK_ENGINE_ENV) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
    }

    let current = std::env::current_exe()
        .map_err(|source| format!("cannot locate the current executable: {source}"))?;
    let name = if cfg!(windows) {
        "mock-engine.exe"
    } else {
        "mock-engine"
    };

    if let Some(parent) = current.parent() {
        let beside = parent.join(name);
        if beside.is_file() {
            return Ok(beside);
        }
        if let Some(above) = parent.parent() {
            let one_up = above.join(name);
            if one_up.is_file() {
                return Ok(one_up);
            }
        }
    }

    Err(String::from(
        "mock engine binary not found; build it with `cargo build -p dosker-test-support --bin mock-engine`",
    ))
}

/// Calls `predicate` until it returns true or the timeout expires.
pub fn wait_until(timeout: Duration, step: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(step);
    }
}

/// Kills a process without waiting for it. Returns false when it is already gone.
#[cfg(unix)]
pub fn kill_process(pid: u32) -> bool {
    let Ok(target) = i32::try_from(pid) else {
        return false;
    };
    if target == 0 {
        return false;
    }
    unsafe { libc::kill(target, libc::SIGKILL) == 0 }
}

/// Kills a process without waiting for it. Returns false when it is already gone.
#[cfg(windows)]
pub fn kill_process(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let terminated = TerminateProcess(handle, 1);
        CloseHandle(handle);
        terminated != 0
    }
}
