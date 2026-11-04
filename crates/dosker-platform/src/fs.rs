//! Filesystem helpers.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::error::PlatformError;

/// How a private disk copy was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyOutcome {
    /// The host cloned the source without copying its blocks.
    Reflink,
    /// The host used the full-copy path.
    FullCopy,
}

/// Copies a file, sharing blocks with the source when the filesystem allows.
///
/// The destination parent is created. A reflink failure falls back to a full
/// copy. On Unix the destination ends up at mode `0o600`.
#[cfg(unix)]
pub fn copy_reflink(source: &Path, destination: &Path) -> Result<CopyOutcome, PlatformError> {
    create_parent(destination)?;

    match try_reflink(source, destination) {
        Ok(true) => {
            set_destination_mode(destination)?;
            Ok(CopyOutcome::Reflink)
        }
        Ok(false) | Err(_) => {
            copy_full(source, destination)?;
            Ok(CopyOutcome::FullCopy)
        }
    }
}

/// Copies a file, always writing every byte.
///
/// The destination parent is created and an existing destination is
/// truncated. On Unix the destination ends up at mode `0o600`.
#[cfg(windows)]
pub fn copy_reflink(source: &Path, destination: &Path) -> Result<CopyOutcome, PlatformError> {
    copy_full(source, destination)?;
    Ok(CopyOutcome::FullCopy)
}

/// Copies a file, writing every byte.
///
/// The destination parent is created and an existing destination is
/// truncated. On Unix the destination ends up at mode `0o600`; on Windows the
/// destination read-only attribute is cleared.
///
/// A source that cannot be opened is reported against the source path. Errors
/// from the copy itself are reported against the destination path.
pub fn copy_full(source: &Path, destination: &Path) -> Result<(), PlatformError> {
    create_parent(destination)?;

    File::open(source).map_err(|source_error| PlatformError::Io {
        path: source.to_path_buf(),
        source: source_error,
    })?;

    fs::copy(source, destination).map_err(|source_error| PlatformError::Io {
        path: destination.to_path_buf(),
        source: source_error,
    })?;

    // Apply the writable mode first: on Unix the copy may inherit the source's
    // read-only mode, and on Windows the copy may inherit its read-only
    // attribute. The sync then opens the destination with write access.
    set_destination_mode(destination)?;

    let file = OpenOptions::new()
        .write(true)
        .open(destination)
        .map_err(|source| PlatformError::Io {
            path: destination.to_path_buf(),
            source,
        })?;
    file.sync_all().map_err(|source| PlatformError::Io {
        path: destination.to_path_buf(),
        source,
    })?;

    Ok(())
}

fn create_parent(destination: &Path) -> Result<(), PlatformError> {
    let parent = match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent).map_err(|source| PlatformError::Io {
        path: parent.to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn set_destination_mode(destination: &Path) -> Result<(), PlatformError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(destination, fs::Permissions::from_mode(0o600)).map_err(|source| {
        PlatformError::Io {
            path: destination.to_path_buf(),
            source,
        }
    })
}

#[cfg(windows)]
fn set_destination_mode(destination: &Path) -> Result<(), PlatformError> {
    clear_read_only(destination)
}

#[cfg(windows)]
fn clear_read_only(destination: &Path) -> Result<(), PlatformError> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_READONLY, GetFileAttributesW, INVALID_FILE_ATTRIBUTES, SetFileAttributesW,
    };

    let path: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES || attributes & FILE_ATTRIBUTE_READONLY == 0 {
        return Ok(());
    }

    let cleared =
        unsafe { SetFileAttributesW(path.as_ptr(), attributes & !FILE_ATTRIBUTE_READONLY) };
    if cleared == 0 {
        return Err(PlatformError::Io {
            path: destination.to_path_buf(),
            source: io::Error::last_os_error(),
        });
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn try_reflink(source: &Path, destination: &Path) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    const FICLONE: libc::c_ulong = 0x4004_9409;

    let source_file = File::open(source)?;
    let destination_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(destination)?;

    let result = unsafe {
        libc::ioctl(
            destination_file.as_raw_fd(),
            FICLONE,
            source_file.as_raw_fd(),
        )
    };

    Ok(result == 0)
}

#[cfg(target_os = "macos")]
fn try_reflink(source: &Path, destination: &Path) -> io::Result<bool> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    let parent = match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let file_name = destination.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination path has no file name",
        )
    })?;
    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(
        ".clone.{}.{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let temp_path = parent.join(temp_name);

    let source_path = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path has a nul byte"))?;
    let temp_path_c = CString::new(temp_path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "temporary path has a nul byte")
    })?;

    let result = unsafe { libc::clonefile(source_path.as_ptr(), temp_path_c.as_ptr(), 0) };
    if result != 0 {
        let _ = fs::remove_file(&temp_path);
        return Ok(false);
    }

    match fs::rename(&temp_path, destination) {
        Ok(()) => Ok(true),
        Err(source) => {
            let _ = fs::remove_file(&temp_path);
            Err(source)
        }
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn try_reflink(_source: &Path, _destination: &Path) -> io::Result<bool> {
    Ok(false)
}

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
