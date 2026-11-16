//! `RUN --host` execution over a staged tree.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use dosker_image::{
    Attributes, DosName, DosPath, LayerDelta, Tree, TreeDir, TreeFile, TreeNode, fixed_timestamp,
};
use dosker_platform::process::{ProcessSupervisor, system};

#[cfg(windows)]
use dosker_platform::process::JobObject;

use crate::error::BuildError;
use crate::normalize::is_reserved_path;
use crate::reporter::BuildReporter;

/// One `RUN --host` command.
pub struct HostRunRequest<'a> {
    /// The Doskerfile line that holds the instruction.
    pub line: usize,
    /// The command passed to the host shell.
    pub command: &'a str,
    /// The image workdir, exported as `DOSKER_WORKDIR`.
    pub workdir: &'a DosPath,
    /// The image environment, exported as `DOSKER_ENV_<NAME>`.
    pub env: &'a BTreeMap<String, String>,
    /// The maximum time the command may run.
    pub timeout: Duration,
}

/// Stages the tree, runs the command, and returns the tree delta.
///
/// The staged tree is created below `work_root` and removed before the
/// function returns. The input tree is not modified.
pub fn run_host(
    tree: &Tree,
    request: &HostRunRequest<'_>,
    work_root: &Path,
    reporter: &mut dyn BuildReporter,
) -> Result<LayerDelta, BuildError> {
    fs::create_dir_all(work_root).map_err(|source| BuildError::Io {
        path: work_root.to_path_buf(),
        source,
    })?;
    let staging = tempfile::Builder::new()
        .prefix("dosker-build-")
        .tempdir_in(work_root)
        .map_err(|source| BuildError::Io {
            path: work_root.to_path_buf(),
            source,
        })?;
    let staged_root = staging.path();

    stage_tree(tree, staged_root)?;
    let outcome = execute(request, staged_root)?;

    reporter
        .output(&outcome.stdout)
        .map_err(BuildError::Reporter)?;
    reporter
        .output(&outcome.stderr)
        .map_err(BuildError::Reporter)?;

    if outcome.timed_out {
        return Err(instruction_error(
            request.line,
            failure_reason(
                format!("timed out after {} seconds", seconds(request.timeout)),
                &outcome.stdout,
                &outcome.stderr,
            ),
        ));
    }
    if !outcome.status.success() {
        return Err(instruction_error(
            request.line,
            failure_reason(
                exit_description(outcome.status),
                &outcome.stdout,
                &outcome.stderr,
            ),
        ));
    }

    let staged = scan_tree(staged_root, request.line)?;
    build_delta(tree, &staged)
}

struct CapturedRun {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: ExitStatus,
    timed_out: bool,
}

fn execute(request: &HostRunRequest<'_>, staged_root: &Path) -> Result<CapturedRun, BuildError> {
    let mut command = Command::new(shell_program());
    command
        .arg(shell_flag())
        .arg(request.command)
        .current_dir(staged_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env_clear();
    apply_environment(&mut command, request, staged_root);
    system()
        .prepare_engine(&mut command)
        .map_err(BuildError::from)?;

    let mut child = command.spawn().map_err(|source| {
        instruction_error(
            request.line,
            format!("failed to start the host shell: {source}"),
        )
    })?;

    let mut guard = match arm_timeout_guard(&child) {
        Ok(guard) => guard,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| instruction_error(request.line, "the host shell has no stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| instruction_error(request.line, "the host shell has no stderr"))?;
    let stdout_reader = thread::spawn(move || read_stream(stdout));
    let stderr_reader = thread::spawn(move || read_stream(stderr));

    let deadline = Instant::now() + request.timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    guard.terminate()?;
                    break child.wait().map_err(|source| {
                        instruction_error(
                            request.line,
                            format!("failed to wait for the host shell: {source}"),
                        )
                    })?;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(source) => {
                let _ = guard.terminate();
                let _ = child.wait();
                return Err(instruction_error(
                    request.line,
                    format!("failed to wait for the host shell: {source}"),
                ));
            }
        }
    };
    if !timed_out {
        guard.terminate()?;
    }

    let stdout = join_output(stdout_reader, request.line)?;
    let stderr = join_output(stderr_reader, request.line)?;
    Ok(CapturedRun {
        stdout,
        stderr,
        status,
        timed_out,
    })
}

fn stage_tree(tree: &Tree, staged_root: &Path) -> Result<(), BuildError> {
    for dir in tree.dirs() {
        let path = host_path(staged_root, &dir.path);
        fs::create_dir_all(&path).map_err(|source| BuildError::Io {
            path: path.clone(),
            source,
        })?;
    }
    for file in tree.files() {
        let path = host_path(staged_root, &file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| BuildError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(&path, &file.bytes).map_err(|source| BuildError::Io {
            path: path.clone(),
            source,
        })?;
    }
    Ok(())
}

fn host_path(root: &Path, path: &DosPath) -> PathBuf {
    let mut result = root.to_path_buf();
    for component in path.components() {
        result.push(component);
    }
    result
}

enum StagedNode {
    File(Vec<u8>),
    Dir,
}

impl StagedNode {
    fn matches(&self, node: &TreeNode) -> bool {
        matches!(
            (self, node),
            (Self::File(_), TreeNode::File(_)) | (Self::Dir, TreeNode::Dir(_))
        )
    }
}

fn scan_tree(staged_root: &Path, line: usize) -> Result<BTreeMap<DosPath, StagedNode>, BuildError> {
    let mut entries = BTreeMap::new();
    let root = DosPath::parse("")?;
    scan_directory(staged_root, &root, &mut entries, line)?;
    Ok(entries)
}

fn scan_directory(
    directory: &Path,
    directory_path: &DosPath,
    entries: &mut BTreeMap<DosPath, StagedNode>,
    line: usize,
) -> Result<(), BuildError> {
    let mut children = fs::read_dir(directory)
        .map_err(|source| BuildError::Io {
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| BuildError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        let file_name = child.file_name();
        let name = file_name.to_str().ok_or_else(|| {
            instruction_error(
                line,
                format!(
                    "the staged path {} has an invalid name",
                    child.path().display()
                ),
            )
        })?;
        let normalized = DosName::normalize(name)
            .map_err(|_| instruction_error(line, format!("invalid DOS 8.3 name {name}")))?;
        let path = directory_path.join(&normalized);
        if is_reserved_path(&path) {
            continue;
        }

        let file_type = child.file_type().map_err(|source| BuildError::Io {
            path: child.path(),
            source,
        })?;
        if file_type.is_symlink() {
            return Err(instruction_error(
                line,
                format!("symlink {} is not allowed", path.to_dos_string()),
            ));
        }
        if file_type.is_dir() {
            insert_staged(entries, path.clone(), StagedNode::Dir, line)?;
            scan_directory(&child.path(), &path, entries, line)?;
        } else if file_type.is_file() {
            let bytes = fs::read(child.path()).map_err(|source| BuildError::Io {
                path: child.path(),
                source,
            })?;
            insert_staged(entries, path, StagedNode::File(bytes), line)?;
        } else {
            return Err(instruction_error(
                line,
                format!("{} is not a file or directory", path.to_dos_string()),
            ));
        }
    }
    Ok(())
}

fn insert_staged(
    entries: &mut BTreeMap<DosPath, StagedNode>,
    path: DosPath,
    node: StagedNode,
    line: usize,
) -> Result<(), BuildError> {
    if entries.insert(path.clone(), node).is_some() {
        return Err(instruction_error(
            line,
            format!("duplicate DOS path {}", path.to_dos_string()),
        ));
    }
    Ok(())
}

fn build_delta(
    tree: &Tree,
    staged: &BTreeMap<DosPath, StagedNode>,
) -> Result<LayerDelta, BuildError> {
    let mut delta = LayerDelta::new();

    let changes: Vec<DosPath> = tree
        .iter()
        .filter(|(path, _)| !is_reserved_path(path))
        .filter_map(|(path, node)| match staged.get(path) {
            Some(staged_node) if staged_node.matches(node) => None,
            _ => Some(path.clone()),
        })
        .collect();
    let changed_paths: BTreeSet<DosPath> = changes.iter().cloned().collect();
    for path in changes {
        if is_topmost(&path, &changed_paths) {
            delta.whiteout(path);
        }
    }

    for (path, staged_node) in staged {
        if is_reserved_path(path) {
            continue;
        }
        match staged_node {
            StagedNode::File(bytes) => {
                let unchanged = matches!(
                    tree.get(path),
                    Some(TreeNode::File(existing)) if existing.bytes == *bytes
                );
                if !unchanged {
                    delta.add_file(TreeFile {
                        path: path.clone(),
                        attributes: Attributes {
                            archive: true,
                            ..Attributes::default()
                        },
                        timestamp: fixed_timestamp(),
                        bytes: bytes.clone(),
                    });
                }
            }
            StagedNode::Dir => {
                let unchanged = matches!(tree.get(path), Some(TreeNode::Dir(_)));
                if !unchanged {
                    delta.add_dir(TreeDir {
                        path: path.clone(),
                        attributes: Attributes::default(),
                        timestamp: fixed_timestamp(),
                    });
                }
            }
        }
    }

    Ok(delta)
}

fn is_topmost(path: &DosPath, changed_paths: &BTreeSet<DosPath>) -> bool {
    let mut parent = path.parent();
    while let Some(current) = parent {
        if changed_paths.contains(&current) {
            return false;
        }
        parent = current.parent();
    }
    true
}

fn apply_environment(command: &mut Command, request: &HostRunRequest<'_>, staged_root: &Path) {
    for name in host_allowlist() {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env("DOSKER_ROOT", staged_root);
    command.env("DOSKER_WORKDIR", request.workdir.to_dos_string());
    for (name, value) in request.env {
        command.env(format!("DOSKER_ENV_{name}"), value);
    }
}

fn read_stream(mut stream: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_output(
    reader: thread::JoinHandle<io::Result<Vec<u8>>>,
    line: usize,
) -> Result<Vec<u8>, BuildError> {
    reader
        .join()
        .map_err(|_| instruction_error(line, "the output reader panicked"))?
        .map_err(|source| {
            instruction_error(line, format!("failed to read the command output: {source}"))
        })
}

fn failure_reason(base: String, stdout: &[u8], stderr: &[u8]) -> String {
    match output_excerpt(stdout, stderr) {
        Some(excerpt) if !excerpt.is_empty() => format!("{base}: {excerpt}"),
        _ => base,
    }
}

fn output_excerpt(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    let mut output = stdout.to_vec();
    output.extend_from_slice(stderr);
    if output.is_empty() {
        return None;
    }
    let start = output.len().saturating_sub(4096);
    Some(
        String::from_utf8_lossy(&output[start..])
            .trim_end()
            .to_owned(),
    )
}

fn exit_description(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("command exited with code {code}"),
        None => format!("command exited with {status}"),
    }
}

fn seconds(timeout: Duration) -> String {
    let value = timeout.as_secs_f64();
    if value.fract() == 0.0 {
        format!("{}", value as u64)
    } else {
        format!("{value}")
    }
}

fn instruction_error(line: usize, reason: impl Into<String>) -> BuildError {
    BuildError::Instruction {
        line,
        instruction: "RUN --host".to_owned(),
        reason: reason.into(),
    }
}

#[cfg(unix)]
struct TimeoutGuard {
    pgid: u32,
}

#[cfg(windows)]
struct TimeoutGuard {
    job: Option<JobObject>,
}

#[cfg(unix)]
fn arm_timeout_guard(child: &Child) -> Result<TimeoutGuard, BuildError> {
    Ok(TimeoutGuard { pgid: child.id() })
}

#[cfg(windows)]
fn arm_timeout_guard(child: &Child) -> Result<TimeoutGuard, BuildError> {
    let job = JobObject::kill_on_close()?;
    job.assign_child(child)?;
    Ok(TimeoutGuard { job: Some(job) })
}

impl TimeoutGuard {
    fn terminate(&mut self) -> Result<(), BuildError> {
        #[cfg(unix)]
        {
            system().kill_group(self.pgid)?;
        }
        #[cfg(windows)]
        {
            self.job.take();
        }
        Ok(())
    }
}

#[cfg(unix)]
fn shell_program() -> &'static str {
    "/bin/sh"
}

#[cfg(windows)]
fn shell_program() -> &'static str {
    "cmd.exe"
}

#[cfg(unix)]
fn shell_flag() -> &'static str {
    "-c"
}

#[cfg(windows)]
fn shell_flag() -> &'static str {
    "/C"
}

#[cfg(unix)]
fn host_allowlist() -> &'static [&'static str] {
    &[
        "PATH", "HOME", "TMPDIR", "USER", "LANG", "LC_ALL", "LC_CTYPE",
    ]
}

#[cfg(windows)]
fn host_allowlist() -> &'static [&'static str] {
    &[
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "COMSPEC",
    ]
}
