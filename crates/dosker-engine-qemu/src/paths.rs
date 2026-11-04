//! QEMU endpoint paths for one container.

use std::io::{self, ErrorKind};
use std::path::PathBuf;

use dosker_engine::EngineSpec;
use dosker_platform::endpoint::engine_socket_path;

/// The four host endpoints QEMU exposes for one container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QemuPaths {
    pub agent_serial: PathBuf,
    pub qmp: PathBuf,
    pub monitor: PathBuf,
    pub gdb: PathBuf,
}

impl QemuPaths {
    /// Resolves every endpoint for a spec.
    pub fn for_spec(spec: &EngineSpec) -> Self {
        Self {
            agent_serial: engine_socket_path(&spec.root, &spec.container_id, "agent"),
            qmp: engine_socket_path(&spec.root, &spec.container_id, "qmp"),
            monitor: engine_socket_path(&spec.root, &spec.container_id, "monitor"),
            gdb: engine_socket_path(&spec.root, &spec.container_id, "gdb"),
        }
    }

    /// Removes every socket file that is left over from an earlier run.
    ///
    /// A missing socket is not an error; any other removal failure is
    /// returned to the caller.
    pub fn remove_stale(&self) -> io::Result<()> {
        for path in [&self.agent_serial, &self.qmp, &self.monitor, &self.gdb] {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }

        Ok(())
    }
}
