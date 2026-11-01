//! Filesystem helpers.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::error::PlatformError;

/// Creates a directory and sets its mode.
///
/// The mode is applied on Unix and ignored on Windows.
pub fn ensure_dir_mode(path: &Path, mode: u32) -> Result<(), PlatformError> {
    fs::create_dir_all(path).map_err(|source| PlatformError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::symlink_metadata(path).map_err(|source| PlatformError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(PlatformError::Io {
                path: path.to_path_buf(),
                source: io::Error::new(io::ErrorKind::InvalidInput, "directory path is a symlink"),
            });
        }
        if !metadata.is_dir() {
            return Err(PlatformError::Io {
                path: path.to_path_buf(),
                source: io::Error::new(io::ErrorKind::InvalidInput, "path is not a directory"),
            });
        }

        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
            PlatformError::Io {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }

    #[cfg(windows)]
    {
        let _ = mode;
    }

    Ok(())
}

/// Writes bytes to a file atomically.
///
/// The bytes go to a temporary file in the same directory. The temporary file
/// is synced and then replaces the target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PlatformError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let file_name = path.file_name().ok_or_else(|| PlatformError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidInput, "target path has no file name"),
    })?;
    let temp_path = parent.join(temp_file_name(file_name));

    if let Err(source) = write_temp(&temp_path, bytes) {
        let _ = fs::remove_file(&temp_path);
        return Err(PlatformError::Io {
            path: temp_path,
            source,
        });
    }

    #[cfg(unix)]
    {
        if let Err(source) = fs::rename(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(PlatformError::Io {
                path: path.to_path_buf(),
                source,
            });
        }

        let directory = fs::File::open(parent).map_err(|source| PlatformError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        directory.sync_all().map_err(|source| PlatformError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    #[cfg(windows)]
    {
        if let Err(source) = replace_file(path, &temp_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(PlatformError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    }

    Ok(())
}

fn temp_file_name(file_name: &std::ffi::OsStr) -> OsString {
    let mut name = OsString::from(".");
    name.push(file_name);
    name.push(format!(".tmp.{}", std::process::id()));
    name
}

fn write_temp(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(windows)]
fn replace_file(path: &Path, temp_path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GetLastError,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let replaced = wide(path);
    let replacement = wide(temp_path);

    let replaced_ok = unsafe {
        ReplaceFileW(
            replaced.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            std::ptr::null(),
            std::ptr::null(),
        )
    };

    if replaced_ok != 0 {
        return Ok(());
    }

    let error = unsafe { GetLastError() };
    if error != ERROR_FILE_NOT_FOUND && error != ERROR_PATH_NOT_FOUND {
        return Err(io::Error::from_raw_os_error(error as i32));
    }

    let moved_ok = unsafe {
        MoveFileExW(
            replacement.as_ptr(),
            replaced.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if moved_ok == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}
