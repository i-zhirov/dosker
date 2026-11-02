#![allow(dead_code)]

//! Shared test support for the agent protocol tests.

pub mod mock_agent;

use std::path::PathBuf;

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
#[cfg(unix)]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use dosker_agent_proto::AgentClient;

pub const INTEGRATION_ENV: &str = "DOSKER_INTEGRATION";
pub const AGENT_IMAGE_ENV: &str = "DOSKER_AGENT_IMAGE";
pub const QEMU_ENV: &str = "DOSKER_QEMU";
pub const BOOT_TIMEOUT_ENV: &str = "DOSKER_QEMU_BOOT_TIMEOUT_MS";
pub const DEFAULT_QEMU: &str = "qemu-system-i386";
pub const DEFAULT_BOOT_TIMEOUT_MS: u64 = 60_000;

#[cfg(unix)]
const SOCKET_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(unix)]
const CONNECT_INTERVAL: Duration = Duration::from_millis(50);

#[cfg(unix)]
static SOCKET_COUNTER: AtomicU32 = AtomicU32::new(0);

pub fn integration_enabled() -> bool {
    matches!(std::env::var(INTEGRATION_ENV).as_deref(), Ok("1"))
}

pub fn agent_image() -> Result<PathBuf, String> {
    match std::env::var_os(AGENT_IMAGE_ENV) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err(format!(
            "{AGENT_IMAGE_ENV} is not set; build the fixture with \
             agent/tools/build-fixture.sh"
        )),
    }
}

#[cfg(unix)]
pub fn qemu_binary() -> String {
    std::env::var(QEMU_ENV)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_QEMU.to_owned())
}

#[cfg(unix)]
pub fn boot_timeout() -> Duration {
    let milliseconds = std::env::var(BOOT_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_BOOT_TIMEOUT_MS);

    Duration::from_millis(milliseconds)
}

#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    let counter = SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);

    PathBuf::from(format!(
        "/tmp/dosker-agent-{}-{counter}.sock",
        std::process::id()
    ))
}

#[cfg(unix)]
pub fn qemu_arguments(image: &Path, socket: &Path) -> Vec<String> {
    vec![
        "-machine".to_owned(),
        "pc".to_owned(),
        "-cpu".to_owned(),
        "486".to_owned(),
        "-m".to_owned(),
        "16".to_owned(),
        "-display".to_owned(),
        "none".to_owned(),
        "-no-reboot".to_owned(),
        "-drive".to_owned(),
        format!("file={},format=raw,if=ide,index=0", image.display()),
        "-serial".to_owned(),
        "none".to_owned(),
        "-serial".to_owned(),
        format!("unix:{},server=on,wait=off", socket.display()),
    ]
}

/// A QEMU process booting a private copy of the agent fixture.
#[cfg(unix)]
pub struct QemuAgent {
    child: Child,
    stream: Option<UnixStream>,
    socket: PathBuf,
    log_path: PathBuf,
    dir: PathBuf,
}

#[cfg(unix)]
impl QemuAgent {
    pub fn start() -> Result<Self, String> {
        let image = agent_image()?;
        let qemu = qemu_binary();
        let timeout = boot_timeout();

        let dir = tempfile::Builder::new()
            .prefix("dosker-agent-")
            .tempdir()
            .map_err(|error| format!("failed to create a temporary directory: {error}"))?;
        let dir_path = dir.path().to_path_buf();
        let image_copy = dir_path.join("dosker-agent.img");

        std::fs::copy(&image, &image_copy).map_err(|error| {
            format!(
                "failed to copy agent image {} to {}: {error}",
                image.display(),
                image_copy.display()
            )
        })?;

        let socket = socket_path();
        let _ = std::fs::remove_file(&socket);

        let log_path = dir_path.join("qemu.log");
        let log = File::create(&log_path)
            .map_err(|error| format!("failed to create {}: {error}", log_path.display()))?;
        let log_err = log
            .try_clone()
            .map_err(|error| format!("failed to clone {}: {error}", log_path.display()))?;

        let mut child = Command::new(&qemu)
            .args(qemu_arguments(&image_copy, &socket))
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .spawn()
            .map_err(|error| format!("failed to start {qemu}: {error}"))?;

        let deadline = Instant::now() + timeout;
        let stream = loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let _ = std::fs::remove_file(&socket);
                    let output = std::fs::read_to_string(&log_path).unwrap_or_default();

                    return Err(format!(
                        "QEMU exited with {status} before opening {}:\n{output}",
                        socket.display()
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(&socket);

                    return Err(format!("failed to poll QEMU: {error}"));
                }
            }

            match UnixStream::connect(&socket) {
                Ok(stream) => break stream,
                Err(error) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = std::fs::remove_file(&socket);
                        let output = std::fs::read_to_string(&log_path).unwrap_or_default();

                        return Err(format!(
                            "QEMU did not open {} within {timeout:?}: {error}\n{output}",
                            socket.display()
                        ));
                    }

                    std::thread::sleep(CONNECT_INTERVAL);
                }
            }
        };

        if let Err(error) = stream
            .set_read_timeout(Some(SOCKET_TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(SOCKET_TIMEOUT)))
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&socket);

            return Err(format!("failed to configure the agent socket: {error}"));
        }

        let dir = dir.keep();

        Ok(Self {
            child,
            stream: Some(stream),
            socket,
            log_path,
            dir,
        })
    }

    pub fn connect(&mut self) -> Result<AgentClient<UnixStream>, String> {
        let stream = self
            .stream
            .take()
            .ok_or_else(|| "the agent connection was already taken".to_owned())?;

        Ok(AgentClient::new(stream))
    }

    pub fn log(&self) -> std::io::Result<String> {
        std::fs::read_to_string(&self.log_path)
    }
}

#[cfg(unix)]
impl Drop for QemuAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
