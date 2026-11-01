//! Windows process liveness and detached spawn flags.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, GetLastError, STILL_ACTIVE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::error::PlatformError;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub fn prepare_detached(cmd: &mut Command) -> Result<(), PlatformError> {
    let _ = cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    Ok(())
}

pub fn prepare_engine(cmd: &mut Command) -> Result<(), PlatformError> {
    prepare_detached(cmd)
}

pub fn kill_group(_pgid: u32) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported("process group cleanup"))
}

pub fn process_alive(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return match GetLastError() {
                ERROR_INVALID_PARAMETER => false,
                ERROR_ACCESS_DENIED => true,
                _ => false,
            };
        }

        let mut exit_code = 0u32;
        let read = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        read != 0 && exit_code == STILL_ACTIVE as u32
    }
}

pub fn process_group(_pid: u32) -> Result<Option<u32>, PlatformError> {
    Ok(None)
}
