//! Shared helpers for tests that drive real processes.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
