//! Pseudo-terminal pair for the Bochs internal debugger.

use std::ffi::{CStr, OsStr};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dosker_engine::EngineError;

/// Serializes access to the static buffer that `ptsname` returns.
static PTY_LOCK: Mutex<()> = Mutex::new(());

/// The master and slave ends of a pseudo-terminal.
pub(crate) struct PtyPair {
    master: File,
    slave: Option<File>,
    slave_path: PathBuf,
}

impl PtyPair {
    /// Opens a new pseudo-terminal pair in raw mode.
    pub(crate) fn open() -> Result<Self, EngineError> {
        // SAFETY: `posix_openpt` takes only flag bits and returns a new
        // descriptor or -1.
        let master_fd =
            unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
        if master_fd < 0 {
            return Err(EngineError::Io(io::Error::last_os_error()));
        }
        // SAFETY: the descriptor is valid and is owned from here on.
        let master = unsafe { File::from_raw_fd(master_fd) };

        // SAFETY: `grantpt` and `unlockpt` take a descriptor returned by
        // `posix_openpt`.
        if unsafe { libc::grantpt(master_fd) } != 0 {
            return Err(EngineError::Io(io::Error::last_os_error()));
        }
        // SAFETY: as above.
        if unsafe { libc::unlockpt(master_fd) } != 0 {
            return Err(EngineError::Io(io::Error::last_os_error()));
        }

        let slave_path = {
            let _guard = PTY_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // SAFETY: `ptsname` returns a pointer to a static buffer that the
            // lock protects; the name is copied before the lock is released.
            let name = unsafe { libc::ptsname(master_fd) };
            if name.is_null() {
                return Err(EngineError::Io(io::Error::last_os_error()));
            }
            // SAFETY: the pointer is non-null and NUL-terminated.
            let name = unsafe { CStr::from_ptr(name) };
            PathBuf::from(OsStr::from_bytes(name.to_bytes()))
        };

        let slave = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
            .open(&slave_path)
            .map_err(EngineError::Io)?;

        let mut termios = MaybeUninit::<libc::termios>::uninit();
        // SAFETY: the slave descriptor is valid and `termios` is writable.
        if unsafe { libc::tcgetattr(slave.as_raw_fd(), termios.as_mut_ptr()) } != 0 {
            return Err(EngineError::Io(io::Error::last_os_error()));
        }
        // SAFETY: `tcgetattr` succeeded and initialized the structure.
        let mut termios = unsafe { termios.assume_init() };
        // SAFETY: `termios` is a valid, initialized structure.
        unsafe { libc::cfmakeraw(&mut termios) };
        // SAFETY: the slave descriptor and the structure are valid.
        if unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &termios) } != 0 {
            return Err(EngineError::Io(io::Error::last_os_error()));
        }

        Ok(Self {
            master,
            slave: Some(slave),
            slave_path,
        })
    }

    /// Returns the slave device path.
    pub(crate) fn slave_path(&self) -> &Path {
        &self.slave_path
    }

    /// Takes the slave end out of the pair.
    pub(crate) fn take_slave(&mut self) -> Option<File> {
        self.slave.take()
    }

    /// Returns the master end.
    pub(crate) fn master(&self) -> &File {
        &self.master
    }

    /// Clones the master end.
    pub(crate) fn master_clone(&self) -> Result<File, EngineError> {
        self.master.try_clone().map_err(EngineError::Io)
    }

    /// Writes one line to the master end, reaching the child's stdin.
    ///
    /// Called at spawn, before the debug bridge sets the master non-blocking.
    pub(crate) fn write_line(&self, line: &str) -> Result<(), EngineError> {
        let mut master = self.master();
        master.write_all(line.as_bytes()).map_err(EngineError::Io)?;
        master.write_all(b"\n").map_err(EngineError::Io)?;
        master.flush().map_err(EngineError::Io)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::FileTypeExt;

    use super::PtyPair;

    /// Waits up to two seconds for `file` to become readable.
    fn wait_readable(file: &File, what: &str) {
        let mut descriptor = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `descriptor` points to one initialized `pollfd` and the
        // finite timeout keeps `poll` from blocking indefinitely.
        let ready = unsafe { libc::poll(&mut descriptor, 1, 2_000) };
        assert!(
            ready > 0,
            "{what} did not become readable within two seconds (poll returned {ready})"
        );
    }

    #[test]
    fn pty_opens_a_pair_with_a_slave_path() {
        let pty = PtyPair::open().expect("open a pty pair");

        assert!(pty.slave_path().exists(), "the slave path must exist");
        let metadata = std::fs::metadata(pty.slave_path()).expect("read the slave path metadata");
        assert!(
            metadata.file_type().is_char_device(),
            "the slave path must be a character device"
        );
    }

    #[test]
    fn pty_master_reads_what_the_slave_writes() {
        let pty = PtyPair::open().expect("open a pty pair");
        let mut slave = pty.slave.as_ref().expect("the slave is open");
        slave.write_all(b"boot\n").expect("write to the slave");

        let mut master = pty.master();
        wait_readable(master, "the master");
        let mut buffer = [0u8; 5];
        master
            .read_exact(&mut buffer)
            .expect("read from the master");

        assert_eq!(&buffer, b"boot\n");
    }

    #[test]
    fn pty_writes_a_line_to_the_child() {
        let mut pty = PtyPair::open().expect("open a pty pair");
        pty.write_line("c").expect("write the line to the master");

        let mut slave = pty.take_slave().expect("take the slave");
        wait_readable(&slave, "the slave");
        let mut buffer = [0u8; 2];
        slave
            .read_exact(&mut buffer)
            .expect("read the line from the slave");

        assert_eq!(&buffer, b"c\n");
    }

    #[test]
    fn pty_descriptors_are_close_on_exec() {
        let pty = PtyPair::open().expect("open a pty pair");
        let master = pty.master().as_raw_fd();
        let slave = pty.slave.as_ref().expect("the slave is open").as_raw_fd();

        for (name, fd) in [("master", master), ("slave", slave)] {
            // SAFETY: `fd` is a valid open descriptor and `F_GETFD` takes no
            // argument.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert!(flags >= 0, "could not read the {name} descriptor flags");
            assert_ne!(
                flags & libc::FD_CLOEXEC,
                0,
                "the {name} descriptor is close-on-exec"
            );
        }
    }

    #[test]
    fn pty_sets_the_slave_to_raw_mode() {
        let mut pty = PtyPair::open().expect("open a pty pair");
        pty.write_line("x").expect("write the line to the master");

        let mut slave = pty.take_slave().expect("take the slave");
        wait_readable(&slave, "the slave");
        let mut buffer = [0u8; 2];
        slave
            .read_exact(&mut buffer)
            .expect("read the line from the slave");
        assert_eq!(&buffer, b"x\n");

        let mut descriptor = libc::pollfd {
            fd: pty.master().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `descriptor` points to one initialized `pollfd` and the
        // finite timeout keeps `poll` from blocking indefinitely.
        let ready = unsafe { libc::poll(&mut descriptor, 1, 100) };
        assert_eq!(ready, 0, "the master must not receive echoed input");
    }
}
