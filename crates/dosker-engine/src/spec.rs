//! Engine inputs and command results.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dosker_core::ContainerId;

/// The pinned inputs an adapter needs to launch an engine for one container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineSpec {
    pub container_id: ContainerId,
    pub workdir: String,
    pub env: BTreeMap<String, String>,
    pub cmd: Option<String>,
    pub expects_agent: bool,
    pub log_dir: PathBuf,
}

/// A command to run inside the guest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecRequest {
    pub command: String,
    pub env: BTreeMap<String, String>,
    pub workdir: String,
    pub timeout_ms: u64,
}

/// The outcome of a guest command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecResult {
    pub exit_code: i32,
    pub output: String,
    pub timed_out: bool,
    pub truncated: bool,
}
