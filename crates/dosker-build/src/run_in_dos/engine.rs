//! An in-DOS runner that drives the engine adapter contract.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use dosker_core::{ContainerId, EngineName};
use dosker_engine::{DisplayMode, EngineAdapter, EngineError, EngineSpec, ExecRequest};
use dosker_engine_qemu::QemuAdapter;
use dosker_platform::fs::ensure_dir_mode;
use dosker_platform::process::{ProcessSupervisor, system};
#[cfg(unix)]
use dosker_platform::{engine_socket_path, short_socket_dir};

use super::{InDosRunOutcome, InDosRunRequest, InDosRunner, RUN_STOP_GRACE};
use crate::error::BuildError;

/// The mode of the directory that holds the engine sockets.
const SOCKET_DIR_MODE: u32 = 0o700;

/// The reason an engine other than QEMU cannot run an in-DOS command.
const UNSUPPORTED_ENGINE_REASON: &str = "in-DOS RUN supports only the qemu engine";

/// Opens an engine adapter for one in-DOS command.
pub trait EngineOpener {
    /// Returns a probed-ready adapter for the selected engine.
    fn open(&mut self, engine: EngineName) -> Result<Box<dyn EngineAdapter>, BuildError>;
}

/// Opens the QEMU adapter and rejects every other engine.
#[derive(Clone, Copy, Debug, Default)]
pub struct QemuEngineOpener;

impl EngineOpener for QemuEngineOpener {
    fn open(&mut self, engine: EngineName) -> Result<Box<dyn EngineAdapter>, BuildError> {
        if engine != EngineName::Qemu {
            return Err(BuildError::Engine {
                engine: engine.as_str().to_owned(),
                reason: UNSUPPORTED_ENGINE_REASON.to_owned(),
            });
        }

        let processes: Arc<dyn ProcessSupervisor> = Arc::new(system());
        QemuAdapter::discover(processes)
            .map(|adapter| Box::new(adapter) as Box<dyn EngineAdapter>)
            .map_err(|error| BuildError::Engine {
                engine: "qemu".to_owned(),
                reason: error.to_string(),
            })
    }
}

/// Runs one command in a transient build VM.
pub struct EngineInDosRunner {
    opener: Box<dyn EngineOpener>,
}

impl EngineInDosRunner {
    /// Creates a runner over the QEMU engine.
    pub fn new() -> Self {
        Self {
            opener: Box::new(QemuEngineOpener),
        }
    }

    /// Creates a runner over a supplied engine opener.
    pub fn with_opener(opener: Box<dyn EngineOpener>) -> Self {
        Self { opener }
    }
}

impl Default for EngineInDosRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl InDosRunner for EngineInDosRunner {
    fn run(&mut self, request: &InDosRunRequest<'_>) -> Result<InDosRunOutcome, BuildError> {
        let line = request.line;

        let mut adapter = self.opener.open(request.engine)?;
        adapter
            .probe()
            .map_err(|error| boot_failure(line, error.to_string()))?;

        let container_id = ContainerId::generate();
        ensure_socket_dirs(request.root, &container_id)
            .map_err(|error| boot_failure(line, error.to_string()))?;

        let spec = EngineSpec {
            root: request.root.to_path_buf(),
            container_id,
            workdir: request.workdir.to_dos_string(),
            env: request.env.clone(),
            cmd: None,
            expects_agent: true,
            log_dir: request.log_dir.to_path_buf(),
            disk_path: request.disk_path.to_path_buf(),
            disk_interface: request.disk_interface.to_owned(),
            serial_ports: request.serial_ports,
            control_port: request.control_port,
            booter: false,
            display: DisplayMode::None,
            vnc_port: None,
            allow_reboot: true,
        };

        let mut session = adapter
            .start(&spec)
            .map_err(|error| boot_failure(line, error.to_string()))?;

        if let Err(error) = session.ready(request.boot_timeout) {
            let reason = ready_failure(request.boot_timeout, &error);
            let _ = session.kill();
            return Err(boot_failure(line, reason));
        }

        let exec = ExecRequest {
            command: request.command.to_owned(),
            env: request.env.clone(),
            workdir: request.workdir.to_dos_string(),
            timeout_ms: u64::try_from(request.timeout.as_millis()).unwrap_or(u64::MAX),
        };
        let result = match session.exec(&exec) {
            Ok(result) => result,
            Err(error) => {
                let _ = session.kill();
                return Err(command_failure(line, error));
            }
        };

        if session.stop(RUN_STOP_GRACE).is_err() {
            let _ = session.kill();
            return Err(boot_failure(line, "the guest did not shut down cleanly"));
        }

        Ok(InDosRunOutcome {
            exit_code: result.exit_code,
            output: result.output,
            truncated: result.truncated,
        })
    }
}

/// Creates the directories the engine sockets bind into.
///
/// A long data root moves some endpoint paths to the short fallback
/// directory. Every socket name is checked because the monitor path is the
/// longest and can fall back while the others still fit.
fn ensure_socket_dirs(root: &Path, id: &ContainerId) -> Result<(), dosker_platform::PlatformError> {
    ensure_dir_mode(&root.join("containers").join(id.as_str()), SOCKET_DIR_MODE)?;

    #[cfg(unix)]
    {
        let short = short_socket_dir();
        for name in ["agent", "qmp", "monitor", "gdb"] {
            if engine_socket_path(root, id, name).starts_with(&short) {
                ensure_dir_mode(&short, SOCKET_DIR_MODE)?;
                break;
            }
        }
    }

    Ok(())
}

fn ready_failure(timeout: Duration, error: &EngineError) -> String {
    match error {
        EngineError::ReadyTimeout(_) => {
            format!(
                "the guest did not boot within {} seconds",
                timeout.as_secs()
            )
        }
        EngineError::Exited(code) => {
            format!("the engine exited with code {code} before the guest was ready")
        }
        other => other.to_string(),
    }
}

fn command_failure(line: usize, error: EngineError) -> BuildError {
    match error {
        EngineError::ExecTimeout(_) => BuildError::RunTimeout { line },
        EngineError::GuestRebooted => BuildError::GuestRebooted { line },
        EngineError::NoAgent => BuildError::RunBootFailed {
            line,
            reason: "the image has no agent".to_owned(),
        },
        other => BuildError::RunBootFailed {
            line,
            reason: other.to_string(),
        },
    }
}

fn boot_failure(line: usize, reason: impl Into<String>) -> BuildError {
    BuildError::RunBootFailed {
        line,
        reason: reason.into(),
    }
}
