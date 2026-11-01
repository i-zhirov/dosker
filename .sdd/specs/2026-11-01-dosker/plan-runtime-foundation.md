# Runtime Foundation Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-01
- **Status**: In Progress
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Roadmap**: `.sdd/specs/2026-11-01-dosker/roadmap.md` — slice: `runtime-foundation`
- **Depends on**: —
- **Manual prerequisites**: Rust toolchain. A Windows CI runner for the
  named-pipe contract tests. No QEMU and no DOS toolchain.
- **Goal**: Build the host runtime every later slice stands on — workspace, domain types, the platform transport and cleanup seams, the atomic state store, the per-container supervisor, the mock engine, and the CLI — and prove a detached mock container lifecycle without QEMU, DOS, or a 16-bit toolchain.

## Constraints

- Supported hosts: Linux x86_64, macOS arm64, Windows x86_64.
- Control endpoint: Unix socket on Linux and macOS, named pipe on Windows.
- Unix socket path limit: 100 bytes. Use `/tmp/dosker-<euid>/<id>.sock` when the default path is longer.
- Process cleanup: parent-death signal or process group on Unix, Job Object on Windows. Slice 1 implements the seam and the Unix side. The Windows Job Object is out of scope.
- Data roots: XDG data directory, Application Support, `%LOCALAPPDATA%\dosker`.
- `DOSKER_ROOT` overrides the data root.
- State writes: `rename` on Unix, `ReplaceFileW` on Windows.
- Control messages: UTF-8 JSON, one object per line.
- Container IDs: 12 lowercase hex characters.
- Container names: `[a-z0-9][a-z0-9_.-]{0,62}`.
- Lifecycle states: created, starting, running, stopping, stopped, exited, failed, removed.
- Stale state: missing supervisor PID and dead control endpoint mark the container `exited`.
- Unix socket directory mode: 0700.
- Windows named pipe: DACL for the current user only.
- Container directory mode: 0700.
- Logs: engine log, serial log, container log.
- Supervisor: `dosker __supervise`. The CLI exits after it starts a detached container.
- No central daemon.

## Technical Context

**Language and toolchain.** Rust 1.98.1, edition 2024, one virtual
workspace. `rust-toolchain.toml` pins the channel and the `rustfmt` and
`clippy` components. The runtime is synchronous and uses the standard
library for I/O and threads; the only product platform FFI lives inside
`dosker-platform`, and the test fixtures may call the OS directly for
process control.

**Crate graph.** Files live under `crates/`:

| Crate | Depends on | Responsibility |
| --- | --- | --- |
| `dosker-core` | serde, thiserror, getrandom | Container IDs, names, lifecycle states, validation errors |
| `dosker-platform` | dosker-core, serde, thiserror, libc (Unix), windows-sys (Windows) | Data root, control transport (Unix socket and named pipe), atomic writes, console, process cleanup and spawn flags |
| `dosker-engine` | dosker-core, serde, thiserror | `EngineAdapter`, `EngineSession`, `EngineSpec`, capability set, exec types, engine errors |
| `dosker-runtime` | dosker-core, dosker-platform, dosker-engine, serde, serde_json, chrono, thiserror | Container record, state store, control protocol, stale recovery, supervisor |
| `dosker-test-support` | dosker-core, dosker-platform, dosker-engine, serde_json, tempfile, libc (Unix), windows-sys (Windows) | Mock engine binary, mock adapter, fixtures, transport contract suite |
| `dosker-cli` | clap, dosker-core, dosker-platform, dosker-engine, dosker-runtime, serde_json, chrono, thiserror; optional dosker-test-support | `dosker` binary: run, ps, status, exec, stop, hidden `__supervise` |

`dosker-test-support` MUST NOT depend on `dosker-runtime`; the runtime
uses it only as a dev-dependency, which keeps the graph acyclic.

**Dependency versions.** `serde = { version = "1", features = ["derive"] }`,
`serde_json = "1"`, `thiserror = "2"`, `getrandom = "0.3"`,
`chrono = { version = "0.4", features = ["serde"] }`,
`clap = { version = "4", features = ["derive"] }`, `tempfile = "3"`,
`libc = "0.2"`, and `windows-sys = { version = "0.59", features =
["Win32_Foundation", "Win32_Storage_FileSystem", "Win32_System_Pipes",
"Win32_Security", "Win32_Security_Authorization",
"Win32_System_SystemServices", "Win32_System_Threading",
"Win32_System_IO"] }`. `libc` is a
`[target.'cfg(unix)'.dependencies]` entry and `windows-sys` is a
`[target.'cfg(windows)'.dependencies]` entry. The workspace table also
holds the six path dependencies: `dosker-core = { path = "crates/dosker-core" }`,
`dosker-platform = { path = "crates/dosker-platform" }`,
`dosker-engine = { path = "crates/dosker-engine" }`,
`dosker-runtime = { path = "crates/dosker-runtime" }`,
`dosker-test-support = { path = "crates/dosker-test-support" }`, and
`dosker-cli = { path = "crates/dosker-cli" }`.

**In-scope requirements.** FR-008 (per-container supervisor, no central
daemon), FR-019 (stale-state recovery on the next CLI call), FR-022
(transport abstraction), and the Slice 1 part of FR-023 (cleanup seam
plus the Unix implementation). FR-023 is split: the Windows Job Object
with kill-on-close is Slice 4 and is not built here. The Windows side of
this slice uses creation flags only, and `ProcessSupervisor::kill_group`
returns `Unsupported` on Windows.

**Data layout under the resolved root.**

```text
<root>/
  containers/
    .names/<name>/id          one line: the container id
    <id>/
      state.json
      engine.log
      serial.log
      container.log
      control.sock            Unix only
```

`state.json` is written atomically. The container directory is 0700; its
parents `containers/` and `containers/.names/` are 0700. The Unix socket
file is 0600. The Windows control endpoint is the pipe name
`\\.\pipe\dosker-<id>-control` and has no on-disk artifact.

When the default Unix control socket path exceeds
`MAX_UNIX_SOCKET_PATH_BYTES`, the record stores
`/tmp/dosker-<euid>/<id>.sock` instead.

**Control protocol.** One UTF-8 JSON object per line. A request is one
object with a `type` field; a response is one object with `ok` plus
either `result` or `error`. The supervisor serves one connection at a
time and reads frames until the client closes, the read timeout fires, or
the container stops. A line is at most 65536 bytes excluding the
terminator. `\n` and `\r\n` both terminate a line; blank lines are
ignored.

| Request | Body | Response `result` |
| --- | --- | --- |
| ping | `{"type":"ping"}` | `{"type":"pong"}` |
| status | `{"type":"status"}` | `{"type":"status","container":{...}}` |
| exec | `{"type":"exec","command":"ECHO HELLO","env":{},"workdir":"C:\\","timeout_ms":30000}` | `{"type":"exec","exit_code":0,"output":"HELLO\r\n","timed_out":false,"truncated":false}` |
| stop | `{"type":"stop","grace_ms":10000}` | `{"type":"stop","status":"stopped"}` |

An error response is
`{"ok":false,"error":{"code":"unknown_message","message":"..."}}`.
Codes: `unknown_message`, `invalid_message`, `not_running`,
`engine_error`, `timeout`, `internal`. A malformed JSON line closes the
connection without a response; an unknown `type` returns
`unknown_message` and keeps the connection open; a known `type` with a
bad body returns `invalid_message` and closes the connection.

**Pinned values.**

| Value | Setting |
| --- | --- |
| Default readiness timeout | 60 s |
| Supervisor accept poll | 50 ms |
| Supervisor request read timeout | 5 s |
| CLI connect timeout | 5 s |
| `run -d` wait for ping | 65 s |
| Default exec timeout | 30 s |
| Default stop grace | 10 s |
| Maximum control line | 65536 bytes |
| Maximum Unix socket path | 100 bytes |
| Named-pipe buffer size | 65536 bytes each direction |
| Mock engine version | `0.1.0` |
| Mock engine forced-start-failure exit | 3 |
| CLI exit: success | 0 |
| CLI exit: runtime or store error | 1 |
| CLI exit: usage or invalid name/id | 2 |
| CLI exit: exec timed out | 124 |
| CLI exit: container not running or unreachable | 125 |

**The mock is a fixture.** The mock engine and the mock adapter are test
fixtures. They are not a product engine: the product binary includes the
mock adapter only behind the `dosker-cli` `mock-engine` feature, and the
mock engine exists only to prove the supervisor, the transport, the state
store, and the CLI without QEMU, DOS, or a 16-bit toolchain.

**Deliberate subset of the data model.** This slice has no image store,
so `ContainerRecord.image` holds the unresolved image reference and
`ContainerRecord.disk_path` is `None`; the image store adds `image_digest`
and the private disk. `ContainerRecord.sockets` is a map with the single
key `control`; the engine sockets are added with the first real engine.
`EngineAdapter` exposes the lifecycle subset (`name`, `version`,
`capabilities`, `probe`, `start`); `build_command`, `screenshot`, and
`debug_endpoint` arrive with the QEMU adapter. `EngineSpec` carries only
the fields the mock needs (`container_id`, `workdir`, `env`, `cmd`,
`expects_agent`, `log_dir`); `MachineProfile` and engine endpoints arrive
with the real engine. Exec output `truncated` is always `false` until the
agent's 1 MiB limit lands.

## Tasks

- [x] **Task 1: Workspace and core domain types**
    - **Implements:** FR-008 and FR-019 supporting types; design §Workspace Crates; data-model §Container, §Validation Rules, §Container Lifecycle.
    - **Create `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`**
        - `Cargo.toml` — virtual workspace with `resolver = "2"`, the six members under `crates/`, `[workspace.package]` (`version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.98"`, `license = "MIT OR Apache-2.0"`), and the workspace dependency table from Technical Context.
        - `rust-toolchain.toml` — `channel = "1.98.1"`, `components = ["rustfmt", "clippy"]`, `profile = "minimal"`.
        - `.gitignore` — `/target/`, `.DS_Store`, `*.tmp`. Do not ignore `Cargo.lock`; the workspace ships a binary.
    - **Create the five sibling crate skeletons** — `crates/dosker-platform`, `crates/dosker-engine`, `crates/dosker-runtime`, `crates/dosker-test-support`, and `crates/dosker-cli`, each with a `Cargo.toml` inheriting `version.workspace = true`, `edition.workspace = true`, `license.workspace = true`; the library crates get a `src/lib.rs` with only a crate doc comment, and the CLI gets a `src/main.rs` with `fn main() {}`; no module is declared until the task that owns it.
    - **Create `crates/dosker-core/Cargo.toml` and `crates/dosker-core/src/lib.rs`**
        - `lib.rs` — `pub mod id; pub mod name; pub mod status; pub mod error;` and re-exports `ContainerId`, `ContainerName`, `Status`, `CoreError`.
        - `Cargo.toml` — serde, thiserror, getrandom, and `serde_json` as a dev-dependency.
    - **Create `crates/dosker-core/tests/id.rs`**
        - `generated_id_is_twelve_lowercase_hex` — 1000 generated IDs are 12 characters of `0-9a-f` and parse back.
        - `generated_ids_are_unique` — 1000 generated IDs produce 1000 distinct values.
        - `parse_accepts_lowercase_hex` — `0123456789ab` is accepted and `as_str` returns it.
        - `parse_rejects_uppercase`, `parse_rejects_wrong_length`, `parse_rejects_non_hex`, `parse_rejects_whitespace` — each returns `CoreError::InvalidContainerId`.
        - `serde_round_trip` — the JSON form is the bare 12-character string; an invalid string fails deserialization.
    - **Create `crates/dosker-core/src/id.rs`**
        - `ContainerId(String)` — newtype with `#[serde(try_from = "String", into = "String")]`, deriving `Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize`.
        - `ContainerId::generate() -> Self` — 6 bytes from `getrandom::fill`, lowercase hex.
        - `ContainerId::parse(value: &str) -> Result<Self, CoreError>` — exactly 12 chars, each `0-9` or `a-f`.
        - `ContainerId::as_str(&self) -> &str`; `Display`; `TryFrom<String>`; `From<ContainerId> for String`.
    - **Create `crates/dosker-core/tests/name.rs`**
        - `parse_accepts_single_character` — `a`.
        - `parse_accepts_max_length_63`.
        - `parse_rejects_length_64`.
        - `parse_rejects_uppercase`.
        - `parse_rejects_leading_dash`, `parse_rejects_leading_dot`, `parse_rejects_leading_underscore`, `parse_rejects_space` — the first character must be `[a-z0-9]`.
        - `parse_accepts_dots_dashes_underscores_after_first` — `a.b-c_d`.
        - `serde_round_trip` — bare string form; an invalid name fails deserialization.
    - **Create `crates/dosker-core/src/name.rs`**
        - `ContainerName(String)` — same newtype, serde attributes, and derives as `ContainerId`.
        - `ContainerName::parse(value: &str) -> Result<Self, CoreError>` — 1 to 63 bytes; first char `[a-z0-9]`; remaining chars `[a-z0-9_.-]`; ASCII only.
        - `ContainerName::as_str(&self) -> &str`; `Display`; `TryFrom<String>`; `From<ContainerName> for String`.
    - **Create `crates/dosker-core/tests/status.rs`**
        - `every_allowed_transition_is_accepted` — created→starting, created→removed, starting→running, starting→exited, starting→failed, running→stopping, running→exited, stopping→stopped, stopping→exited, stopped→starting, stopped→removed, exited→starting, exited→removed, failed→removed.
        - `forbidden_transitions_are_rejected` — created→running, starting→stopped, running→created, stopped→running, failed→starting, removed→created.
        - `active_states` — starting, running, and stopping are active; the rest are not.
        - `terminal_state` — only removed is terminal.
        - `serializes_snake_case` — every variant becomes `created`, `starting`, `running`, `stopping`, `stopped`, `exited`, `failed`, `removed`.
    - **Create `crates/dosker-core/src/status.rs`**
        - `Status` — `Created, Starting, Running, Stopping, Stopped, Exited, Failed, Removed`, deriving `Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize` with `rename_all = "snake_case"`.
        - `Status::can_transition_to(self, next: Status) -> bool` — the table above.
        - `Status::is_active(self) -> bool`; `Status::is_terminal(self) -> bool`; `Status::as_str(self) -> &'static str`; `impl Display for Status` returning `as_str`.
    - **Create `crates/dosker-core/src/error.rs`**
        - `CoreError` — `InvalidContainerId(String)`, `InvalidContainerName(String)`, `InvalidTransition { from: Status, to: Status }`, deriving `Debug, thiserror::Error` with messages `invalid container id: {0}`, `invalid container name: {0}`, `invalid transition from {from} to {to}`.
    - **Verify:** `cargo test -p dosker-core`

- [x] **Task 2: Platform primitives — data root, endpoints, atomic files, console**
    - **Implements:** FR-019 (atomic state writes), FR-022 (endpoint naming), design §Host Platform Abstraction, §Security And Privacy; data-model §Validation Rules.
    - **Create `crates/dosker-platform/Cargo.toml` and `crates/dosker-platform/src/lib.rs`**
        - `lib.rs` — `pub mod console; pub mod data_root; pub mod endpoint; pub mod error; pub mod fs;` and re-exports `Endpoint`, `EndpointRecord`, `PlatformError`.
        - `Cargo.toml` — dosker-core, serde, thiserror; `libc` under `[target.'cfg(unix)'.dependencies]`; `windows-sys` under `[target.'cfg(windows)'.dependencies]` with `Win32_Foundation` and `Win32_Storage_FileSystem` for `write_atomic`; `tempfile` and `serde_json` as dev-dependencies.
    - **Create `crates/dosker-platform/tests/data_root.rs`**
        - `dosker_root_overrides_every_host` — with `DOSKER_ROOT` set to a temp directory path, `data_root()` returns its absolute form on every host.
        - `dosker_root_relative_is_made_absolute` — a relative `DOSKER_ROOT` is returned joined to the current directory.
        - `dosker_root_empty_falls_back` — an empty value behaves as unset.
        - `linux_uses_xdg_data_home` (`cfg(target_os = "linux")`) — `$XDG_DATA_HOME/dosker`.
        - `linux_falls_back_to_home_local_share` (`cfg(target_os = "linux")`) — `$HOME/.local/share/dosker` when `XDG_DATA_HOME` is unset or empty.
        - `macos_uses_application_support` (`cfg(target_os = "macos")`) — `$HOME/Library/Application Support/dosker`.
        - `windows_uses_localappdata` (`cfg(windows)`) — `%LOCALAPPDATA%\dosker`.
        - `missing_home_is_an_error` (`cfg(unix)`) — with `HOME` and the host-specific variable unset, `data_root()` returns `PlatformError::MissingEnvironment` naming the variable.
        - All tests in this file take one file-local `static ENV_LOCK: Mutex<()>` and restore every variable they touch through a guard, because the environment is process-global. On edition 2024, the guard uses `unsafe { std::env::set_var(...) }` and restores the variables on drop.
    - **Create `crates/dosker-platform/src/data_root.rs`**
        - `pub const ROOT_ENV: &str = "DOSKER_ROOT";`
        - `pub fn data_root() -> Result<PathBuf, PlatformError>` — the precedence and values above; the result is passed through `std::path::absolute`.
        - Linux helpers `xdg_data_home() -> Result<PathBuf, PlatformError>` and macOS helper `application_support() -> Result<PathBuf, PlatformError>`.
    - **Create `crates/dosker-platform/src/error.rs`**
        - `PlatformError` — `Io { path: PathBuf, source: io::Error }`, `IoPlain(#[from] io::Error)`, `MissingEnvironment(&'static str)`, `UnsupportedEndpoint`, `Unsupported(&'static str)`, `InvalidEndpoint(String)`, deriving `Debug, thiserror::Error`; messages name the path, the variable, or the operation.
    - **Create `crates/dosker-platform/tests/endpoint.rs`**
        - `unix_endpoint_uses_container_directory` (`cfg(unix)`) — `<root>/containers/<id>/control.sock`.
        - `named_pipe_name_uses_container_id` (`cfg(windows)`) — `\\.\pipe\dosker-<id>-control`.
        - `endpoint_record_round_trip` — `Endpoint` to `EndpointRecord` and back, for the host-native variant.
        - `endpoint_record_serializes_with_kind_tag` — `{"kind":"unix_socket","path":"..."}` on Unix and `{"kind":"named_pipe","name":"..."}` on Windows.
    - **Create `crates/dosker-platform/src/endpoint.rs`**
        - `Endpoint` — `UnixSocket(PathBuf)` or `NamedPipe(String)`, deriving `Clone, Debug, PartialEq, Eq`.
        - `EndpointRecord` — `#[serde(tag = "kind", rename_all = "snake_case")]` enum with `UnixSocket { path: PathBuf }` and `NamedPipe { name: String }`, deriving the same.
        - `pub fn control_endpoint(root: &Path, id: &ContainerId) -> Endpoint` — the path or pipe name above, selected with `#[cfg]`.
        - `Endpoint::record(&self) -> EndpointRecord`; `Endpoint::display(&self) -> String`; `impl From<EndpointRecord> for Endpoint`.
    - **Create `crates/dosker-platform/tests/atomic_write.rs`**
        - `write_atomic_creates_file_with_contents` — a missing target is created with the exact bytes and no trailing change.
        - `write_atomic_replaces_existing_contents` — a shorter write fully replaces a longer previous file.
        - `write_atomic_leaves_no_temp_file` — the directory contains only the target after the call.
        - `concurrent_readers_never_observe_partial_content` — one writer alternates two valid JSON payloads of different lengths 100 times while a reader loops; every successful read parses as one of the two payloads, never empty or truncated.
        - `ensure_dir_mode_sets_owner_only_permissions` (`cfg(unix)`) — mode `0o700` after the call.
        - `ensure_dir_mode_is_idempotent` — a second call succeeds and leaves the mode unchanged.
    - **Create `crates/dosker-platform/src/fs.rs`**
        - `pub fn ensure_dir_mode(path: &Path, mode: u32) -> Result<(), PlatformError>` — `create_dir_all`, then on Unix `set_permissions` with the given mode; on Windows the mode argument is accepted and ignored.
        - `pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PlatformError>` — temp file `<dir>/.<name>.tmp.<pid>` in the same directory, write, `sync_all`, then `rename` and parent-directory `sync_all` on Unix, or `ReplaceFileW` with `REPLACEFILE_IGNORE_MERGE_ERRORS` on Windows, falling back to `MoveFileExW(tmp, dst, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` when the target does not exist.
    - **Create `crates/dosker-platform/tests/console.rs`**
        - `line_appends_newline` — `line("a")` writes `a\n` to stdout and flushes.
        - `error_line_uses_stderr` — `error_line("e")` writes only to the error writer.
        - `raw_writes_without_newline` — `raw("x")` writes `x` and flushes.
        - `broken_pipe_is_ignored` — a writer that returns `ErrorKind::BrokenPipe` makes every method return `Ok(())`.
    - **Create `crates/dosker-platform/src/console.rs`**
        - `Console` — holds `Box<dyn Write + Send>` for stdout and stderr.
        - `Console::std() -> Self`; `Console::new(stdout: Box<dyn Write + Send>, stderr: Box<dyn Write + Send>) -> Self`.
        - `Console::line(&mut self, text: &str) -> io::Result<()>`; `error_line(&mut self, text: &str) -> io::Result<()>`; `raw(&mut self, text: &str) -> io::Result<()>`. Each flushes; `BrokenPipe` is swallowed.
    - **Verify:** `cargo test -p dosker-platform`

- [x] **Task 3: Control transport abstraction, Unix socket backend, and transport contract suite**
    - **Implements:** FR-022 (Unix side); design §Host Platform Abstraction, §Control Endpoint Encoding, §Security And Privacy; data-model §Validation Rules.
    - **Create `crates/dosker-test-support/tests/transport_contract.rs`** — one contract suite that both native backends run. The factory is `fn endpoint() -> (tempfile::TempDir, Endpoint)`, building `control_endpoint(dir.path(), &ContainerId::generate())` so parallel tests never collide.
        - `round_trip_single_request` — server accepts, client writes `ping\n`, server reads the line and writes `pong\n`, client reads it.
        - `multiple_sequential_connections` — three connect/round-trip cycles on one listener.
        - `accept_returns_none_when_no_client_connects` — `accept(100ms)` returns `Ok(None)` within 2 s.
        - `accept_returns_connection_after_client_connects` — `accept(2s)` returns `Some`.
        - `server_read_times_out_on_idle_client` — server read timeout 100 ms; an idle client makes the read return `WouldBlock` or `TimedOut`.
        - `client_read_times_out_when_server_stays_silent` — client read timeout 100 ms; no server write makes the read return `WouldBlock` or `TimedOut`.
        - `connect_to_missing_endpoint_fails` — a fresh endpoint with no listener returns an error within 2 s.
        - `endpoint_alive_is_false_for_missing_endpoint` and `endpoint_alive_is_true_for_bound_endpoint`.
        - `large_payload_round_trip` — 1 MiB written and read back byte-for-byte, with the write on its own thread.
        - `socket_and_directory_permissions_are_owner_only` (`cfg(unix)`) — the socket parent directory is 0700 and the socket file is 0600.
        - `bind_fails_when_the_endpoint_is_live` (`cfg(unix)`) — a second `bind` on a path with a live listener returns an error; the endpoint ID is unique to the test so the first listener can stay up.
        - `pipe_is_connectable_by_current_user` (`cfg(windows)`) — bind, connect, and round-trip one line.
    - **Modify `crates/dosker-test-support/Cargo.toml`** — add dosker-core, dosker-platform, and tempfile.
    - **Create `crates/dosker-platform/src/control/mod.rs` and `crates/dosker-platform/src/control/unix.rs`; add `pub mod control;` to `src/lib.rs`**
        - `pub trait ControlConnection: Read + Write + Send` with `set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>`, `set_write_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>`, `shutdown(&mut self) -> io::Result<()>`.
        - `pub trait ControlListener: Send` with `accept(&self, timeout: Duration) -> io::Result<Option<Box<dyn ControlConnection>>>`.
        - `pub fn bind(endpoint: &Endpoint) -> Result<Box<dyn ControlListener>, PlatformError>` — on Unix, ensure the parent directory is 0700, remove an existing socket file only when `connect` with a 100 ms budget fails, bind, set the socket file mode 0600, set the listener non-blocking, and spawn an acceptor thread that pushes accepted connections into an unbounded `mpsc` channel; `accept` is `recv_timeout`, with `Timeout` mapped to `Ok(None)`.
        - `pub fn connect(endpoint: &Endpoint, timeout: Duration) -> Result<Box<dyn ControlConnection>, PlatformError>` — on Unix, retry `UnixStream::connect` every 20 ms until the deadline on `NotFound` and `ConnectionRefused`, then return a connection with no read or write timeout set.
        - `pub fn endpoint_alive(endpoint: &Endpoint) -> bool` — `connect` with a 200 ms budget, then drop the connection.
        - `pub fn remove_endpoint(endpoint: &Endpoint) -> Result<(), PlatformError>` — on Unix remove the socket file, treating `NotFound` as success; on Windows a no-op.
        - `UnixConnection` — wraps `UnixStream`; `Read`, `Write`, timeouts, and `shutdown(Both)`.
        - On Unix, bind must fail with `PlatformError::Io` carrying `AddrInUse` when a live listener already owns the path. Windows allows several instances of one pipe name by design; the supervisor is the single instance per container.
    - **Verify:** `cargo test -p dosker-test-support --test transport_contract`

- [x] **Task 3A: Unix short-path socket fallback**
    - **Implements:** FR-022 (the transport remains usable under a long data root), FR-019 (the recorded endpoint is the one stale recovery checks), design §Host Platform Abstraction, §Security And Privacy; constraints “Unix socket path limit: 100 bytes” and “Unix socket directory mode: 0700”.
    - **Create `crates/dosker-platform/tests/short_socket.rs`** — all tests are `#[cfg(unix)]`; the Windows named-pipe name is unchanged.
        - `short_default_path_is_used_when_it_fits` — a short root yields `<root>/containers/<id>/control.sock`.
        - `fallback_path_is_used_when_default_exceeds_limit` — a root with a 200-byte path yields a path under `/tmp/dosker-<euid>/`.
        - `fallback_path_is_at_most_100_bytes` — the fallback path length is <= 100 bytes.
        - `fallback_is_unique_per_id` — two IDs yield different paths.
        - `fallback_round_trips_a_line` — `bind`, `connect`, write `ping\n`, read `pong\n`, then `remove_endpoint`.
        - `fallback_dir_is_owner_only` — the fallback directory mode is 0700 after `bind`.
        - `fallback_socket_file_is_owner_only` — the socket file mode is 0600.
        - `remove_endpoint_removes_the_fallback` — `endpoint_alive` is false and the file is gone after `remove_endpoint`.
        - `fallback_endpoint_record_round_trips` — the fallback path survives `Endpoint::record` and `From<EndpointRecord>`.
        - `default_endpoint_with_a_long_root_is_not_used` — the returned path is not the over-limit default.
    - **Modify `crates/dosker-platform/src/endpoint.rs`**
        - `pub const MAX_UNIX_SOCKET_PATH_BYTES: usize = 100;` (Unix only).
        - `pub fn short_socket_dir() -> PathBuf` — `/tmp/dosker-<euid>`; if `/tmp` is not a directory, `std::env::temp_dir().join(format!("dosker-{euid}"))`; `euid` is `libc::geteuid()`.
        - `pub fn short_socket_path(id: &ContainerId) -> PathBuf` — `short_socket_dir().join(format!("{id}.sock"))`.
        - `control_endpoint(root, id)` — on Unix, compute the default path; return it when `unix_path_len(&default) <= MAX_UNIX_SOCKET_PATH_BYTES`; otherwise return `short_socket_path(id)`. On Windows the named-pipe name is unchanged.
        - `fn unix_path_len(path: &Path) -> usize` — `path.as_os_str().as_bytes().len()`.
    - **Modify `crates/dosker-platform/src/lib.rs`** — re-export `short_socket_dir`, `short_socket_path`, and `MAX_UNIX_SOCKET_PATH_BYTES` under `#[cfg(unix)]`.
    - **Modify `crates/dosker-test-support/tests/transport_contract.rs`**
        - `long_root_uses_the_short_fallback` (`cfg(unix)`) — create a temp dir, append a 200-byte component, create the directory, call `control_endpoint`, assert the fallback path, and run one `ping`/`pong` round-trip.
    - **Verify:** `cargo test -p dosker-platform --test short_socket && cargo test -p dosker-test-support --test transport_contract`

- [ ] **Task 4: Windows named-pipe backend**
    - **Implements:** FR-022 (Windows side); design §Host Platform Abstraction, §Security And Privacy; constraints “Control endpoint: … named pipe on Windows” and “Windows named pipe: DACL for the current user only”.
    - **Add Windows-only contract checks to `crates/dosker-test-support/tests/transport_contract.rs`**
        - `pipe_acl_has_one_ace_for_current_user` (`cfg(windows)`) — open the bound pipe by name with `CreateFileW(..., GENERIC_READ | GENERIC_WRITE | READ_CONTROL, ...)`, call `GetSecurityInfo(handle, SE_KERNEL_OBJECT, DACL_SECURITY_INFORMATION, ...)`, assert `GetAclInformation` reports one ACE, `GetAce` returns an access-allowed ACE, and `EqualSid` matches the current user's token SID.
        - The other contract tests run unchanged against the pipe backend.
    - **Create `crates/dosker-platform/src/control/windows.rs`; add the `#[cfg(windows)]` module and dispatchers in `control/mod.rs`**
        - `PipeListener` — wraps `mpsc::Receiver<io::Result<Box<dyn ControlConnection>>>`; an acceptor thread creates one pipe instance at a time with `CreateNamedPipeW(name, PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, &security_attributes)`, blocks in `ConnectNamedPipe` with an `OVERLAPPED` event, treats `ERROR_PIPE_CONNECTED` as connected, pushes the handle, and creates the next instance.
        - Security attributes — a `SECURITY_DESCRIPTOR` built with `ConvertStringSecurityDescriptorToSecurityDescriptorW` from the SDDL `D:P(A;;GA;;;<sid>)`, where `<sid>` is the current user's SID string from `OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY)`, `GetTokenInformation(TokenUser)`, and `ConvertSidToStringSidW`; `SECURITY_ATTRIBUTES { nLength, lpSecurityDescriptor, bInheritHandle: FALSE }`.
        - `PipeConnection` — overlapped `ReadFile` and `WriteFile` with a per-call event; `set_read_timeout` and `set_write_timeout` store the duration and a timed wait calls `CancelIoEx` and then `GetOverlappedResult` with the wait flag, returning `ErrorKind::TimedOut`; `shutdown` is a no-op; dropping closes the handle.
        - `connect` — `WaitNamedPipeW(name, timeout_ms)` then `CreateFileW(name, GENERIC_READ | GENERIC_WRITE, 0, null, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, null)`, retrying while `ERROR_PIPE_BUSY` until the deadline.
        - `endpoint_alive` — `connect` with a 200 ms budget, then drop.
        - `remove_endpoint` — no-op.
    - **Modify `crates/dosker-platform/Cargo.toml`** — verify the `windows-sys` target dependency carries `Win32_System_Pipes`, `Win32_Security`, `Win32_Security_Authorization`, `Win32_System_Threading`, and `Win32_System_IO` from the workspace table; add any missing feature for the named-pipe backend, DACL, and process liveness.
    - **Modify `crates/dosker-test-support/Cargo.toml`** — verify `windows-sys` is present under `[target.'cfg(windows)'.dependencies]` with `Win32_Foundation`, `Win32_Security`, `Win32_Security_Authorization`, `Win32_Storage_FileSystem`, `Win32_System_SystemServices`, and `Win32_System_Threading` for the ACL check; add any missing feature.
    - **Verify:** `cargo test -p dosker-test-support --test transport_contract` on the Windows runner (the same command runs the Unix backend on Linux and macOS). Also run `cargo test -p dosker-platform` locally to prove the Unix build is unaffected.
    - **Status note:** implemented and reviewed by inspection; the checkbox stays open until the Windows matrix leg passes.

- [x] **Task 5: Process-cleanup seam, Unix cleanup, and detached spawn**
    - **Implements:** FR-023 Slice 1 part — the seam and the Unix implementation; the Windows Job Object is Slice 4. Design §Process Model, §Host Platform Abstraction.
    - **Create `crates/dosker-platform/tests/process.rs`**
        - `engine_child_gets_its_own_process_group` (`cfg(unix)`) — spawn `sleep 30` through `prepare_engine`; `process_group(child.id())` is `Some(child.id())`.
        - `kill_group_terminates_every_member` (`cfg(unix)`) — a shell starts `sleep 30 &` and writes its PID to a file before `wait`; `kill_group(leader_pid)` makes both PIDs dead within 2 s.
        - `kill_group_is_idempotent` (`cfg(unix)`) — a second kill of the same group returns `Ok(())`.
        - `process_alive_is_false_after_reap` — spawn and wait on a trivial child, then `process_alive(pid)` is false.
        - `process_alive_is_true_for_current_process` — `process_alive(std::process::id())` is true on every host.
        - `kill_group_is_unsupported_on_windows` (`cfg(windows)`) — returns `PlatformError::Unsupported`.
    - **Create `crates/dosker-platform/src/process/mod.rs`, `src/process/unix.rs`, and `src/process/windows.rs`; add `pub mod process;` to `src/lib.rs`**
        - `pub trait ProcessSupervisor: Send + Sync` with `prepare_detached(&self, cmd: &mut Command) -> Result<(), PlatformError>`, `prepare_engine(&self, cmd: &mut Command) -> Result<(), PlatformError>`, `kill_group(&self, pgid: u32) -> Result<(), PlatformError>`, `process_alive(&self, pid: u32) -> bool`, and `process_group(&self, pid: u32) -> Result<Option<u32>, PlatformError>`.
        - `pub struct SystemProcesses;` implementing the trait; `pub fn system() -> SystemProcesses`; derive `Clone, Copy, Debug, Default`.
        - Unix `prepare_detached` — `pre_exec` calls `setsid()`.
        - Unix `prepare_engine` — `pre_exec` calls `setpgid(0, 0)` on both Linux and macOS; on Linux it then calls `prctl(PR_SET_PDEATHSIG, SIGKILL)` and exits with code 1 if `getppid() == 1`.
        - Unix `kill_group` — `kill(-(pgid as i32), SIGKILL)`; `ESRCH` is success.
        - Unix `process_alive` — `kill(pid, 0)`; `ESRCH` is false, `EPERM` is true.
        - Unix `process_group` — `getpgid(pid)`; `ESRCH` is `Ok(None)`.
        - Windows `prepare_detached` and `prepare_engine` — `creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)` with values `0x08000000` and `0x00000200`.
        - Windows `kill_group` — `Err(PlatformError::Unsupported("process group cleanup"))`.
        - Windows `process_alive` — `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid)`; null with `ERROR_INVALID_PARAMETER` is false, `ERROR_ACCESS_DENIED` is true; otherwise `GetExitCodeProcess` compared with `STILL_ACTIVE`, then `CloseHandle`.
        - Windows `process_group` — `Ok(None)`.
    - **Verify:** `cargo test -p dosker-platform --test process`

- [x] **Task 6: Engine trait, spec, capability set, and exec types**
    - **Implements:** design §Engine Adapter, §Engine Capability Matrix; FR-008 (adapter seam used by the supervisor).
    - **Modify `crates/dosker-engine/Cargo.toml` and `crates/dosker-engine/src/lib.rs`; create the four module files**
        - `lib.rs` — `pub mod adapter; pub mod capability; pub mod error; pub mod spec;` and re-exports `EngineAdapter`, `EngineSession`, `EngineSpec`, `ExecRequest`, `ExecResult`, `Capability`, `CapabilitySet`, `EngineError`.
        - `Cargo.toml` — dosker-core, serde, thiserror; `serde_json` as a dev-dependency.
    - **Create `crates/dosker-engine/tests/capability.rs`**
        - `from_name_round_trips_every_variant` — every `Capability` has a stable name and parses back.
        - `names_match_the_documented_names` — the hard-coded name list matches the design and Doskerfile contract, including `gdbstub` for `GdbStub`.
        - `unknown_name_is_none` — `"nope"` returns `None`.
        - `contains_reports_only_inserted` — insert `Boot` and `Serial`; `Stop` is absent.
        - `empty_contains_nothing`.
        - `iter_yields_inserted_in_declaration_order` — `Boot`, `Serial`, `Stop` in enum order.
        - `set_serializes_as_u16` — `CapabilitySet` serializes as a number.
    - **Create `crates/dosker-engine/src/capability.rs`**
        - `Capability` — `Boot, Serial, Stop, Headless, HostDirLive, DebugNative, GdbStub, Screenshot, Vnc, Nat, Audio`, deriving `Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize` with `rename_all = "snake_case"`; `Capability::ALL: [Capability; 11]`.
        - `Capability::from_name(name: &str) -> Option<Self>`; `Capability::name(self) -> &'static str` — the stable name; `GdbStub` is `gdbstub` to match the design and Doskerfile contract.
        - `CapabilitySet(u16)` with `#[serde(transparent)]`, deriving `Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize`; `EMPTY`, `insert`, `contains`, `iter`, `names`; `FromIterator<Capability>`.
    - **Create `crates/dosker-engine/tests/adapter.rs`**
        - `adapter_is_object_safe` — a stub adapter is stored as `Box<dyn EngineAdapter>`, `start` returns `Box<dyn EngineSession>`, and every trait method is callable.
        - `spec_holds_the_pinned_fields` — a constructed `EngineSpec` exposes `container_id`, `workdir`, `env`, `cmd`, `expects_agent`, and `log_dir`.
        - `exec_result_equality_covers_all_fields`.
    - **Create `crates/dosker-engine/src/spec.rs`**
        - `EngineSpec` — fields `container_id: ContainerId`, `workdir: String`, `env: BTreeMap<String, String>`, `cmd: Option<String>`, `expects_agent: bool`, `log_dir: PathBuf`; deriving `Clone, Debug, PartialEq, Eq`.
        - `ExecRequest` — `command: String`, `env: BTreeMap<String, String>`, `workdir: String`, `timeout_ms: u64`; deriving `Clone, Debug, PartialEq, Eq`.
        - `ExecResult` — `exit_code: i32`, `output: String`, `timed_out: bool`, `truncated: bool`; deriving `Clone, Debug, PartialEq, Eq`.
    - **Create `crates/dosker-engine/src/adapter.rs`**
        - `pub trait EngineAdapter: Send` with `name(&self) -> &str`, `version(&self) -> &str`, `capabilities(&self) -> CapabilitySet`, `probe(&mut self) -> Result<(), EngineError>`, `start(&mut self, spec: &EngineSpec) -> Result<Box<dyn EngineSession>, EngineError>`.
        - `pub trait EngineSession: Send` with `pid(&self) -> Option<u32>`, `process_group(&self) -> Option<u32>`, `ready(&mut self, timeout: Duration) -> Result<(), EngineError>`, `exec(&mut self, request: &ExecRequest) -> Result<ExecResult, EngineError>`, `stop(&mut self, grace: Duration) -> Result<i32, EngineError>`, `kill(&mut self) -> Result<i32, EngineError>`, `try_exit(&mut self) -> Result<Option<i32>, EngineError>`.
    - **Create `crates/dosker-engine/src/error.rs`**
        - `EngineError` — `Io(#[from] io::Error)`, `NotFound(PathBuf)`, `Setup(String)`, `Protocol(String)`, `ReadyTimeout(Duration)`, `ExecTimeout(Duration)`, `StopTimeout(Duration)`, `Exited(i32)`, deriving `Debug, thiserror::Error`; messages name the path, duration, exit code, or protocol detail.
    - **Verify:** `cargo test -p dosker-engine`

- [x] **Task 7: Container record, runtime paths, and atomic state store**
    - **Implements:** FR-019 (state read by the next CLI call), FR-008 (per-container state directory), constraints “Container directory mode: 0700”, “Logs: engine log, serial log, container log”, “Container IDs: 12 lowercase hex characters”, “Container names: …”.
    - **Modify `crates/dosker-runtime/Cargo.toml` and `crates/dosker-runtime/src/lib.rs`; create the three module files**
        - `lib.rs` — `pub mod paths; pub mod record; pub mod store;` and re-exports `ContainerRecord`, `RuntimePaths`, `StateStore`, `StoreError`, `CONTROL_SOCKET`.
        - `Cargo.toml` — dosker-core, dosker-platform, dosker-engine, serde, serde_json, chrono, thiserror; `dosker-test-support` and `tempfile` as dev-dependencies.
    - **Create `crates/dosker-runtime/tests/record.rs`**
        - `new_record_starts_created` — status `created`, schema version 1, `engine_version` empty, `disk_path` `None`, `dirty` false, `pid`, `engine_pid`, `engine_pgid`, `exit_code`, `started_at`, and `finished_at` all `None`.
        - `new_record_inserts_the_control_socket` — `sockets["control"]` equals the supplied record.
        - `transition_sets_started_at_once` — `Running` sets `started_at`; a later `Running` does not overwrite it.
        - `transition_sets_finished_at_for_outcomes` — `Stopped`, `Exited`, and `Failed` each set `finished_at`.
        - `transition_repeated_outcome_keeps_finished_at` — repeating `Exited` does not move `finished_at`.
        - `transition_allows_starting_to_exited` — a stale `starting` record becomes `exited` and sets `finished_at`.
        - `transition_rejects_a_forbidden_edge` — `Created -> Running` returns `CoreError::InvalidTransition`.
        - `serde_round_trip_preserves_every_field`.
        - `unknown_fields_are_ignored` — a `state.json` with an extra key loads.
    - **Create `crates/dosker-runtime/src/record.rs`**
        - `pub const CONTROL_SOCKET: &str = "control";` and `pub const RECORD_SCHEMA_VERSION: u32 = 1;`.
        - `ContainerRecord` — `schema_version: u32`, `id: ContainerId`, `name: ContainerName`, `image: String`, `engine: String`, `engine_version: String`, `status: Status`, `pid: Option<u32>`, `engine_pid: Option<u32>`, `engine_pgid: Option<u32>` with `#[serde(default)]`, `exit_code: Option<i32>`, `created_at: DateTime<Utc>`, `started_at: Option<DateTime<Utc>>`, `finished_at: Option<DateTime<Utc>>`, `sockets: BTreeMap<String, EndpointRecord>`, `disk_path: Option<PathBuf>`, `dirty: bool`; deriving `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`.
        - `ContainerRecord::new(id: ContainerId, name: ContainerName, image: String, engine: String, control: EndpointRecord, now: DateTime<Utc>) -> Self`.
        - `ContainerRecord::control_endpoint(&self) -> Option<&EndpointRecord>`.
        - `ContainerRecord::transition(&mut self, next: Status, now: DateTime<Utc>) -> Result<(), CoreError>`.
    - **Create `crates/dosker-runtime/tests/store.rs`**
        - `create_writes_state_and_three_logs` — `state.json` loads back, and `engine.log`, `serial.log`, and `container.log` exist and are empty.
        - `create_rejects_duplicate_name_across_ids`.
        - `create_rejects_duplicate_id`.
        - `create_reclaims_a_stale_name_lock` — a name directory whose recorded container has no `state.json` is removed and the name is reused; a missing or unparseable `id` file is also stale.
        - `find_by_name_removes_a_name_dir_whose_container_is_gone`.
        - `list_skips_non_container_entries`.
        - `save_writes_pretty_json_with_trailing_newline`.
        - `load_ignores_unknown_fields_through_the_store`.
        - `load_returns_saved_record`.
        - `load_missing_returns_not_found`.
        - `save_replaces_record` — a status change persists.
        - `list_returns_all_records_sorted_by_created_at_then_id`.
        - `list_returns_empty_when_root_is_absent` — a fresh root is not an error.
        - `find_by_name_returns_match`.
        - `find_by_name_on_missing_root_is_none`.
        - `corrupt_state_file_is_reported_with_path`.
        - `unsupported_schema_version_is_rejected`.
        - `root_containers_and_container_dir_are_owner_only` (`cfg(unix)`) — modes `0o700`.
        - `log_file_names_are_engine_serial_and_container` — exact file names only.
    - **Create `crates/dosker-runtime/src/paths.rs`**
        - `RuntimePaths` — `new(root: PathBuf) -> Self`, `root(&self) -> &Path`, `containers_dir(&self) -> PathBuf`, `names_dir(&self) -> PathBuf`, `container_dir(&self, id: &ContainerId) -> PathBuf`, `name_dir(&self, name: &ContainerName) -> PathBuf`, `state_file(&self, id) -> PathBuf`, `engine_log(&self, id) -> PathBuf`, `serial_log(&self, id) -> PathBuf`, `container_log(&self, id) -> PathBuf`.
        - File names are exactly `state.json`, `engine.log`, `serial.log`, `container.log`.
    - **Create `crates/dosker-runtime/src/store.rs`**
        - `StateStore` — `new(root: PathBuf) -> Self`, `paths(&self) -> &RuntimePaths`, `create(&self, record: &ContainerRecord) -> Result<(), StoreError>`, `load(&self, id: &ContainerId) -> Result<ContainerRecord, StoreError>`, `save(&self, record: &ContainerRecord) -> Result<(), StoreError>`, `list(&self) -> Result<Vec<ContainerRecord>, StoreError>`, `find_by_name(&self, name: &ContainerName) -> Result<Option<ContainerRecord>, StoreError>`.
        - `create` order: ensure root, `containers/`, and `containers/.names/` at 0700; return `AlreadyExists(id)` when `containers/<id>/state.json` already exists; create `containers/.names/<name>/` with `create_dir`; on `AlreadyExists` read `containers/.names/<name>/id`, and if that container has no `state.json` remove the name directory and retry once, otherwise return `NameInUse`; write `containers/.names/<name>/id`; create the container directory and the three logs; write `state.json` last with `write_atomic`; on any failure after the name lock, remove the name directory and the container directory before returning. A name lock whose `id` file is missing or unparseable is stale.
        - `save` writes pretty JSON with a trailing newline through `write_atomic`.
        - `load` rejects `schema_version != RECORD_SCHEMA_VERSION` with `StoreError::Corrupt`.
        - `list` iterates `containers/*` directories that contain `state.json`, skips every other entry, and sorts by `created_at` then `id`.
        - `find_by_name` resolves through the name directory; a name directory whose container is gone is removed and treated as absent.
        - `StoreError` — `NotFound(ContainerId)`, `AlreadyExists(ContainerId)`, `NameInUse(ContainerName)`, `Io { path: PathBuf, source: io::Error }`, `Corrupt { path: PathBuf, message: String }`, `Platform(#[from] PlatformError)`, `Core(#[from] CoreError)`.
    - **Verify:** `cargo test -p dosker-runtime --test record --test store`

- [x] **Task 8: JSON-lines control protocol codec**
    - **Implements:** FR-008 (control surface of the supervisor), FR-019 (status surface), design §Control Endpoint Encoding, §Supervisor.
    - **Create `crates/dosker-runtime/tests/protocol.rs`**
        - `read_frame_returns_one_line` — `ping\n` yields `Line("ping")`.
        - `read_frame_accepts_crlf` — `ping\r\n` yields `Line("ping")`.
        - `read_frame_reads_each_of_two_lines` — `ping\nstatus\n` yields the two lines in order.
        - `read_frame_handles_split_reads` — a reader that returns one byte per `read` still yields the full line.
        - `read_frame_reports_clean_eof` — an empty reader yields `Eof`.
        - `read_frame_skips_blank_lines` — leading `\n` and `\r\n` are skipped.
        - `read_frame_accepts_line_at_limit` — 65536 bytes succeed.
        - `read_frame_accepts_crlf_line_at_limit` — 65536 content bytes plus `\r\n` succeed.
        - `read_frame_rejects_line_over_limit` — 65537 bytes return `FrameError::TooLarge`.
        - `read_frame_rejects_invalid_utf8` — returns `FrameError::NotUtf8`.
        - `parse_request_accepts_each_known_type` — `ping`, `status`, `exec`, `stop`.
        - `parse_request_applies_exec_defaults` — missing `env` is empty, missing `workdir` is `C:\`, missing `timeout_ms` is 30000.
        - `parse_request_applies_stop_default` — missing `grace_ms` is 10000.
        - `parse_request_reports_unknown_type` — missing `type`, `{"type":"attach"}`, and `{"type":"nope"}`.
        - `parse_request_reports_malformed` — `{` and `[1]`.
        - `parse_request_reports_invalid_body` — `{"type":"exec"}` and `{"type":"stop","grace_ms":"x"}`.
        - `response_round_trip_for_each_reply` — `Pong`, `Status`, `Exec`, and `Stop`.
        - `response_error_omits_result` — serialized error has `ok:false` and no `result`.
        - `unknown_message_response_uses_unknown_message_code`.
        - `exec_output_with_newlines_stays_one_physical_line` — output `A\r\nB\r\n` is escaped inside one JSON line.
    - **Create `crates/dosker-runtime/src/protocol.rs`; add `pub mod protocol;` and the protocol re-exports to `src/lib.rs`**
        - Re-export `Request`, `Reply`, `Response`, `ProtocolError`, `ErrorCode`, `ContainerSummary`, `FrameRead`, `FrameError`, `RequestParse`, `read_frame`, `parse_request`, `write_message`, `MAX_MESSAGE_BYTES`, `DEFAULT_EXEC_TIMEOUT_MS`, and `DEFAULT_STOP_GRACE_MS`.
        - `pub const MAX_MESSAGE_BYTES: usize = 65_536;`, `pub const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;`, `pub const DEFAULT_STOP_GRACE_MS: u64 = 10_000;`.
        - `Request` — `#[serde(tag = "type", rename_all = "snake_case")]`: `Ping`, `Status`, `Exec { command: String, env: BTreeMap<String, String>, workdir: String, timeout_ms: u64 }`, `Stop { grace_ms: u64 }`; exec fields use `#[serde(default)]` and the two defaults above.
        - `Reply` — `#[serde(tag = "type", rename_all = "snake_case")]`: `Pong`, `Status { container: ContainerSummary }`, `Exec { exit_code: i32, output: String, timed_out: bool, truncated: bool }`, `Stop { status: Status }`.
        - `ContainerSummary` — `id`, `name`, `image`, `status`, `pid`, `engine_pid`, `exit_code`.
        - `ErrorCode` — `UnknownMessage, InvalidMessage, NotRunning, EngineError, Timeout, Internal` with `rename_all = "snake_case"`.
        - `ProtocolError` — `code: ErrorCode`, `message: String`.
        - `Response` — `ok: bool`, `result: Option<Reply>`, `error: Option<ProtocolError>`, both optional fields skipped when `None`; `Response::ok(reply: Reply) -> Self`, `Response::error(code: ErrorCode, message: impl Into<String>) -> Self`.
        - `FrameRead` — `Line(String)` or `Eof`; `FrameError` — `TooLarge`, `NotUtf8`, `Io(#[from] io::Error)`.
        - `pub fn read_frame<R: BufRead>(reader: &mut R) -> Result<FrameRead, FrameError>` — reads to `\n` with the 65536-byte limit excluding the terminator, strips one trailing `\r`, and skips blank lines.
        - `RequestParse` — `Known(Request)`, `UnknownType`, `Malformed`, `InvalidBody`.
        - `pub fn parse_request(line: &str) -> RequestParse` — parses a JSON value first; a non-object or invalid JSON is `Malformed`; a missing or unrecognized `type` is `UnknownType`; a known type that fails to deserialize is `InvalidBody`.
        - `pub fn write_message<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()>` — compact JSON plus `\n`, then flush.
    - **Verify:** `cargo test -p dosker-runtime --test protocol`

- [x] **Task 9: Stale-state recovery**
    - **Implements:** FR-019, FR-023 (kill the orphaned engine group on Unix); design §Process Model, §Error Handling And Edge Cases; data-model §Container Lifecycle.
    - **Create `crates/dosker-runtime/tests/stale.rs`**
        - `dead_supervisor_marks_running_container_exited`.
        - `dead_endpoint_marks_running_container_exited_even_when_pid_is_alive`.
        - `missing_supervisor_pid_marks_starting_container_exited`.
        - `dead_endpoint_leaves_a_starting_container_untouched_when_the_pid_is_alive` — the endpoint is bound after the `starting` record is written, so startup is not stale.
        - `live_supervisor_and_endpoint_are_left_untouched`.
        - `stale_engine_group_is_killed` — the fake probe records `kill_group(engine_pgid)` exactly once.
        - `kill_group_error_leaves_the_record_untouched` — a kill error is returned and the active record keeps its status and pids.
        - `stale_recovery_sets_dirty_and_clears_pids` — status `exited`, `pid`, `engine_pid`, `engine_pgid`, and `exit_code` are `None`, `finished_at` is set, `dirty` is true.
        - `stale_recovery_returns_only_changed_ids`.
        - `non_active_statuses_are_untouched` — created, stopped, exited, failed, and removed records are left alone.
        - `stale_recovery_is_idempotent` — a second call returns an empty vector.
        - The test-local `FakeProbe` implements `HostProbe` with sets of live PIDs and dead endpoints and a recorded kill list.
    - **Create `crates/dosker-runtime/src/stale.rs`; add `pub mod stale;` to `src/lib.rs` and re-export `HostProbe`, `SystemProbe`, and `recover_stale`**
        - `pub trait HostProbe: Send + Sync` with `process_alive(&self, pid: u32) -> bool`, `endpoint_alive(&self, endpoint: &EndpointRecord) -> bool`, `kill_group(&self, pgid: u32) -> Result<(), PlatformError>`.
        - `pub struct SystemProbe;` implementing `HostProbe` through `SystemProcesses` and `dosker_platform::control::endpoint_alive`; `SystemProbe::new() -> Self`.
        - `pub fn recover_stale(store: &StateStore, probe: &dyn HostProbe, now: DateTime<Utc>) -> Result<Vec<ContainerId>, StoreError>`.
        - Rule: an active record is stale when its supervisor PID is absent or its supervisor process is dead. A `running` or `stopping` record is also stale when its control endpoint is missing or dead; a `starting` record is not judged by the endpoint because the supervisor binds it after writing `starting`. On stale: if `engine_pgid` is set and alive, call `probe.kill_group`; then set status `Exited`, clear `pid`, `engine_pid`, `engine_pgid`, and `exit_code`, set `finished_at` to `now`, set `dirty` true, and save atomically. Non-active records are untouched. A `kill_group` error is returned; the state is not lost.
    - **Verify:** `cargo test -p dosker-runtime --test stale`

- [x] **Task 10: Mock engine child process, mock adapter, and fixtures**
    - **Implements:** design §Testing Strategy (“Adapter contract tests use a mock engine”), §Process Model; FR-023 (a real child process so the orphan test is real); FR-008 and FR-019 (mock lifecycle used by the supervisor and CLI tests).
    - **Create `crates/dosker-test-support/tests/mock_engine.rs`**
        - `mock_engine_reports_ready` — the first stdout line is `{"event":"ready","pid":<pid>}`.
        - `mock_engine_echoes_command` — `{"op":"exec","command":"ECHO HELLO"}` returns exit 0 and output `HELLO\r\n`.
        - `mock_engine_returns_requested_exit_code` — `EXIT 7` returns 7.
        - `mock_engine_unknown_command_succeeds` — anything else returns exit 0 and empty output.
        - `mock_engine_stops_on_stop` — `{"op":"stop"}` exits 0.
        - `mock_engine_survives_stdin_eof` — drop stdin, sleep 200 ms, `try_wait()` is still `None`, then kill it.
        - `mock_engine_forced_start_failure_exits_three` — `MOCK_FAIL_START=1` writes no ready line and exits 3.
    - **Create `crates/dosker-test-support/src/bin/mock-engine.rs`**
        - Reads `MOCK_READY_DELAY_MS` (default 0) and `MOCK_FAIL_START` (default unset).
        - On start: exit 3 when `MOCK_FAIL_START=1`; otherwise sleep the delay, write the ready line, and flush.
        - Reads JSON lines from stdin: `exec` with a `command` field, or `stop`; unknown ops reply `{"error":"unknown_op"}` and continue.
        - An `exec` request carries an `id` (default 0). The response is `{"id":<id>,"exit_code":<code>,"output":"<string>"}`. The output is JSON-escaped.
        - `ECHO <rest>` → output `<rest>\r\n`, exit 0; `EXIT <n>` → output empty, exit `n`; `SLEEP <ms>` → sleep then output empty, exit 0; any other command → output empty, exit 0.
        - A `stop` op writes `{"event":"stopped"}` and exits 0.
        - Stdin EOF blocks forever in a sleep loop; it never exits on EOF. This is what makes the orphan test meaningful.
        - Every output line is flushed and contains no literal newline inside the JSON.
    - **Create `crates/dosker-test-support/tests/mock_adapter.rs`**
        - `start_reports_pid_and_process_group` (`cfg(unix)`) — `pid` is `Some(child pid)`, `process_group` is `Some(child pid)`.
        - `ready_returns_after_ready_event`.
        - `ready_times_out_when_delay_exceeds_timeout` — `MOCK_READY_DELAY_MS=2000`, `ready(200 ms)` returns `ReadyTimeout`.
        - `exec_returns_output_and_exit_code`.
        - `exec_timeout_is_reported` — `SLEEP 5000` with `timeout_ms = 100` returns `ExecTimeout`.
        - `stop_returns_exit_code_zero`.
        - `kill_reaps_the_child` — `try_exit` is `Some(137)` on Unix after `kill`; the child is not running.
        - `try_exit_is_none_while_running`.
        - `capabilities_are_boot_serial_and_stop`.
        - `start_fails_when_the_binary_is_missing` — `NotFound`.
    - **Create `crates/dosker-test-support/src/mock_adapter.rs`**
        - `MockAdapter` — `new(engine_path: PathBuf, processes: Arc<dyn ProcessSupervisor>) -> Self`; `with_child_env(self, env: Vec<(String, String)>) -> Self`.
        - `impl EngineAdapter for MockAdapter` — name `mock`, version `0.1.0`, capabilities the set `{Boot, Serial, Stop}`, `probe` checks the binary is a file, `start` spawns the child with piped stdin and stdout, stderr appended to `log_dir/engine.log`, `prepare_engine` applied, and returns a `MockSession`.
        - `MockSession` — a reader thread parses stdout lines into a channel; `ready` expects the ready event within the timeout and returns `Exited` if the channel closes; `exec` writes `{"op":"exec","id":N,"command":...,"workdir":...,"timeout_ms":...}` and waits up to `timeout_ms` for the response with the same `id`, parsing `exit_code` and `output` into `ExecResult` with `timed_out = false` and `truncated = false`; `stop(grace)` writes `{"op":"stop"}` and polls `try_wait` every 10 ms until the deadline, then returns `StopTimeout`; `kill` terminates and reaps, returning `code().unwrap_or(137)` on Unix and `code().unwrap_or(-1)` on Windows; `try_exit` polls `try_wait`; `process_group` returns `Some(pid)` on Unix and `None` on Windows.
    - **Create `crates/dosker-test-support/src/fixtures.rs` and update `crates/dosker-test-support/src/lib.rs`**
        - `pub mod fixtures; pub mod mock_adapter;` in `lib.rs`.
        - `TempRoot` — `new() -> Self`, `path(&self) -> &Path`; wraps `tempfile::TempDir`.
        - `pub fn mock_engine_path() -> Result<PathBuf, String>` — `DOSKER_MOCK_ENGINE` when set and a file, otherwise `mock-engine` (`.exe` on Windows) beside `current_exe()`, otherwise one directory above `current_exe()`; the error names the build command `cargo build -p dosker-test-support --bin mock-engine`.
        - `pub fn wait_until(timeout: Duration, step: Duration, predicate: impl FnMut() -> bool) -> bool`.
        - `pub fn kill_process(pid: u32) -> bool` — `SIGKILL` on Unix; `OpenProcess(PROCESS_TERMINATE)` plus `TerminateProcess` on Windows; false when the process is already gone.
        - `crates/dosker-test-support/Cargo.toml` adds dosker-engine, serde_json, `libc` (Unix), and the `[[bin]]` entry `name = "mock-engine"`, `path = "src/bin/mock-engine.rs"`; `windows-sys` is already present from the named-pipe ACL test.
    - **Verify:** `cargo test -p dosker-test-support --test mock_engine --test mock_adapter`

- [x] **Task 11: Per-container supervisor and lifecycle**
    - **Implements:** FR-008 (headless container, per-container supervisor, no central daemon), FR-019 (engine death recorded), FR-022 (served over the transport trait), design §Process Model, §Supervisor, §Runtime.
    - **Create `crates/dosker-runtime/tests/supervisor.rs`**
        - `supervisor_answers_ping_status_exec_and_stop` — start in a thread against a temp root with a `created` record and a `MockAdapter`; `ping` returns `pong`; `status` reports `running`; `exec ECHO HELLO` returns `HELLO\r\n` and exit 0; `stop` reports `stopped`; the thread returns 0; the final record is `stopped` with `pid = None`.
        - `supervisor_records_running_state_before_serving` — after `ping`, the record is `running`, `pid` is the supervisor PID, `engine_pid` is set, and `engine_version` is `0.1.0`.
        - `supervisor_creates_engine_serial_and_container_logs` — the three files exist after start.
        - `supervisor_marks_failed_when_engine_never_ready` — `MOCK_READY_DELAY_MS=2000` with a 200 ms readiness timeout; `run` returns 1; the record is `failed` with `finished_at` set.
        - `supervisor_marks_exited_when_engine_process_dies` — kill the engine PID from the record with `kill_process`; within 2 s the record becomes `exited`, `dirty` true, `engine_pid` `None`, `finished_at` set.
        - `malformed_frame_closes_connection_without_state_change` — send `{`, then read EOF; the status is unchanged.
        - `unknown_message_returns_unknown_message_and_keeps_connection` — send `{"type":"attach"}`, then `ping` on the same connection; the first response has `code = unknown_message` and the second is `pong`.
        - `one_connection_serves_multiple_requests` — `ping`, `status`, `ping` on one connection return three responses in order.
        - `invalid_body_returns_invalid_message_and_closes` — send `{"type":"exec"}`; the response has `code = invalid_message`; the next read is EOF.
        - `exec_timeout_marks_container_exited_and_replies_timed_out` — `SLEEP 5000` with a 100 ms exec timeout; the reply has `timed_out: true` and exit 124; the record becomes `exited` with `dirty` true.
        - `exec_appends_a_line_to_container_log` — after a successful exec, `container.log` contains `exec exit=0 timed_out=false command=ECHO HELLO`.
    - **Create `crates/dosker-runtime/src/supervisor.rs`; add `pub mod supervisor;` to `src/lib.rs` and re-export `Supervisor`, `SupervisorConfig`, `SupervisorError`, `DEFAULT_READY_TIMEOUT`, `ACCEPT_POLL`, and `SERVER_READ_TIMEOUT`**
        - `pub const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(60);`, `pub const ACCEPT_POLL: Duration = Duration::from_millis(50);`, `pub const SERVER_READ_TIMEOUT: Duration = Duration::from_secs(5);`.
        - `SupervisorConfig` — `root: PathBuf`, `container_id: ContainerId`, `ready_timeout: Duration`.
        - `Supervisor` — `new(config: SupervisorConfig, adapter: Box<dyn EngineAdapter>) -> Self`, `run(self) -> Result<u8, SupervisorError>`.
        - `SupervisorError` — `Store(#[from] StoreError)`, `Engine(#[from] EngineError)`, `Platform(#[from] PlatformError)`, `Core(#[from] CoreError)`, `Protocol(#[from] io::Error)`.
        - Run order: load the record; set `pid` to the current process and transition to `starting` (accept `created`, `stopped`, or `exited` as the source status); set `engine_version` from `adapter.version()`; call `adapter.probe`; ensure the three logs; bind the control endpoint; build the `EngineSpec` (`workdir` `C:\`, empty `env`, no `cmd`, `expects_agent` false, `log_dir` the container directory); start the session; record `engine_pid` and `engine_pgid`; call `ready` with the configured timeout; transition to `running` and save; enter the loop. On a `probe`, `bind`, `start`, or `ready` error, call `session.kill()` when a session exists, transition to `failed`, clear `pid`, set `finished_at`, save, remove the endpoint, and return 1.
        - Loop: before each `accept`, call `session.try_exit`; a natural exit while idle writes `exited` with `dirty = true`, clears `pid` and `engine_pid`, sets `finished_at`, saves, and ends the loop. An accepted connection is served inline with one `BufReader` per connection reused for every frame (so read-ahead is not lost): read frames with the 5 s timeout, dispatch, write one response per request, and continue until EOF, a frame error, an `invalid_message` body, or a `stop`/exec-timeout shutdown.
        - `stop` handling: if the record is already `stopped`, `exited`, or `failed`, reply with its status and end the loop; otherwise transition to `stopping` and save; call `session.stop(grace)`; on success transition to `stopped` with `exit_code`, `dirty = false`; on `StopTimeout` call `session.kill()` and transition to `exited` with `dirty = true`; on any other engine error call `session.kill()` and transition to `exited` with `dirty = true`; clear `pid` and `engine_pid`, save, reply with the final status, and end the loop.
        - `exec` handling: reply `not_running` unless the status is `running`; call `session.exec`; on `ExecTimeout` call `session.kill()`, transition to `exited` with `dirty = true`, clear `pid` and `engine_pid`, reply with `exit_code = 124`, `output = ""`, `timed_out = true`, `truncated = false`, and end the loop; on any other engine error transition to `exited` with `dirty = true`, clear `pid` and `engine_pid`, reply `engine_error`, and end the loop; on success reply with the result and append one line to `container.log` in the form `exec exit=<code> timed_out=<bool> command=<command>`.
        - On every exit path remove the control endpoint; return 0 for `stopped` or `exited`, 1 for `failed`. The `failed` path also clears `pid`.
    - **Verify:** `cargo build -p dosker-test-support --bin mock-engine && cargo test -p dosker-runtime --test supervisor`

- [x] **Task 12: CLI shell, hidden supervise entry, ps, and status**
    - **Implements:** FR-008 (detached supervisor entry and state-only `ps`), FR-019 (`ps` and `status` recover before reporting), FR-022 (CLI client over the transport trait), design §CLI, §Process Model.
    - **Modify `crates/dosker-cli/Cargo.toml` and `src/main.rs`; create `src/cli.rs`, `src/context.rs`, `src/error.rs`, `src/resolve.rs`, `src/control_client.rs`, `src/adapter.rs`, and `src/commands/mod.rs`**
        - `Cargo.toml` — `[[bin]] name = "dosker"`, `path = "src/main.rs"`; `[features] mock-engine = ["dep:dosker-test-support"]`; `dosker-test-support` optional plus a dev-dependency on it; dev-dependency `tempfile`.
        - `main.rs` — `fn main() -> std::process::ExitCode` calling `cli::run(std::env::args_os())` and converting the `u8`.
        - `cli.rs` — clap derive `Cli { command: Command }`; `Command` variants `Run(RunArgs)`, `Ps(PsArgs)`, `Status(StatusArgs)`, `Exec(ExecArgs)`, `Stop(StopArgs)`, and `#[command(name = "__supervise", hide = true)] Supervise(SuperviseArgs)`; `pub fn run(args: impl IntoIterator<Item = OsString>) -> u8`; constants `EXIT_OK`, `EXIT_ERROR`, `EXIT_USAGE`, `EXIT_NOT_RUNNING`, `EXIT_TIMEOUT` with values 0, 1, 2, 125, 124.
        - `RunArgs` — `detach: bool` (`-d`/`--detach`), `name: Option<String>`, `engine: Option<String>`, `image: String`.
        - `PsArgs` — `all: bool` (`-a`/`--all`); `StatusArgs` — `container: String`; `SuperviseArgs` — `root: PathBuf`, `container: String`, `engine: String`.
        - `context.rs` — `Context { pub paths: RuntimePaths, pub store: StateStore, pub console: Console }`; `Context::load() -> Result<Self, CliError>` using `data_root`; `Context::with_console(root: PathBuf, console: Console) -> Self`.
        - `error.rs` — `CliError` — `Usage(String)`, `Message(String)`, `Platform(#[from] PlatformError)`, `Core(#[from] CoreError)`, `Store(#[from] StoreError)`, `Io(#[from] io::Error)`; `CliError::exit_code(&self) -> u8`: usage and invalid name/id are 2, everything else is 1.
        - `resolve.rs` — `Selector` — `Id(ContainerId)` or `Name(ContainerName)`; `parse_selector(value: &str) -> Result<Selector, CliError>`; `resolve(store: &StateStore, value: &str) -> Result<ContainerRecord, CliError>` (id first, then name; not found is `Message`/1).
        - `control_client.rs` — `pub fn request(endpoint: &EndpointRecord, request: &Request, timeout: Duration) -> Result<Response, CliError>`: connect, set read and write timeouts to the timeout plus 5 s, write one frame, read one frame, and deserialize a `Response`.
        - `adapter.rs` — `pub fn default_engine() -> Option<&'static str>` (Some `mock` with the feature, otherwise `None`), `pub fn is_known(engine: &str) -> bool`, and `pub fn build(engine: &str) -> Result<Box<dyn EngineAdapter>, CliError>` (with the feature, `mock` builds a `MockAdapter` from `mock_engine_path` and `SystemProcesses`; unknown is `Usage`).
        - `commands/mod.rs` — declare `ps`, `status`, and `supervise`; Tasks 13 and 14 add `exec`, `run`, and `stop`.
        - `commands/supervise.rs` — `pub fn execute(args: SuperviseArgs) -> Result<u8, CliError>`: parse `container`, build the adapter, and run `Supervisor::new(SupervisorConfig { root, container_id, ready_timeout: DEFAULT_READY_TIMEOUT }, adapter)`; map a `SupervisorError` to `CliError::Message`.
        - `cli.rs` — dispatch `Ps` and `Status` to their commands with `&SystemProbe::new()` and `Utc::now()`; dispatch `Supervise`; `Run`, `Exec`, and `Stop` return `Usage` until Tasks 13 and 14 replace their arms.
    - **Create `crates/dosker-cli/tests/cli_ps.rs`**
        - `ps_lists_only_active_by_default`.
        - `ps_all_includes_stopped_and_exited`.
        - `ps_reports_exited_after_dead_supervisor` — a `running` record with a dead PID and dead endpoint; the output row says `exited`.
        - `ps_hides_created_unless_all` — a `created` record is hidden by default.
        - `ps_prints_only_the_header_on_a_fresh_root`.
        - `ps_does_not_connect_to_live_containers` — a live record with a fake probe is listed as `running` and no control request is made.
    - **Create `crates/dosker-cli/src/commands/ps.rs`**
        - `execute(args: PsArgs, ctx: &mut Context, probe: &dyn HostProbe, now: DateTime<Utc>) -> Result<u8, CliError>`: `recover_stale`, `list`, filter active unless `-a`, print the header `ID NAME IMAGE STATUS CREATED` and one row per record in the store's order (`created_at` then id), created time as RFC 3339 seconds.
    - **Create `crates/dosker-cli/tests/cli_status.rs`**
        - `status_accepts_a_full_id`.
        - `status_accepts_a_name`.
        - `status_uses_the_supervisor_reply_when_reachable` — a stub listener returns a canned `status` reply with `running`; the CLI prints it instead of the record.
        - `status_falls_back_to_the_record_when_the_endpoint_is_dead` — prints `exited`.
        - `status_unknown_container_exits_one`.
        - `status_rejects_an_invalid_reference_with_exit_two` — `Bad Name` is neither a full id nor a valid name.
    - **Create `crates/dosker-cli/src/commands/status.rs`**
        - `execute(args: StatusArgs, ctx: &mut Context, probe: &dyn HostProbe, now: DateTime<Utc>) -> Result<u8, CliError>`: resolve, `recover_stale`, and if the record is active query `status` over the endpoint; on a transport failure fall back to the recovered record. Print `{id} {name} {status} pid={pid-or-dash} engine_pid={engine_pid-or-dash}`.
    - **Verify:** `cargo test -p dosker-cli --features mock-engine --test cli_ps --test cli_status`

- [x] **Task 13: Detached `run -d`**
    - **Implements:** FR-008 (detached per-container supervisor, no central daemon), FR-022 (spawned supervisor owns the control endpoint), design §Process Model, §CLI (“The CLI exits after it starts a detached container”), §Runtime.
    - **Create `crates/dosker-cli/tests/cli_run.rs`**
        - `run_requires_detach` — exit 2.
        - `run_rejects_an_unknown_engine` — exit 2.
        - `run_rejects_a_duplicate_name` — exit 1.
        - `run_prints_a_twelve_hex_id` — stdout is a single 12-hex line.
        - `run_generates_a_name_when_omitted` — the record name is `dosker-` plus the first 8 id characters.
        - `run_records_the_control_endpoint_under_the_root` — the state record’s `sockets["control"]` points into the temp root.
        - `run_returns_after_the_container_is_running` — immediately after `run`, the state is `running` and the supervisor PID is alive.
        - `run_reports_a_failed_start` — the `run` child is launched with `MOCK_FAIL_START=1`; the supervisor and the mock engine inherit it; `run` exits 1 and the record is `failed`.
        - The test helper builds `Command::new(env!("CARGO_BIN_EXE_dosker"))`, sets `DOSKER_ROOT`, sets `DOSKER_MOCK_ENGINE` from `mock_engine_path()`, and never mutates the test process environment.
    - **Create `crates/dosker-cli/src/commands/run.rs`**
        - `execute(args: RunArgs, ctx: &mut Context, now: DateTime<Utc>) -> Result<u8, CliError>`: refuse a missing `-d` with `Usage`; choose the engine (`--engine`, else `default_engine`, else `Usage`); reject an unknown engine with `Usage`; generate the id first; take `--name` or build `dosker-<first 8 hex>`; build the `EndpointRecord`; create a `created` record; spawn `current_exe __supervise --root <absolute root> --container <id> --engine <engine>` with stdin null, stdout and stderr appended to `container.log`, and the `ProcessSupervisor::prepare_detached` method; drop the child; poll up to 65 s (100 ms steps) by reloading the record (a `failed` or `exited` record returns 1 after printing its status) and sending `ping` with a 200 ms budget; on `pong` print the id and return 0; on deadline print the record status and return 1.
    - **Modify `crates/dosker-cli/src/cli.rs`** — dispatch `Run` to `commands::run::execute(args, &mut ctx, Utc::now())`.
    - **Verify:** `cargo build -p dosker-test-support --bin mock-engine && cargo test -p dosker-cli --features mock-engine --test cli_run`

- [x] **Task 14: `exec` and `stop` commands**
    - **Implements:** FR-008 (`exec` and `stop` reached over the control endpoint), FR-019 (a dead supervisor recovers before the command reports), FR-022 (client transport), design §CLI, §Supervisor.
    - **Create `crates/dosker-cli/tests/cli_exec.rs`**
        - `exec_echo_returns_output_and_zero`.
        - `exec_passes_the_guest_exit_code_through` — `EXIT 7` exits 7.
        - `exec_timeout_exits_124` — `SLEEP 5000` with `--timeout 1`.
        - `exec_on_a_stopped_container_exits_125`.
        - `exec_joins_multiple_words_with_spaces` — `ECHO HELLO WORLD` prints `HELLO WORLD`.
        - `exec_invalid_env_exits_two`.
        - `exec_unknown_container_exits_one`.
        - `exec_writes_guest_output_verbatim`.
    - **Create `crates/dosker-cli/src/commands/exec.rs`**
        - `execute(args: ExecArgs, ctx: &mut Context, probe: &dyn HostProbe, now: DateTime<Utc>) -> Result<u8, CliError>`: resolve, `recover_stale`, require `running` (otherwise `EXIT_NOT_RUNNING`); parse `-e K=V` (split at the first `=`, invalid is `Usage`); join the trailing command words with single spaces; send `exec` with the workdir (default `C:\`) and the 30 s default or `--timeout` seconds; write `output` verbatim, appending one `\n` only when the output is non-empty and does not already end with `\n`; `timed_out` returns 124; otherwise return the guest code clamped to 0..=255; a `not_running` or `engine_error` reply returns 125.
        - `ExecArgs { env: Vec<String>, workdir: String, timeout: u64, container: String, command: Vec<String> }` with `-e/--env`, `-w/--workdir`, `--timeout`, and a `trailing_var_arg` required command.
    - **Create `crates/dosker-cli/tests/cli_stop.rs`**
        - `stop_running_container_reports_stopped`.
        - `stop_is_idempotent` — a second stop is 0 and prints `stopped`.
        - `stop_dead_supervisor_recovers_to_exited`.
        - `stop_unknown_container_exits_one`.
    - **Create `crates/dosker-cli/src/commands/stop.rs`**
        - `execute(args: StopArgs, ctx: &mut Context, probe: &dyn HostProbe, now: DateTime<Utc>) -> Result<u8, CliError>`: resolve; `recover_stale`; if the status is already `stopped`, `exited`, or `failed`, print `{name} {status}` and return 0; otherwise send `stop` with `--time` seconds and print `{name} {status}` from the reply; a connect failure re-runs stale recovery and returns 0 when the recovered status is `exited`, otherwise 1.
        - `StopArgs { time: u64, container: String }` with `--time` default 10.
    - **Modify `crates/dosker-cli/src/cli.rs`** — dispatch `Exec` and `Stop`.
    - **Verify:** `cargo build -p dosker-test-support --bin mock-engine && cargo test -p dosker-cli --features mock-engine --test cli_exec --test cli_stop`

- [x] **Task 15: End-to-end detached lifecycle, orphan cleanup, and `DOSKER_ROOT`**
    - **Implements:** FR-008 (detached container answers over the socket), FR-019 (the next `ps` marks a killed supervisor’s container `exited`), FR-022 (end-to-end over the native endpoint), FR-023 (no orphan process after a supervisor kill on Unix), constraint “`DOSKER_ROOT` overrides the data root”.
    - **Create `crates/dosker-cli/tests/detached_lifecycle.rs`**
        - `detached_container_answers_exec_status_and_stop` — `run -d`, assert the state is `running`, `exec ECHO HELLO` prints `HELLO` and exits 0, `status` reports `running`, `stop` reports `stopped`, `ps -a` lists the record as `stopped`.
        - `killing_the_supervisor_leaves_no_engine_and_ps_reports_exited` (`cfg(unix)`) — `run -d`; read `state.json` for the supervisor and engine PIDs; `kill_process(supervisor_pid)`; run `ps`, which performs recovery and kills the recorded engine group; wait up to 5 s for the engine PID to disappear; assert the row says `exited`, the state says `exited` with `pid`, `engine_pid`, and `engine_pgid` all `None`, and `dirty` true.
        - `dosker_root_redirects_all_state` — run with `DOSKER_ROOT` set to a temp dir and the host data-root variables (`HOME` and `XDG_DATA_HOME` on Unix, `LOCALAPPDATA` on Windows) set to another empty temp dir; assert `containers/<id>/state.json`, `engine.log`, `serial.log`, `container.log`, and (on Unix) `control.sock` exist under the first root, and the second root stays empty.
        - `long_dosker_root_starts_and_stops_through_the_fallback` (`cfg(unix)`) — set `DOSKER_ROOT` to a temp dir with a 200-byte component; `run -d`, `exec ECHO HELLO`, and `stop` succeed; the state's `sockets["control"]` is the fallback path under `/tmp/dosker-<euid>/`, the path is at most 100 bytes, and the socket file is gone after `stop`.
        - `status_reads_state_from_the_dosker_root` — a container created under one root is not found when `DOSKER_ROOT` names another root.
        - `exec_and_stop_on_an_absent_container_exit_one` — both commands report the missing container with exit code 1.
        - The helper spawns the real `dosker` binary, sets all environment variables per child, parses the container id from `run` output, and cleans up with `stop`; if `stop` fails it kills the supervisor and the engine PID from the state.
    - **Verify:** `cargo build -p dosker-test-support --bin mock-engine && cargo test -p dosker-cli --features mock-engine --test detached_lifecycle`

- [x] **Task 16: Continuous integration for Linux, macOS, and Windows**
    - **Implements:** design §Testing Strategy (“CI runs formatting, clippy, and dependency checks on every commit. Unit and mock tests run on Linux, macOS, and Windows.”), §Success Criteria portability.
    - **Create `.github/workflows/ci.yml`**
        - Job `fmt` on `ubuntu-latest`: checkout, install toolchain 1.98.1 with `rustfmt` and `clippy`, `cargo fmt --all -- --check`.
        - Job `test` with `strategy.matrix.os = [ubuntu-latest, macos-14, windows-latest]` and `fail-fast: false`.
        - Steps on every matrix entry: checkout; install toolchain 1.98.1 with `rustfmt` and `clippy`; cache `~/.cargo` and `target` with `Swatinem/rust-cache@v2`; `cargo build --workspace --all-targets --features dosker-cli/mock-engine`; `cargo clippy --workspace --all-targets --features dosker-cli/mock-engine -- -D warnings`.
        - Step `unit and mock suites`: `cargo test --workspace --features dosker-cli/mock-engine`.
        - Step `transport contract suite`: `cargo test -p dosker-test-support --test transport_contract`.
        - Step `detached lifecycle suite`: `cargo test -p dosker-cli --features mock-engine --test detached_lifecycle`.
        - Pin `macos-14` for the arm64 host and `windows-latest` for the x86_64 host.
    - **Verify:** run the same commands locally in this order: `cargo fmt --all -- --check`; `cargo build --workspace --all-targets --features dosker-cli/mock-engine`; `cargo clippy --workspace --all-targets --features dosker-cli/mock-engine -- -D warnings`; `cargo test --workspace --features dosker-cli/mock-engine`. The Windows and macOS legs run in the matrix once the workflow is pushed.

## Requirements Coverage

| Requirement | Tasks |
| --- | --- |
| FR-008 | 6, 11, 12, 13, 14, 15 |
| FR-019 | 1, 2, 7, 8, 9, 11, 12, 14, 15 |
| FR-022 | 2, 3, 3A, 4, 11, 12, 13, 14, 15 |
| FR-023 (Slice 1 part: seam and Unix) | 5, 9, 15 |

FR-023’s Job Object half is not planned here; it belongs to the Windows
runtime slice, which also adds the Windows engine endpoints and the
Windows smoke tests.

## Done When Map

| Roadmap acceptance clause | Proved by |
| --- | --- |
| A mock container starts detached on Linux and macOS | Task 13 `run_returns_after_the_container_is_running`; Task 15 `detached_container_answers_exec_status_and_stop` |
| It answers `exec`, `status`, and `stop` over the Unix socket | Task 11 supervisor tests; Task 15 end-to-end test |
| It writes valid state | Task 7 store tests; Task 15 state assertions |
| Killing the supervisor makes the next `ps` report `exited` | Task 9 stale tests; Task 15 orphan test |
| It leaves no mock process | Task 5 group kill; Task 9 `stale_engine_group_is_killed`; Task 15 orphan test |
| The named-pipe backend passes the same contract suite on the Windows runner | Task 4 contract suite in the Task 16 Windows matrix leg |
| `DOSKER_ROOT` redirects all state | Task 2 data-root tests; Task 15 `dosker_root_redirects_all_state` |
| A long `DOSKER_ROOT` still starts and stops through the fallback | Task 3A short-path tests; Task 15 `long_dosker_root_starts_and_stops_through_the_fallback` |

## Out Of Scope

- Any real engine (QEMU, DOSBox-X, Bochs), the DOS agent, image storage, the build path, volumes, NAT, development mode, `attach`, `cp`, `debug`, and `screenshot`.
- `run` without `-d` (foreground attach); the command requires `-d` and there is no attach shell yet.
- The Windows Job Object with kill-on-close; this slice only reaches it through the `ProcessSupervisor` seam and the Windows creation flags.
- Windows engine endpoints on localhost TCP; there is no engine yet.
- `start`, `restart`, `rm`, `inspect`, `logs`, and `port`.
- `MachineProfile`, boot metadata, container disks, ports, and volumes in the state record.

## Risks

- **Single connection at a time.** A slow `exec` delays a concurrent `stop` or `status` until the connection loop is free. A single DOS machine serialises guest commands anyway, and the CLI uses one request per connection; the risk is a hung client holding the loop for the 5 s read timeout. The mitigation is the server read timeout and the `exec` timeout.
- **macOS orphan window.** Without a parent-death signal, the engine survives a `SIGKILL` of the supervisor until the next CLI call, which kills the recorded process group. The done-when is measured after that call. Pid reuse is the residual risk; the record clears the PID and group after recovery, and the window is bounded by the next CLI invocation.
- **Windows DACL verification.** The named-pipe security test runs only on the Windows runner and is the only proof of the current-user-only DACL; no cross-user attempt is automated.
- **Mock binary availability.** Cross-package tests resolve `mock-engine` beside the test binary. Every Verify command that needs it builds it first, and CI builds all targets before testing.
- **Atomic write portability on Windows.** `ReplaceFileW` has no target-created path; the fallback to `MoveFileExW` covers the first write. The contract is exercised by the Windows matrix leg.
- **State schema drift.** `ContainerRecord` is versioned and ignores unknown fields, so later slices add fields without breaking reads; a version change is still a breaking change and is rejected on load.
- **No `rm` in this slice.** A stopped, failed, or created container keeps its name and directory. A retry uses a new name; `rm` arrives with the image and lifecycle work.
- **Unix socket path length.** A Unix socket path is limited to roughly 104 bytes on macOS. Task 3A falls back to `/tmp/dosker-<euid>/<id>.sock` when the default path exceeds 100 bytes, records that path in `sockets["control"]`, and removes it on exit. The residual risk is a host without a usable short temp directory; `bind` returns the OS error and the supervisor records a failed start.

## Deviations

- `GdbStub` is named `gdbstub` in `Capability::name` and serde, matching the design and Doskerfile contract. The generic snake-case derive would have produced `gdb_stub`.
- `Starting -> Exited` is an allowed transition; stale recovery needs it for a supervisor that dies during startup.
- Task 4 verifies the workspace `windows-sys` features instead of extending them; the workspace table carries the transport and ACL features, including `Win32_System_SystemServices` for `ACCESS_ALLOWED_ACE_TYPE`.
- Task 12 declares only `ps`, `status`, and `supervise`; Tasks 13 and 14 add `exec`, `run`, and `stop`.
- The CLI has a `src/lib.rs` library target so integration tests can call `Context::with_console`; `main.rs` calls `dosker_cli::cli::run`.
- `ensure_dir_mode` rejects a symlink directory after `create_dir_all`.
- The supervisor removes its control endpoint only after a successful bind; a failed bind leaves another supervisor's socket alone.
- On macOS, `serve` treats a `set_read_timeout` `InvalidInput` as a closed peer and continues; other errors propagate.
- The `run` liveness probe uses a hard 200 ms connect and read budget.
- `exec` maps only a connect failure (`CliError::Platform`) to 125; other client errors exit 1.
- The design CLI table includes `dosker status`; the process-model stale wording is `exited`.

