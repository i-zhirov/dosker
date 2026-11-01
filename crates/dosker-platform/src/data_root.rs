//! Data root discovery.

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::error::PlatformError;

/// The environment variable that overrides the data root.
pub const ROOT_ENV: &str = "DOSKER_ROOT";

/// Returns the data root for the current host.
///
/// `DOSKER_ROOT` takes precedence. The result is always absolute.
pub fn data_root() -> Result<PathBuf, PlatformError> {
    let root = match non_empty_env(ROOT_ENV) {
        Some(value) => PathBuf::from(value),
        None => host_data_root()?,
    };

    Ok(std::path::absolute(root)?)
}

fn non_empty_env(name: &'static str) -> Option<OsString> {
    env::var_os(name).filter(|value| !value.is_empty())
}

#[cfg(target_os = "linux")]
fn host_data_root() -> Result<PathBuf, PlatformError> {
    xdg_data_home()
}

#[cfg(target_os = "macos")]
fn host_data_root() -> Result<PathBuf, PlatformError> {
    application_support()
}

#[cfg(windows)]
fn host_data_root() -> Result<PathBuf, PlatformError> {
    local_app_data()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn host_data_root() -> Result<PathBuf, PlatformError> {
    Err(PlatformError::Unsupported("data root discovery"))
}

#[cfg(target_os = "linux")]
fn xdg_data_home() -> Result<PathBuf, PlatformError> {
    if let Some(value) = non_empty_env("XDG_DATA_HOME") {
        return Ok(PathBuf::from(value).join("dosker"));
    }

    let home = non_empty_env("HOME").ok_or(PlatformError::MissingEnvironment("HOME"))?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("dosker"))
}

#[cfg(target_os = "macos")]
fn application_support() -> Result<PathBuf, PlatformError> {
    let home = non_empty_env("HOME").ok_or(PlatformError::MissingEnvironment("HOME"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("dosker"))
}

#[cfg(windows)]
fn local_app_data() -> Result<PathBuf, PlatformError> {
    let value =
        non_empty_env("LOCALAPPDATA").ok_or(PlatformError::MissingEnvironment("LOCALAPPDATA"))?;
    Ok(PathBuf::from(value).join("dosker"))
}
