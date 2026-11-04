//! Shared helpers for tests that drive the real `dosker` binary.

#![allow(dead_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dosker_core::ContainerId;
use dosker_platform::process::{ProcessSupervisor, system};
use dosker_runtime::StateStore;
use dosker_test_support::fixtures::{kill_process, mock_engine_path, qemu_binary};
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions, format_volume};

const SECTOR_SIZE: u64 = 512;
const PARTITION_TABLE_OFFSET: usize = 446;
const PARTITION_TYPE_FAT16: u8 = 0x06;
const IMAGE_SIZE: u64 = 32 * 1024 * 1024;
const PARTITION_OFFSET: u64 = 2048 * SECTOR_SIZE;
const PARTITION_LENGTH: u64 = IMAGE_SIZE - PARTITION_OFFSET;
const SPAWN_TIMEOUT: Duration = Duration::from_secs(60);
const QEMU_COMMAND_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_STEP: Duration = Duration::from_millis(20);

static CAPTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Creates a 32 MiB MBR-partitioned FAT16 disk image with an `AUTOEXEC.BAT`.
pub fn make_disk_image(path: &Path) {
    let file = File::create(path).expect("create image");
    file.set_len(IMAGE_SIZE).expect("size image");
    write_mbr(path);

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open image");
    let region = Region::new(file, PARTITION_OFFSET, PARTITION_LENGTH);
    let options = FormatVolumeOptions::new()
        .fat_type(FatType::Fat16)
        .total_sectors((PARTITION_LENGTH / SECTOR_SIZE) as u32);
    format_volume(region, options).expect("format FAT16");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open image");
    let region = Region::new(file, PARTITION_OFFSET, PARTITION_LENGTH);
    let fs = FileSystem::new(region, FsOptions::new()).expect("mount FAT");
    {
        let root = fs.root_dir();
        let mut entry = root
            .create_file("AUTOEXEC.BAT")
            .expect("create AUTOEXEC.BAT");
        entry
            .write_all(b"@ECHO OFF\r\n")
            .expect("write AUTOEXEC.BAT");
    }
    fs.unmount().expect("unmount FAT");
}

fn write_mbr(path: &Path) {
    let mut sector = [0u8; SECTOR_SIZE as usize];
    let entry = PARTITION_TABLE_OFFSET;
    sector[entry] = 0x80;
    sector[entry + 4] = PARTITION_TYPE_FAT16;
    let start_lba = (PARTITION_OFFSET / SECTOR_SIZE) as u32;
    let sectors = (PARTITION_LENGTH / SECTOR_SIZE) as u32;
    sector[entry + 8..entry + 12].copy_from_slice(&start_lba.to_le_bytes());
    sector[entry + 12..entry + 16].copy_from_slice(&sectors.to_le_bytes());
    sector[510] = 0x55;
    sector[511] = 0xAA;

    let mut file = OpenOptions::new().write(true).open(path).expect("open MBR");
    file.seek(SeekFrom::Start(0)).expect("seek MBR");
    file.write_all(&sector).expect("write MBR");
}

/// A temporary DOSKER_ROOT with a helper to run the CLI against it.
pub struct CliRoot {
    dir: tempfile::TempDir,
}

impl CliRoot {
    pub fn new() -> Self {
        #[cfg(unix)]
        let dir = tempfile::Builder::new()
            .prefix("ds")
            .tempdir_in("/tmp")
            .expect("create a temporary directory");
        #[cfg(windows)]
        let dir = tempfile::tempdir().expect("create a temporary directory");

        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn store(&self) -> StateStore {
        StateStore::new(self.path().to_path_buf())
    }

    /// Builds a command with only `DOSKER_ROOT` set.
    pub fn base_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dosker"));
        command.env("DOSKER_ROOT", self.path());
        command
    }

    pub fn command(&self) -> Command {
        let mut command = self.base_command();
        command.env(
            "DOSKER_MOCK_ENGINE",
            mock_engine_path().expect("locate the mock engine"),
        );
        command
    }

    /// Builds a command that selects the real QEMU engine.
    ///
    /// The command keeps `DOSKER_ROOT` from [`Self::base_command`] and does
    /// not set `DOSKER_MOCK_ENGINE`.
    pub fn command_with_qemu(&self) -> Command {
        let mut command = self.base_command();
        command.env("DOSKER_QEMU", qemu_binary());
        command
    }

    pub fn run(&self, args: &[&str]) -> Output {
        let mut command = self.command();
        command.args(args);
        run_captured(&mut command, Duration::from_secs(45))
    }

    /// Runs a command against a real QEMU engine.
    pub fn run_with_qemu(&self, args: &[&str]) -> Output {
        let mut command = self.command_with_qemu();
        command.args(args);
        run_captured(&mut command, QEMU_COMMAND_TIMEOUT)
    }

    /// Imports a generated disk image as `freedos:latest` and returns that
    /// reference.
    pub fn import_image(&self) -> String {
        self.import_image_as("freedos:latest")
    }

    /// Imports a generated disk image under an explicit reference.
    pub fn import_image_as(&self, reference: &str) -> String {
        let dir = tempfile::tempdir().expect("create an image source directory");
        let source = dir.path().join("disk.img");
        make_disk_image(&source);
        let output = self.run(&[
            "image",
            "import",
            source.to_str().expect("source path is utf-8"),
            reference,
        ]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "stderr: {}",
            stderr_text(&output)
        );
        reference.to_owned()
    }

    /// Starts a detached container and returns its id.
    pub fn start(&self) -> ContainerId {
        self.import_image();

        let mut command = self.command();
        command.args(["run", "-d", "freedos:latest"]);
        let output = run_captured(&mut command, Duration::from_secs(15));
        assert_eq!(
            output.status.code(),
            Some(0),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let id = stdout_text(&output);
        ContainerId::parse(id.trim()).expect("parse the id printed by run")
    }

    /// Returns the engine PID recorded for a container.
    pub fn engine_pid(&self, id: &ContainerId) -> Option<u32> {
        self.store()
            .load(id)
            .expect("load the container record")
            .engine_pid
    }

    /// Kills the supervisor of a container without stopping its engine.
    pub fn kill_supervisor(&self, id: &ContainerId) {
        let record = self.store().load(id).expect("load the container record");
        if let Some(pid) = record.pid {
            kill_process(pid);
        }
    }

    /// Kills a process if it is still running.
    pub fn kill(&self, pid: Option<u32>) {
        if let Some(pid) = pid {
            kill_process(pid);
        }
    }

    /// Runs a command with scripted stdin and returns its captured output.
    pub fn spawn_with_stdin(&self, mut command: Command, args: &[&str], stdin: &str) -> Output {
        let (dir, stdout_path, stderr_path) = capture_files("stdin");
        let stdout = File::create(&stdout_path).expect("create stdout capture");
        let stderr = File::create(&stderr_path).expect("create stderr capture");
        let mut child = command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn the dosker binary");

        {
            let mut pipe = child.stdin.take().expect("stdin is piped");
            pipe.write_all(stdin.as_bytes())
                .expect("write scripted stdin");
            pipe.flush().expect("flush scripted stdin");
        }

        let status = wait_for_exit(&mut child, SPAWN_TIMEOUT);
        let stdout = fs::read(&stdout_path).unwrap_or_default();
        let stderr = fs::read(&stderr_path).unwrap_or_default();
        drop(dir);
        Output {
            status,
            stdout,
            stderr,
        }
    }

    /// Runs a command with scripted stdin chunks, each written after its delay.
    pub fn spawn_with_stdin_chunks(
        &self,
        mut command: Command,
        args: &[&str],
        chunks: &[(Duration, &str)],
    ) -> Output {
        let (dir, stdout_path, stderr_path) = capture_files("chunks");
        let stdout = File::create(&stdout_path).expect("create stdout capture");
        let stderr = File::create(&stderr_path).expect("create stderr capture");
        let mut child = command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn the dosker binary");

        {
            let mut pipe = child.stdin.take().expect("stdin is piped");
            for (delay, chunk) in chunks {
                std::thread::sleep(*delay);
                pipe.write_all(chunk.as_bytes())
                    .expect("write scripted stdin chunk");
                pipe.flush().expect("flush scripted stdin chunk");
            }
        }

        let status = wait_for_exit(&mut child, SPAWN_TIMEOUT);
        let stdout = fs::read(&stdout_path).unwrap_or_default();
        let stderr = fs::read(&stderr_path).unwrap_or_default();
        drop(dir);
        Output {
            status,
            stdout,
            stderr,
        }
    }

    /// Spawns a long-running command with file-backed output captures.
    ///
    /// The caller owns the returned [`tempfile::TempDir`] and must keep it
    /// alive while the child runs and the capture files are read.
    pub fn spawn_captured(
        &self,
        mut command: Command,
        args: &[&str],
    ) -> (Child, PathBuf, PathBuf, tempfile::TempDir) {
        let (dir, stdout_path, stderr_path) = capture_files("follow");
        let stdout = File::create(&stdout_path).expect("create stdout capture");
        let stderr = File::create(&stderr_path).expect("create stderr capture");
        let child = command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn the dosker binary");
        (child, stdout_path, stderr_path, dir)
    }
}

impl Drop for CliRoot {
    fn drop(&mut self) {
        let Ok(records) = self.store().list() else {
            return;
        };
        let processes = system();
        for record in records {
            if let Some(pgid) = record.engine_pgid {
                let _ = processes.kill_group(pgid);
            }
            if let Some(pid) = record.engine_pid {
                kill_process(pid);
            }
            if let Some(pid) = record.pid {
                kill_process(pid);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn capture_files(label: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let counter = CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let prefix = format!("dosker-{label}-{}-{counter}", std::process::id());
    let dir = tempfile::Builder::new()
        .prefix(&prefix)
        .tempdir()
        .expect("create a capture directory");
    let stdout = dir.path().join("stdout");
    let stderr = dir.path().join("stderr");
    (dir, stdout, stderr)
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("poll the dosker binary") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {timeout:?}");
        }
        std::thread::sleep(POLL_STEP);
    }
}

/// Runs a command with file-backed stdout/stderr and a hard timeout.
///
/// A detached child must not inherit the capture pipes; files avoid a hang
/// when the supervisor outlives the `run` process.
pub fn run_captured(command: &mut Command, timeout: Duration) -> Output {
    let dir = tempfile::tempdir().expect("create a capture directory");
    let out_path = dir.path().join("stdout");
    let err_path = dir.path().join("stderr");
    let stdout = fs::File::create(&out_path).expect("create stdout capture");
    let stderr = fs::File::create(&err_path).expect("create stderr capture");

    let mut child = command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn the dosker binary");

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the dosker binary") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let stdout = fs::read(&out_path).unwrap_or_default();
            let stderr = fs::read(&err_path).unwrap_or_default();
            panic!(
                "command timed out after {timeout:?}; stdout: {}; stderr: {}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let stdout = fs::read(&out_path).expect("read stdout capture");
    let stderr = fs::read(&err_path).expect("read stderr capture");
    Output {
        status,
        stdout,
        stderr,
    }
}

pub fn stdout_text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is utf-8")
}

pub fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A read/write/seek view over one region of a file.
struct Region {
    file: File,
    base: u64,
    length: u64,
    position: u64,
}

impl Region {
    fn new(file: File, base: u64, length: u64) -> Self {
        Self {
            file,
            base,
            length,
            position: 0,
        }
    }

    fn remaining(&self) -> u64 {
        self.length.saturating_sub(self.position)
    }
}

impl Read for Region {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let limit = self.remaining().min(buf.len() as u64) as usize;
        if limit == 0 {
            return Ok(0);
        }
        self.file.seek(SeekFrom::Start(self.base + self.position))?;
        let read = self.file.read(&mut buf[..limit])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Write for Region {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let limit = self.remaining().min(buf.len() as u64) as usize;
        if limit == 0 && !buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "write past the end of the region",
            ));
        }
        self.file.seek(SeekFrom::Start(self.base + self.position))?;
        let written = self.file.write(&buf[..limit])?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Region {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::End(offset) => self.length as i128 + i128::from(offset),
            SeekFrom::Current(offset) => self.position as i128 + i128::from(offset),
        };
        self.position = target.clamp(0, self.length as i128) as u64;
        Ok(self.position)
    }
}
