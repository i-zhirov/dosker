//! Unix process cleanup and detached spawn flags.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

use crate::error::PlatformError;

pub fn prepare_detached(cmd: &mut Command) -> Result<(), PlatformError> {
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

pub fn prepare_engine(cmd: &mut Command) -> Result<(), PlatformError> {
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(io::Error::last_os_error());
            }

            #[cfg(target_os = "linux")]
            {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() == 1 {
                    libc::_exit(1);
                }
            }

            Ok(())
        });
    }
    Ok(())
}

pub fn kill_group(pgid: u32) -> Result<(), PlatformError> {
    let target = group_target(pgid)?;
    if unsafe { libc::kill(target, libc::SIGKILL) } == 0 {
        return Ok(());
    }

    let source = io::Error::last_os_error();
    if source.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(PlatformError::IoPlain(source))
}

pub fn process_alive(pid: u32) -> bool {
    let Ok(target) = process_id(pid) else {
        return false;
    };
    if unsafe { libc::kill(target, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn process_group(pid: u32) -> Result<Option<u32>, PlatformError> {
    let target = process_id(pid)?;
    let result = unsafe { libc::getpgid(target) };
    if result != -1 {
        return Ok(Some(result as u32));
    }

    let source = io::Error::last_os_error();
    if source.raw_os_error() == Some(libc::ESRCH) {
        return Ok(None);
    }
    Err(PlatformError::IoPlain(source))
}

fn group_target(pgid: u32) -> Result<i32, PlatformError> {
    if pgid == 0 {
        return Err(PlatformError::IoPlain(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process group id is zero",
        )));
    }
    i32::try_from(pgid).map(|value| -value).map_err(|_| {
        PlatformError::IoPlain(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process group id is too large",
        ))
    })
}

fn process_id(pid: u32) -> Result<i32, PlatformError> {
    if pid == 0 {
        return Err(PlatformError::IoPlain(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process id is zero",
        )));
    }
    i32::try_from(pid).map_err(|_| {
        PlatformError::IoPlain(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process id is too large",
        ))
    })
}
