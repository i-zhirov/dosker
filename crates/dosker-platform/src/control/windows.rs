//! Windows named-pipe control transport.

use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE,
    GetLastError, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree, WAIT_FAILED, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED, OVERLAPPED_0};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, INFINITE, OpenProcessToken, ResetEvent, WaitForSingleObject,
};
use windows_sys::core::PWSTR;

use crate::endpoint::Endpoint;
use crate::error::PlatformError;

use super::{ControlConnection, ControlListener};

const PIPE_BUFFER_BYTES: u32 = 65_536;
const ALIVE_PROBE_BUDGET: Duration = Duration::from_millis(200);
const RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// A control connection backed by a named-pipe instance.
pub struct PipeConnection {
    handle: PipeHandle,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

impl PipeConnection {
    fn new(handle: HANDLE) -> Self {
        Self::from_pipe(PipeHandle::new(handle))
    }

    fn from_pipe(handle: PipeHandle) -> Self {
        Self {
            handle,
            read_timeout: None,
            write_timeout: None,
        }
    }

    fn transfer(
        &self,
        buffer: *mut u8,
        length: u32,
        timeout: Option<Duration>,
        write: bool,
    ) -> io::Result<usize> {
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        let event = PipeHandle::new(event);
        let mut overlapped = overlapped_for(event.raw());
        let mut transferred = 0u32;

        let started = unsafe {
            if write {
                WriteFile(
                    self.handle.raw(),
                    buffer,
                    length,
                    &mut transferred,
                    &mut overlapped,
                )
            } else {
                ReadFile(
                    self.handle.raw(),
                    buffer,
                    length,
                    &mut transferred,
                    &mut overlapped,
                )
            }
        };

        if started != 0 {
            return Ok(transferred as usize);
        }

        let error = unsafe { GetLastError() };
        if error == ERROR_IO_PENDING {
            return wait_for_transfer(self.handle.raw(), &overlapped, &event, timeout, !write);
        }
        if !write && is_eof_error(error) {
            return Ok(0);
        }
        Err(io::Error::from_raw_os_error(error as i32))
    }
}

impl Read for PipeConnection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let length = buf.len().min(u32::MAX as usize) as u32;
        self.transfer(buf.as_mut_ptr(), length, self.read_timeout, false)
    }
}

impl Write for PipeConnection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let length = buf.len().min(u32::MAX as usize) as u32;
        self.transfer(buf.as_ptr() as *mut u8, length, self.write_timeout, true)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ControlConnection for PipeConnection {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.read_timeout = timeout;
        Ok(())
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.write_timeout = timeout;
        Ok(())
    }

    fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A listener that hands out named-pipe connections.
pub struct PipeListener {
    connections: Receiver<io::Result<Box<dyn ControlConnection>>>,
}

impl ControlListener for PipeListener {
    fn accept(&self, timeout: Duration) -> io::Result<Option<Box<dyn ControlConnection>>> {
        match self.connections.recv_timeout(timeout) {
            Ok(connection) => connection.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "control acceptor stopped",
            )),
        }
    }
}

pub fn bind(endpoint: &Endpoint) -> Result<Box<dyn ControlListener>, PlatformError> {
    let name = pipe_name(endpoint)?;
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let security = PipeSecurity::for_current_user()?;
    let first = create_instance(&wide, &security).map_err(|source| PlatformError::Io {
        path: PathBuf::from(name),
        source,
    })?;

    let (sender, connections) = mpsc::channel();
    thread::spawn(move || accept_loop(wide, security, first, sender));

    Ok(Box::new(PipeListener { connections }))
}

pub fn connect(
    endpoint: &Endpoint,
    timeout: Duration,
) -> Result<Box<dyn ControlConnection>, PlatformError> {
    let name = pipe_name(endpoint)?;
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let deadline = Instant::now() + timeout;
    let mut last_error = ERROR_FILE_NOT_FOUND;

    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(pipe_error(name, last_error));
        }

        let remaining = deadline.saturating_duration_since(now);
        unsafe {
            WaitNamedPipeW(wide.as_ptr(), wait_millis(remaining));
        }

        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            return Ok(Box::new(PipeConnection::new(handle)));
        }

        let error = unsafe { GetLastError() };
        if !is_retryable_connect(error) {
            return Err(pipe_error(name, error));
        }
        last_error = error;

        let now = Instant::now();
        if now >= deadline {
            return Err(pipe_error(name, last_error));
        }
        thread::sleep(RETRY_INTERVAL.min(deadline.saturating_duration_since(now)));
    }
}

pub fn endpoint_alive(endpoint: &Endpoint) -> bool {
    connect(endpoint, ALIVE_PROBE_BUDGET).is_ok()
}

pub fn remove_endpoint(_endpoint: &Endpoint) -> Result<(), PlatformError> {
    Ok(())
}

fn accept_loop(
    name: Vec<u16>,
    security: PipeSecurity,
    first: PipeHandle,
    sender: Sender<io::Result<Box<dyn ControlConnection>>>,
) {
    let event = unsafe { CreateEventW(null(), 1, 0, null()) };
    if event.is_null() {
        let _ = sender.send(Err(io::Error::last_os_error()));
        return;
    }
    let event = PipeHandle::new(event);
    let mut instance = first;

    loop {
        if let Err(error) = connect_instance(&instance, &event) {
            let _ = sender.send(Err(error));
            break;
        }

        let next = create_instance(&name, &security);
        let connection: Box<dyn ControlConnection> = Box::new(PipeConnection::from_pipe(instance));

        if sender.send(Ok(connection)).is_err() {
            break;
        }

        match next {
            Ok(handle) => instance = handle,
            Err(error) => {
                let _ = sender.send(Err(error));
                break;
            }
        }
    }
}

fn connect_instance(instance: &PipeHandle, event: &PipeHandle) -> io::Result<()> {
    unsafe {
        ResetEvent(event.raw());
    }
    let mut overlapped = overlapped_for(event.raw());

    let connected = unsafe { ConnectNamedPipe(instance.raw(), &mut overlapped) };
    if connected != 0 {
        return Ok(());
    }

    match unsafe { GetLastError() } {
        ERROR_PIPE_CONNECTED => Ok(()),
        ERROR_IO_PENDING => {
            let wait = unsafe { WaitForSingleObject(event.raw(), INFINITE) };
            match wait {
                WAIT_OBJECT_0 => {
                    overlapped_result(instance.raw(), &overlapped, false, false).map(|_| ())
                }
                WAIT_FAILED => Err(io::Error::last_os_error()),
                _ => Err(io::Error::other("unexpected connect wait result")),
            }
        }
        error => Err(io::Error::from_raw_os_error(error as i32)),
    }
}

fn wait_for_transfer(
    handle: HANDLE,
    overlapped: &OVERLAPPED,
    event: &PipeHandle,
    timeout: Option<Duration>,
    eof_as_zero: bool,
) -> io::Result<usize> {
    let milliseconds = timeout.map(timeout_millis).unwrap_or(INFINITE);
    match unsafe { WaitForSingleObject(event.raw(), milliseconds) } {
        WAIT_OBJECT_0 => overlapped_result(handle, overlapped, false, eof_as_zero),
        WAIT_TIMEOUT => {
            unsafe {
                CancelIoEx(handle, overlapped);
            }
            let _ = overlapped_result(handle, overlapped, true, eof_as_zero);
            Err(io::Error::from(io::ErrorKind::TimedOut))
        }
        WAIT_FAILED => Err(io::Error::last_os_error()),
        _ => Err(io::Error::other("unexpected transfer wait result")),
    }
}

fn overlapped_result(
    handle: HANDLE,
    overlapped: &OVERLAPPED,
    wait: bool,
    eof_as_zero: bool,
) -> io::Result<usize> {
    let mut transferred = 0u32;
    let completed = unsafe {
        GetOverlappedResult(
            handle,
            overlapped,
            &mut transferred,
            if wait { 1 } else { 0 },
        )
    };
    if completed != 0 {
        return Ok(transferred as usize);
    }

    let error = unsafe { GetLastError() };
    if eof_as_zero && is_eof_error(error) {
        return Ok(0);
    }
    Err(io::Error::from_raw_os_error(error as i32))
}

fn overlapped_for(event: HANDLE) -> OVERLAPPED {
    OVERLAPPED {
        Internal: 0,
        InternalHigh: 0,
        Anonymous: OVERLAPPED_0 {
            Pointer: null_mut(),
        },
        hEvent: event,
    }
}

fn create_instance(name: &[u16], security: &PipeSecurity) -> io::Result<PipeHandle> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.descriptor,
        bInheritHandle: 0,
    };

    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            &attributes,
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(PipeHandle::new(handle))
}

fn pipe_name(endpoint: &Endpoint) -> Result<&str, PlatformError> {
    match endpoint {
        Endpoint::NamedPipe(name) => Ok(name),
        Endpoint::UnixSocket(path) => Err(PlatformError::InvalidEndpoint(format!(
            "unix socket {} on a windows host",
            path.display()
        ))),
    }
}

fn pipe_error(name: &str, error: u32) -> PlatformError {
    PlatformError::Io {
        path: PathBuf::from(name),
        source: io::Error::from_raw_os_error(error as i32),
    }
}

fn is_retryable_connect(error: u32) -> bool {
    error == ERROR_PIPE_BUSY || error == ERROR_FILE_NOT_FOUND
}

fn is_eof_error(error: u32) -> bool {
    error == ERROR_BROKEN_PIPE || error == ERROR_PIPE_NOT_CONNECTED || error == ERROR_NO_DATA
}

fn wait_millis(duration: Duration) -> u32 {
    (duration.as_millis().min(u32::MAX as u128) as u32).max(1)
}

fn timeout_millis(duration: Duration) -> u32 {
    duration.as_millis().min(u32::MAX as u128) as u32
}

struct PipeHandle(HANDLE);

impl PipeHandle {
    fn new(handle: HANDLE) -> Self {
        Self(handle)
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for PipeHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

unsafe impl Send for PipeHandle {}

struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
}

impl PipeSecurity {
    fn for_current_user() -> io::Result<Self> {
        let sid = current_user_sid_string()?;
        let sddl = format!("D:P(A;;GA;;;{sid})");
        let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();

        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self { descriptor })
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe {
                LocalFree(self.descriptor as HLOCAL);
            }
        }
    }
}

// The descriptor is immutable after construction and the acceptor thread owns it.
unsafe impl Send for PipeSecurity {}

fn current_user_sid_string() -> io::Result<String> {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }

        let result = token_user_sid_string(token);
        CloseHandle(token);
        result
    }
}

fn token_user_sid_string(token: HANDLE) -> io::Result<String> {
    unsafe {
        let mut needed = 0u32;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed);
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }

        let words = (needed as usize).div_ceil(std::mem::size_of::<usize>());
        let mut buffer = vec![0usize; words];
        let filled = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr() as *mut c_void,
            needed,
            &mut needed,
        );
        if filled == 0 {
            return Err(io::Error::last_os_error());
        }

        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut sid_string: PWSTR = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid_string) == 0 {
            return Err(io::Error::last_os_error());
        }

        let length = (0..)
            .take_while(|&index| *sid_string.add(index) != 0)
            .count();
        let slice = std::slice::from_raw_parts(sid_string, length);
        let result = String::from_utf16_lossy(slice);
        LocalFree(sid_string as HLOCAL);
        Ok(result)
    }
}
