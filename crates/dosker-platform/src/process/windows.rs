//! Windows process liveness, cleanup, and detached spawn flags.

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::Command;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, GetLastError, HANDLE, STILL_ACTIVE,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    TerminateProcess,
};

use crate::error::PlatformError;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// A Windows Job Object that kills its processes when it closes.
pub struct JobObject {
    handle: HANDLE,
}

impl JobObject {
    /// Creates a Job Object that kills its processes when it closes.
    pub fn kill_on_close() -> Result<Self, PlatformError> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(last_error());
            }

            let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &information as *const _ as *const c_void,
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .unwrap_or(u32::MAX),
            );
            if configured == 0 {
                let error = last_error();
                CloseHandle(handle);
                return Err(error);
            }

            Ok(Self { handle })
        }
    }

    /// Adds a child process to the job.
    pub fn assign_child(&self, child: &std::process::Child) -> Result<(), PlatformError> {
        let assigned =
            unsafe { AssignProcessToJobObject(self.handle, child.as_raw_handle() as HANDLE) };
        if assigned == 0 {
            return Err(last_error());
        }

        Ok(())
    }
}

unsafe impl Send for JobObject {}

impl Drop for JobObject {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

pub fn prepare_detached(cmd: &mut Command) -> Result<(), PlatformError> {
    let _ = cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    Ok(())
}

pub fn prepare_engine(cmd: &mut Command) -> Result<(), PlatformError> {
    prepare_detached(cmd)
}

pub fn kill_group(pid: u32) -> Result<(), PlatformError> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return Err(last_error());
        }

        let terminated = TerminateProcess(handle, 1);
        let error = if terminated == 0 {
            Some(last_error())
        } else {
            None
        };
        CloseHandle(handle);

        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
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

fn last_error() -> PlatformError {
    PlatformError::IoPlain(io::Error::last_os_error())
}
