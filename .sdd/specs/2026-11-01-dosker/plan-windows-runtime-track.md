# Windows Runtime Track Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox. Mark it done when its changes are implemented and its
> `Verify` command passes.
>
> Write the listed tests first. Run the task `Verify` command and confirm the
> expected failure before you implement the task.

- **Date**: 2026-11-04
- **Status**: In Progress
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Roadmap**: `.sdd/specs/2026-11-01-dosker/roadmap.md` — slice: `windows-runtime-track`
- **Depends on**: Slices 1 and 3.
- **Manual prerequisites**: a Windows CI runner with QEMU. A Windows QEMU build with a pinned version and checksum.
  The human supplies `DOSKER_WINDOWS_QEMU_URL` as a ZIP archive,
  `DOSKER_WINDOWS_QEMU_SHA256` for that archive, and
  `DOSKER_WINDOWS_QEMU_VERSION` as GitHub Actions repository variables. The human
  supplies a prepared FreeDOS image through `DOSKER_QEMU_IMAGE_URL` or a local
  `DOSKER_QEMU_IMAGE`. The design does not name the Windows QEMU source, version,
  or checksum. This plan does not invent them.
- **Goal**: complete the Windows x86_64 runtime path for the QEMU engine, with a named-pipe control channel, Job Object cleanup, loopback TCP engine endpoints, discovery checks, smoke tests, and documentation.

## Constraints

- The control endpoint on Windows is the named pipe `\\.\pipe\dosker-<id>-control`.
  Its DACL has one access entry for the current user SID with the form
  `D:P(A;;GA;;;<current-user-sid>)`.
- The data root is `%LOCALAPPDATA%\dosker`. `DOSKER_ROOT` overrides it.
- State writes use `ReplaceFileW`. A missing target falls back to
  `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`.
- Engine endpoints on Windows bind to `127.0.0.1` only. The serial agent, QMP,
  monitor, and gdbstub use TCP. VNC uses `127.0.0.1:<port>` with a port at or
  above `5900`.
- The QEMU process runs in a Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
- The QEMU program name on Windows is `qemu-system-i386.exe`. Discovery checks
  `DOSKER_QEMU` first, then `PATH`, then `%ProgramFiles%\qemu`,
  `%ProgramFiles(x86)%\qemu`, and `%LOCALAPPDATA%\Programs\qemu`.
- `DOSKER_QEMU_SHA256` pins the executable SHA-256 when it is set.
  `DOSKER_QEMU_VERSION` pins the exact version when it is set. Its format is
  `X.Y.Z` with decimal components and no prefix or suffix.
- The Windows QEMU minimum version is `7.1.0`.
- Real engine tests run when `DOSKER_INTEGRATION=1`. The prepared image comes
  from `DOSKER_QEMU_IMAGE`. The binary comes from `DOSKER_QEMU`.
- The QEMU adapter keeps its current capability set. `Capability::Nat` stays
  absent. QEMU NAT is not part of this slice. Slice 9 owns it.
- `--display window` returns `EngineError::Unsupported("window display on Windows")`.
  `--display none` and `--display vnc` remain supported.
- The Windows job object and the Windows smoke tests run on a Windows x86_64
  runner only. Windows arm64 is out of scope.
- Do not change the build path, `dosbox-x`, `bochs`, or COM1 pass-through.

## Deferred Rulings

- **NAT**: the roadmap slice now records that QEMU NAT remains in Slice 9.
  Design M1-W and FR-021, FR-022, and FR-023 do not require NAT. No task in
  this plan enables `-nic user`, adds `Capability::Nat`, or publishes a guest
  port. This plan records the deferral and does not implement NAT.
- **Windows arm64**: out of scope per the design.
- **Build path, DOSBox-X, Bochs, COM1 pass-through**: out of scope.

## Shared Interfaces

These types and signatures are the contract between tasks. Later tasks use
exactly these names.

- `dosker_engine::EngineEndpointKind`
  - `UnixSocket(PathBuf)`
  - `Tcp { host: std::net::Ipv4Addr, port: u16 }`
- `dosker_engine::EngineEndpoint { name: String, kind: EngineEndpointKind }`
- `dosker_engine::EngineCommand` gains `endpoints: Vec<EngineEndpoint>`.
- `dosker_platform::EndpointRecord` gains `Tcp { host: Ipv4Addr, port: u16 }`.
- `dosker_engine_qemu::QemuEndpoints` holds the optional serial agent endpoint
  and the QMP, monitor, and gdb endpoints.
- `dosker_engine_qemu::EngineStream` carries a Unix stream on Unix and a
  `TcpStream` on Windows.
- `dosker_platform::process::JobObject` owns the Windows job handle.
- `dosker_engine_qemu::MIN_QEMU_VERSION` is
  `QemuVersion { major: 7, minor: 1, patch: 0 }`.

## Requirement Coverage

| Requirement | Tasks | Evidence |
| --- | --- | --- |
| FR-021 Windows x86_64 runtime | 1, 2, 3, 4, 6, 7, 8, 9 | Windows QEMU smoke tests, discovery tests, CI job |
| FR-022 control and engine transport abstraction | 1, 2, 3, 4, 7 | Named-pipe control test, loopback TCP endpoint records, smoke tests |
| FR-023 Job Object and orphan cleanup | 5, 7 | Job Object unit tests, supervisor crash smoke test |

## Done When Map

| Roadmap acceptance clause | Tasks | Proof |
| --- | --- | --- |
| The design's M1-W definition of done passes on Windows x86_64 | 7, 8, 9 | `windows_qemu_runtime` suite in the Windows CI job |
| The CLI reaches the supervisor through a named pipe | 7 | `control_transport_is_a_named_pipe` and existing transport contract |
| The engine uses localhost TCP | 2, 3, 4, 7 | `engine_endpoints_are_loopback_tcp`, command and stream tests |
| A supervisor crash leaves no QEMU process | 5, 7 | `supervisor_crash_leaves_no_engine` |
| `engines` finds the Windows QEMU build and reports its capabilities | 6, 7 | `engines_and_doctor_report_qemu` |

## Risks

- The Windows runner can place the supervisor in a parent Job Object. Nested
  jobs work on Windows 8 and later. If `AssignProcessToJobObject` fails, fix
  the runner environment. Do not drop the job cleanup.
- The loopback ports are free when allocated and could be taken before QEMU
  binds. The engine allocation holds all listeners until all four ports are
  selected. VNC allocation excludes the four engine ports. Another container
  can still race for the same port. A bind failure fails the start with a
  clear engine error.
- The exact Windows QEMU artifact is not in the design. The human supplies the
  artifact and its checksum.

## Tasks

- [x] **Task 1: Engine endpoint values and TCP state records**

    - **Implements**: FR-021, FR-022, design Process Model, Host Platform
      Abstraction, and Data Model `Container.sockets`.
    - **Create tests in `crates/dosker-engine/tests/adapter.rs`**
        - `engine_endpoint_tcp_displays_loopback_address` — a TCP endpoint
          renders `127.0.0.1:<port>`.
        - `engine_endpoint_unix_displays_the_path` — a Unix endpoint renders
          its path.
    - **Create tests in `crates/dosker-platform/tests/endpoint.rs`**
        - `tcp_endpoint_record_round_trips` — the record serializes and
          deserializes as `{"kind":"tcp","host":"127.0.0.1","port":<port>}`.
        - `tcp_endpoint_record_has_no_local_artifact` — `local_endpoint`
          returns `None` for a TCP record.
    - **Create tests in `crates/dosker-runtime/tests/store.rs`**
        - `remove_ignores_tcp_engine_endpoints` — a stored TCP endpoint does
          not stop `StateStore::remove`.
    - **Create tests in `crates/dosker-runtime/tests/supervisor.rs`**
        - `supervisor_records_tcp_engine_endpoints` — the supervisor writes
          `EndpointRecord::Tcp` for a mock TCP endpoint.
    - **Modify `crates/dosker-engine/src/spec.rs`**
        - Add `pub enum EngineEndpointKind { UnixSocket(PathBuf), Tcp { host: Ipv4Addr, port: u16 } }`.
        - Add methods `unix_path(&self) -> Option<&Path>`,
          `tcp_addr(&self) -> Option<SocketAddr>`, and `display(&self) -> String`.
        - Change `EngineEndpoint` to
          `pub struct EngineEndpoint { pub name: String, pub kind: EngineEndpointKind }`.
    - **Modify `crates/dosker-engine/src/lib.rs`**
        - Re-export `EngineEndpointKind`.
    - **Modify `crates/dosker-platform/src/endpoint.rs`**
        - Add `EndpointRecord::Tcp { host: Ipv4Addr, port: u16 }`.
        - Add `EndpointRecord::tcp(port: u16) -> Self` with host
          `Ipv4Addr::LOCALHOST`.
        - Add `EndpointRecord::local_endpoint(&self) -> Option<Endpoint>`.
          Return `None` for `Tcp`.
        - Replace `impl From<EndpointRecord> for Endpoint` with
          `impl TryFrom<EndpointRecord> for Endpoint`. A TCP record returns
          `PlatformError::InvalidEndpoint`.
        - Convert every `Endpoint::from` call site:
          - `crates/dosker-runtime/src/stale.rs:50` — use a `match`. A TCP
            record returns `false` from `endpoint_alive`.
          - `crates/dosker-runtime/src/supervisor.rs:89` — use
            `Endpoint::try_from(endpoint.clone())?`. `SupervisorError` already
            converts `PlatformError`.
          - `crates/dosker-runtime/src/store.rs:206` — use
            `local_endpoint()` and remove `Endpoint` from the `dosker_platform`
            import.
          - `crates/dosker-cli/src/control_client.rs:22` and `:46` — use
            `Endpoint::try_from(endpoint.clone())?`.
          - `crates/dosker-cli/src/commands/attach.rs:121` — use
            `Endpoint::try_from(endpoint.clone())?`.
          - `crates/dosker-platform/tests/endpoint.rs:47` — use
            `Endpoint::try_from(record).expect("control endpoint")`.
          - `crates/dosker-platform/tests/short_socket.rs:171` — use
            `Endpoint::try_from(record).expect("control endpoint")`.
          - `crates/dosker-cli/tests/cli_status.rs:209` — use
            `Endpoint::try_from(endpoint).expect("control endpoint")`.
          - `crates/dosker-cli/tests/cli_ps.rs:321` — use
            `Endpoint::try_from(endpoint).expect("control endpoint")`.
    - **Modify `crates/dosker-runtime/src/supervisor.rs`**
        - Add `fn endpoint_record(endpoint: &EngineEndpoint) -> EndpointRecord`.
          Map `UnixSocket` to `EndpointRecord::UnixSocket` and `Tcp` to
          `EndpointRecord::Tcp`.
        - Use it in the `session.endpoints()` loop.
    - **Modify `crates/dosker-runtime/src/store.rs`**
        - Call `recorded.local_endpoint()` in `StateStore::remove`. Call
          `remove_endpoint` only when it returns `Some`.
    - **Modify `crates/dosker-cli/src/control_client.rs` and
      `crates/dosker-cli/src/commands/attach.rs`**
        - Use `Endpoint::try_from` for the control record.
    - **Update existing constructors and matches**
        - Change every `EngineEndpoint { name, path }` literal to
          `EngineEndpoint { name, kind: EngineEndpointKind::UnixSocket(path) }`.
          The affected files are `crates/dosker-runtime/tests/supervisor.rs`,
          `crates/dosker-test-support/src/mock_adapter.rs`, and
          `crates/dosker-engine-qemu/src/session.rs`.
        - Add a `Tcp` arm to the control endpoint match in
          `crates/dosker-cli/tests/cli_run.rs`. The arm must panic because a
          control endpoint is never TCP.
    - **Verify**: `cargo build -p dosker-test-support --bin mock-engine`
    - **Verify**: `cargo test -p dosker-engine -p dosker-platform -p dosker-runtime -p dosker-cli --features dosker-cli/mock-engine`

- [x] **Task 2: QEMU command endpoints and command payload**

    - **Implements**: FR-021, FR-022, design Process Model, Host Platform
      Abstraction, and Engine Adapter `build_command`.
    - **Replace `crates/dosker-engine-qemu/tests/paths.rs` with
      `crates/dosker-engine-qemu/tests/endpoints.rs`**
        - Remove the file-level `#![cfg(unix)]`. Gate the Unix assertions with
          `#[cfg(unix)]` and the Windows assertions with `#[cfg(windows)]`.
        - `unix_endpoints_use_the_container_socket_directory` — the four
          endpoints are `agent.sock`, `qmp.sock`, `monitor.sock`, and
          `gdb.sock`.
        - `unix_endpoints_fall_back_for_a_long_root` — long roots use the short
          socket directory.
        - `windows_endpoints_are_distinct_loopback_tcp_ports` — four distinct
          ports, all with host `127.0.0.1`.
        - `unix_endpoints_are_unique_per_container` — two containers never
          share a Unix path. Gate this test with `#[cfg(unix)]`.
        - `remove_stale_removes_existing_unix_sockets` — a stale socket file is
          gone and a missing file is ignored.
        - `endpoints_omit_the_agent_for_a_booter` — the agent endpoint is
          absent when `EngineSpec.booter` is true.
    - **Create tests in `crates/dosker-engine/tests/adapter.rs`**
        - `engine_command_carries_its_endpoints` — the command exposes the
          endpoints it rendered.
    - **Update `crates/dosker-engine-qemu/tests/command.rs`**
        - Remove the file-level `#![cfg(unix)]`. Gate the Unix assertions with
          `#[cfg(unix)]` and the Windows assertions with `#[cfg(windows)]`.
        - Replace `QemuPaths` with `QemuEndpoints` and use
          `unix_path().expect(...)` where the test needs a path.
        - `command_carries_one_endpoint_for_each_engine_channel` — the command
          carries `serial_agent`, `qmp`, `monitor`, and `gdb`.
        - `command_uses_unix_sockets_on_unix` — `-serial`, `-qmp`,
          `-monitor`, and `-chardev` use `unix:<path>` values.
        - `command_uses_loopback_tcp_on_windows` — the same options use
          `tcp:127.0.0.1:<port>` values.
        - `command_uses_distinct_loopback_ports_on_windows` — the four options
          use four different ports.
        - `vnc_port_does_not_reuse_an_engine_port` — with VNC and no explicit
          port, the selected VNC port is not one of the four engine TCP ports.
          Gate this test with `#[cfg(windows)]`.
        - `windows_rejects_the_window_display` — `DisplayMode::Window` returns
          `EngineError::Unsupported("window display on Windows")`.
        - `capabilities_never_include_nat` — `Capability::Nat` is absent.
    - **Modify `crates/dosker-engine/src/spec.rs`**
        - Add `pub endpoints: Vec<EngineEndpoint>` to `EngineCommand`.
    - **Create `crates/dosker-engine-qemu/src/endpoints.rs`**
        - `pub struct QemuEndpoints { pub agent_serial: Option<EngineEndpointKind>, pub qmp: EngineEndpointKind, pub monitor: EngineEndpointKind, pub gdb: EngineEndpointKind }`.
        - `pub fn for_spec(spec: &EngineSpec) -> Result<Self, EngineError>`.
          On Unix call `dosker_platform::endpoint::engine_socket_path`. On
          Windows call `reserve_tcp_ports(4)` for a normal image and
          `reserve_tcp_ports(3)` for a booter. Use host
          `Ipv4Addr::LOCALHOST`. Set `agent_serial` to `None` when
          `spec.booter` is true.
        - `#[cfg(windows)] fn reserve_tcp_ports(count: usize) -> Result<Vec<u16>, EngineError>`.
          Bind all `count` listeners on `127.0.0.1:0` and keep them alive
          until every address is collected. Sort the ports, drop the listeners,
          and return the ports. All selected listeners exist at the same time,
          so the ports cannot repeat.
        - `pub fn from_command(command: &EngineCommand) -> Result<Self, EngineError>`.
          Select `agent_serial` when a `serial_agent` endpoint exists. Always
          select `qmp`, `monitor`, and `gdb`.
        - `pub fn engine_endpoints(&self) -> Vec<EngineEndpoint>`. Include the
          agent endpoint only when `agent_serial` is `Some`.
        - `pub fn remove_stale(&self) -> Result<(), EngineError>`. On Unix
          remove every socket path. On Windows do nothing.
    - **Modify `crates/dosker-engine-qemu/src/command.rs`**
        - Use `QemuEndpoints::for_spec`.
        - Format `-serial`, `-qmp`, and `-monitor` as
          `unix:<path>,server=on,wait=off` on Unix and
          `tcp:127.0.0.1:<port>,server=on,wait=off` on Windows.
        - Format the gdb chardev as
          `socket,id=gdb0,path=<path>,server=on,wait=off` on Unix and
          `socket,id=gdb0,host=127.0.0.1,port=<port>,server=on,wait=off` on
          Windows.
        - Put `endpoints.engine_endpoints()` into `EngineCommand.endpoints`.
        - Collect the TCP ports from `endpoints.engine_endpoints()` and pass
          them to `display_arguments(display, vnc_port, &engine_ports)`.
        - Keep the VNC argument `127.0.0.1:<port - 5900>`.
        - Add `pub fn free_vnc_port_excluding(excluded: &[u16]) -> Result<u16, EngineError>`.
          Try `free_vnc_port`. If the result is in `excluded`, try successive
          loopback ports until one binds outside `excluded`. Return
          `EngineError::Setup` when no port is free. Drop the successful
          listener before returning. Use it when `DisplayMode::Vnc` has no
          explicit port.
        - Return `EngineError::Unsupported("window display on Windows")` for
          `DisplayMode::Window` on Windows. `none` and `vnc` remain supported.
        - Gate `window_backend` with `#[cfg(not(windows))]` so the Windows build
          has no dead function.
    - **Modify `crates/dosker-engine-qemu/src/lib.rs`**
        - Remove `#[cfg(unix)]` from `pub mod command;`. The command module is
          used on every host. The four engine endpoints do not use
          `free_tcp_port`. They use `reserve_tcp_ports`.
        - Add `pub mod endpoints;` and re-export `QemuEndpoints`. Keep the
          Unix-only `paths` module until the session task removes its last
          user.
    - **Modify `crates/dosker-engine-qemu/src/adapter.rs`**
        - Remove the `cfg(windows)` `build_command` stub. Use
          `command::build` on every host.
    - **Update existing `EngineCommand` literals**
        - Add `endpoints: Vec::new()` in
          `crates/dosker-test-support/src/mock_adapter.rs`,
          `crates/dosker-engine/tests/adapter.rs`,
          `crates/dosker-engine-qemu/tests/qemu_session.rs`,
          `crates/dosker-engine-qemu/src/session.rs` (the `session_with` test
          helper), and `crates/dosker-runtime/tests/supervisor.rs`.
    - **Verify**: `cargo build -p dosker-test-support --bin mock-engine`
    - **Verify**: `cargo test -p dosker-engine -p dosker-engine-qemu -p dosker-test-support -p dosker-runtime`

- [x] **Task 3: Loopback TCP transport and QMP client**

    - **Implements**: FR-021, FR-022, design Host Platform Abstraction and
      Control Endpoint Encoding.
    - **Create `crates/dosker-engine-qemu/tests/stream.rs`**
        - `stream_connects_to_a_loopback_listener` — a TCP endpoint connects to
          a bound `TcpListener`.
        - `stream_connect_stops_at_the_deadline` — a missing TCP server returns
          an I/O timeout before the deadline plus one second.
        - `stream_sets_read_and_write_timeouts` — the transport applies both
          timeouts.
        - `probe_reports_a_live_tcp_listener` — a bound `TcpListener` probes
          true.
        - `probe_reports_a_closed_tcp_port` — a closed loopback port probes
          false.
        - `probe_reports_a_missing_unix_path` — gate this test with
          `#[cfg(unix)]`. A missing socket path probes false.
    - **Update `crates/dosker-engine-qemu/tests/qmp.rs`**
        - Remove the file-level `#![cfg(unix)]`. Gate the Unix socket server
          with `#[cfg(unix)]`.
        - Change the existing Unix tests to pass
          `EngineEndpointKind::UnixSocket(path)`.
        - Add `qmp_connects_over_loopback_tcp` — the client reads the greeting,
          sends `qmp_capabilities`, and returns a command result over TCP.
    - **Create `crates/dosker-engine-qemu/src/stream.rs`**
        - `pub enum EngineStream { #[cfg(unix)] Unix(UnixStream), Tcp(TcpStream) }`.
        - Implement `Read`, `Write`, and
          `dosker_agent_proto::Transport` for `EngineStream`.
        - Implement `pub fn connect(kind: &EngineEndpointKind, deadline: Instant) -> Result<EngineStream, EngineError>`.
          On Unix keep the non-blocking Unix connect and deadline logic from
          `qmp.rs`. For a TCP endpoint, loop `TcpStream::connect_timeout` with
          a 20 millisecond retry until the deadline. Set `TCP_NODELAY`. On
          Windows, a Unix endpoint returns `EngineError::Unsupported`.
        - Implement `pub fn probe(kind: &EngineEndpointKind, timeout: Duration) -> bool`.
          For `UnixSocket`, return `unix_path().is_some_and(|path| path.exists())`.
          For `Tcp`, use `TcpStream::connect_timeout` and return true on
          success.
    - **Modify `crates/dosker-engine-qemu/src/qmp.rs`**
        - Replace `BufReader<UnixStream>` with `BufReader<EngineStream>`.
        - Change `connect` and `connect_until` to accept
          `&EngineEndpointKind`.
        - Keep the greeting, `qmp_capabilities`, `execute`, `quit`,
          `screendump`, and deadline behavior.
        - Remove the Unix-only connect helpers now held by `EngineStream`.
        - Move the existing `wait_writable_stops_at_the_deadline` test from
          the `qmp.rs` test module to `stream.rs`. Gate it with
          `#[cfg(all(test, unix))]` because it uses `libc::pipe`.
        - Do not leave a `#[cfg(unix)]`-only test module inside `qmp.rs`. The
          `qmp` module must compile on Windows. The Windows build must not
          reference `libc` from `qmp.rs`.
    - **Modify `crates/dosker-engine-qemu/src/session.rs`**
        - Pass `EngineEndpointKind::UnixSocket(self.paths.qmp.clone())` to
          `QmpClient::connect_until` until the session task replaces the field.
    - **Modify `crates/dosker-engine-qemu/src/lib.rs`**
        - Compile `qmp` and `serial_log` on every host. Add `pub mod stream;`
          and re-export `EngineStream`.
    - **Verify**: `cargo test -p dosker-engine-qemu --test stream --test qmp`

- [x] **Task 4: Windows QEMU session over host endpoints**

    - **Implements**: FR-021, FR-022, design Process Model and Engine Adapter
      `start`, `wait_ready`, `stop`, and `debug_endpoint`.
    - **Create `crates/dosker-engine-qemu/tests/session_endpoints.rs`**
        - Spawn a stand-in child (`sleep 30` on Unix and
          `cmd /C ping -n 30 127.0.0.1 > NUL` on Windows) with a command that
          carries `QemuEndpoints::for_spec`.
        - `session_reports_the_endpoints_from_its_command` — a session built
          from a command reports the same four endpoint names.
        - `session_omits_the_agent_endpoint_for_a_booter` — the booter session
          has no `serial_agent`.
        - Add `windows_ready_reports_an_early_exit` — gate this test with
          `#[cfg(windows)]`. Spawn
          `cmd /C "ping -n 2 127.0.0.1 > NUL & exit 1"`, call
          `ready(Duration::from_secs(3))`, and expect
          `EngineError::Exited(1)` in under two seconds. The short delay lets
          the Job Object assignment finish before the process exits.
        - Add `debug_endpoint_reports_unsupported_for_a_missing_gdb_endpoint`
          — gate this test with `#[cfg(windows)]`. Use a stand-in child, leave
          the gdb TCP port closed, and expect
          `EngineError::Unsupported("debugger")`.
    - **Update `crates/dosker-engine-qemu/tests/qemu_session.rs`**
        - Replace `QemuPaths` with `QemuEndpoints`.
        - Add the endpoints from `QemuEndpoints::for_spec` to the manual
          `EngineCommand` used by `ready_reports_an_early_exit`.
        - Keep the real-image tests under `#![cfg(unix)]`.
    - **Update the unit-test helpers in
      `crates/dosker-engine-qemu/src/session.rs`**
        - `delayed_agent` — return
          `AgentClient<SerialLog<EngineStream>>` and wrap the host side in
          `EngineStream::Unix`.
        - `session_with` — build `endpoints: QemuEndpoints::for_spec(&spec)`
          and set `EngineCommand.endpoints` to the same values.
        - `bind_stalled_qmp` — use
          `QemuEndpoints::for_spec(spec).expect("qemu endpoints").qmp.unix_path().expect("qmp path")`
          for the listener path.
        - Add `debug_endpoint_reports_unsupported_for_a_missing_gdb_path`
          under `#[cfg(all(test, unix))]`. Remove the gdb socket path before
          the call and expect `EngineError::Unsupported("debugger")`.
        - Keep these helpers and the four tests that use them under
          `#[cfg(all(test, unix))]`.
    - **Modify `crates/dosker-engine-qemu/src/session.rs`**
        - Remove `use std::os::unix::net::UnixStream`.
        - Change the agent field to
          `Option<AgentClient<SerialLog<EngineStream>>>`.
        - Replace the `paths: QemuPaths` field with
          `endpoints: QemuEndpoints`. Build it with
          `QemuEndpoints::from_command(&command)`.
        - In `spawn`, call `endpoints.remove_stale()` before the process
          starts.
        - In `ready`, take `agent_serial.as_ref()` and return
          `EngineError::NoAgent` when it is `None`. Keep the existing
          `poll_exit` check and the `AGENT_SOCKET_RETRY` loop. Each connect
          attempt gets a short deadline (add
          `AGENT_CONNECT_ATTEMPT: Duration = Duration::from_millis(100)`). Run
          `poll_exit` after each attempt. Do not call `EngineStream::connect`
          with the full readiness deadline, because that would hide an engine
          early exit on Windows. Keep the agent handshake and overall deadline
          behavior.
        - In `qmp_with_deadline`, use `QmpClient::connect_until` with the
          `qmp` endpoint.
        - In `endpoints`, return `self.endpoints.engine_endpoints()`.
        - In `debug_endpoint`, handle `DebugKind::Gdb` with
          `EngineStream::probe`. On Unix, a missing gdb socket path returns
          `EngineError::Unsupported("debugger")`. On Windows, a TCP probe with
          `DEBUG_PROBE_TIMEOUT: Duration = Duration::from_millis(250)` returns
          `Unsupported("debugger")` when the port refuses the connection. Only
          return `self.endpoints.gdb.display()` after the probe succeeds.
        - Keep `process_group` as `Some(self.child.id())` on Unix. On Windows,
          return `None`.
        - Gate the Unix-specific unit tests with `#[cfg(all(test, unix))]`.
    - **Remove `crates/dosker-engine-qemu/src/paths.rs`**
        - `QemuEndpoints` replaces it.
    - **Modify `crates/dosker-engine-qemu/src/lib.rs`**
        - Compile `session` on every host. Remove the `paths` module and its
          re-export.
    - **Modify `crates/dosker-engine-qemu/src/adapter.rs`**
        - Remove the `cfg(windows)` `start` and `start_with_command` stubs.
          Call `QemuSession::spawn` on every host.
    - **Verify**: `cargo test -p dosker-engine-qemu`
    - **Verify (Windows)**: `cargo test -p dosker-engine-qemu --test session_endpoints`

- [x] **Task 5: Windows Job Object and process termination**

    - **Implements**: FR-023, design Process Model and Host Platform
      Abstraction.
    - **Create tests in `crates/dosker-platform/tests/process.rs`**
        - `job_object_kills_an_assigned_process_when_dropped` — a `cmd.exe`
          child assigned to the job is gone after the job handle drops.
        - `job_object_keeps_an_assigned_process_alive_while_open` — the same
          child stays alive while the job handle is open.
        - `kill_group_terminates_the_named_process_on_windows` — the Windows
          implementation ends the named process.
        - Gate all three tests with `#[cfg(windows)]`.
    - **Modify `Cargo.toml`**
        - Add `"Win32_System_JobObjects"` to the workspace `windows-sys`
          features.
    - **Modify `crates/dosker-platform/Cargo.toml`**
        - Add `"Win32_System_JobObjects"` to the Windows `windows-sys`
          features.
    - **Modify `crates/dosker-platform/src/process/windows.rs`**
        - Add `pub struct JobObject { handle: HANDLE }`.
        - Add `pub fn kill_on_close() -> Result<Self, PlatformError>`.
          Call `CreateJobObjectW`. Call `SetInformationJobObject` with
          `JobObjectExtendedLimitInformation` and
          `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
        - Add `pub fn assign_child(&self, child: &std::process::Child) -> Result<(), PlatformError>`.
          Call `AssignProcessToJobObject` with `child.as_raw_handle()`.
        - Add `unsafe impl Send for JobObject {}`. The handle is owned and is
          used only through the job value.
        - Add `Drop` that calls `CloseHandle`.
        - Change `kill_group(pid)` to open the process with `PROCESS_TERMINATE`
          and call `TerminateProcess`. Keep `process_group` as `None`.
    - **Modify `crates/dosker-platform/src/process/mod.rs`**
        - Re-export `JobObject` under `#[cfg(windows)]`.
        - Update the `kill_group` doc comment. On Windows it terminates the
          named process, and the Job Object ends the engine when the supervisor
          exits.
    - **Modify `crates/dosker-engine-qemu/src/session.rs`**
        - Add `#[cfg(windows)] job: Option<JobObject>`.
        - In `spawn`, create the job before the process starts. After spawn,
          call `assign_child`. On failure, kill and wait for the child, then
          return `EngineError::Setup`.
        - Store the job in the session.
    - **Update the Windows process test**
        - Remove `kill_group_is_unsupported_on_windows`.
    - **Verify**: `cargo test -p dosker-platform --test process`
    - **Verify**: `cargo test -p dosker-engine-qemu`
    - **Verify (Windows)**: `cargo test -p dosker-platform --test process -- --nocapture`

- [x] **Task 6: Windows QEMU discovery, version checks, and checksum**

    - **Implements**: FR-021, design Assumptions, Security, and Engine
      Capability Matrix source rule.
    - **Create unit tests in `crates/dosker-engine-qemu/src/discovery.rs`**
        - `minimum_version_rejects_an_older_build` — `7.0.9` fails. Gate this
          test with `#[cfg(windows)]`.
        - `minimum_version_accepts_the_floor` — `7.1.0` passes. Gate this test
          with `#[cfg(windows)]`.
        - `version_pin_parses_a_bare_three_part_version` — `8.2.0` parses as
          `QemuVersion { major: 8, minor: 2, patch: 0 }`.
        - `version_pin_rejects_a_prefix_or_suffix` — `QEMU 8.2.0` and
          `8.2.0-debian` are rejected.
        - `version_pin_rejects_a_mismatch` — a pinned `8.2.0` rejects `8.2.1`.
        - `checksum_accepts_a_matching_file` — the expected digest passes.
        - `checksum_rejects_a_modified_file` — a changed byte fails.
        - `windows_program_name_gets_an_exe_suffix` — a bare name resolves to
          `<name>.exe` on Windows. Gate this test with `#[cfg(windows)]`.
        - `windows_path_candidates_include_the_install_roots` — the three
          install roots are searched in order. Gate this test with
          `#[cfg(windows)]`.
    - **Modify `crates/dosker-engine-qemu/src/discovery.rs`**
        - Add `pub const QEMU_SHA256_ENV: &str = "DOSKER_QEMU_SHA256";`.
        - Add `pub const QEMU_VERSION_ENV: &str = "DOSKER_QEMU_VERSION";`.
        - Add `pub const MIN_QEMU_VERSION: QemuVersion = QemuVersion { major: 7, minor: 1, patch: 0 };`.
          This is the existing `SCREENDUMP_FORMAT_VERSION` floor.
        - Add `fn program_candidates(name: &OsStr) -> Vec<OsString>`. On
          Windows add the `PATHEXT` suffixes, including `.EXE`, when the name
          has no extension.
        - Add `fn windows_install_candidates(name: &OsStr) -> Vec<PathBuf>`.
          Return `%ProgramFiles%\qemu`, `%ProgramFiles(x86)%\qemu`, and
          `%LOCALAPPDATA%\Programs\qemu` in that order.
        - Change `resolve_program` to use `program_candidates`. For a value with
          a path separator, try each candidate in the named directory. For a
          bare name, search `PATH` first, then
          `windows_install_candidates` on Windows.
        - Add `fn file_sha256(path: &Path) -> Result<String, EngineError>`.
          Use a streaming SHA-256 read so a large binary does not load into
          memory.
        - Add `fn verify_checksum(path: &Path, expected: &str) -> Result<(), EngineError>`.
          Compare the lowercase hexadecimal digest.
        - Add `QemuVersion::parse_pin(value: &str) -> Option<Self>`. Accept
          exactly `major.minor.patch` with decimal components and no prefix or
          suffix. This is the format of `DOSKER_QEMU_VERSION`.
        - Add `fn check_version(version: QemuVersion, pinned: Option<&str>) -> Result<(), EngineError>`.
          Parse `pinned` with `QemuVersion::parse_pin`. An invalid pin returns
          `EngineError::Setup`. A pinned mismatch names the expected and found
          versions. On Windows reject a version below `MIN_QEMU_VERSION` and
          name the floor.
        - In `QemuBinary::from_path`, return `EngineError::NotFound(path)` when
          the path is not a file. Verify `DOSKER_QEMU_SHA256` before it runs the
          binary. Probe the version. Read `DOSKER_QEMU_VERSION` and call
          `check_version`.
    - **Modify `crates/dosker-engine-qemu/Cargo.toml`**
        - Add `sha2.workspace = true`.
    - **Modify `crates/dosker-engine-qemu/src/lib.rs`**
        - Re-export `MIN_QEMU_VERSION`, `QEMU_SHA256_ENV`, and
          `QEMU_VERSION_ENV`.
    - **Verify**: `cargo test -p dosker-engine-qemu --lib discovery::`
    - **Verify**: `cargo test -p dosker-engine-qemu --test discovery`

- [ ] **Task 7: Windows QEMU smoke suite**

    - **Implements**: FR-021, FR-022, FR-023, design Testing Strategy and
      M1-W definition of done.
    - **Do not re-implement the existing Windows foundation**
        - `crates/dosker-platform/src/data_root.rs` and
          `crates/dosker-platform/src/fs.rs` already implement the data root and
          atomic writes. Their existing Windows tests cover them.
        - The named-pipe DACL proof is
          `pipe_acl_has_one_ace_for_current_user` in
          `crates/dosker-test-support/tests/transport_contract.rs`. This task
          relies on that Slice 1 proof and does not add a second DACL assertion.
    - **Create `crates/dosker-cli/tests/windows_qemu_runtime.rs`**
        - Add `#![cfg(all(windows, feature = "qemu-engine"))]`.
        - Add `fn image() -> Option<PathBuf>`. Return `None` when
          `integration_enabled()` is false. When integration is enabled and
          `qemu_image()` fails, print the error and panic with the missing
          prepared image path.
        - Tests that use the image path start with
          `let Some(image) = image() else { return; };`. Tests that do not use
          the path call `if image().is_none() { return; }` so the binding is not
          unused.
        - Reuse `common::{CliRoot, stderr_text, stdout_text}`. Import with
          `CliRoot::run_with_qemu` and `qemu_image()`. Do not use
          `CliRoot::start` or `CliRoot::import_image`, which select the mock
          engine.
        - Serialize the tests with one `static Mutex<()>` because they use real
          QEMU and fixed container names.
        - Add `prepared_image_boots_execs_and_restarts` — import the prepared
          image, `run -d`, `exec ECHO HELLO`, `stop`, `start`, `exec` again.
        - Add `control_transport_is_a_named_pipe` — the record control endpoint
          is `EndpointRecord::NamedPipe { name }` and `exec` succeeds through
          it.
        - Add `attach_provides_a_line_shell` — send `ECHO HELLO` and `exit`
          through `attach`, then assert `HELLO` and a running container.
        - Add `engine_endpoints_are_loopback_tcp` — `serial_agent`, `qmp`,
          `monitor`, and `gdb` are `EndpointRecord::Tcp` with host
          `127.0.0.1`. Connect to `qmp`.
        - Add `monitor_accepts_a_loopback_connection` — connect to the recorded
          monitor TCP endpoint with a short connect timeout. The test asserts
          the connection, not a banner.
        - Add `debug_prints_a_loopback_target` — `debug --gdb` prints
          `target remote 127.0.0.1:<port>` and that port accepts a connection.
        - Add `supervisor_crash_leaves_no_engine` — start a container, kill the
          supervisor PID, wait for the engine PID to disappear, then assert
          `ps -a` reports `exited`.
        - Add `vnc_listens_on_loopback` — start with `--display vnc` and
          `--vnc-port`, then read the `RFB ` greeting from
          `127.0.0.1:<port>`.
        - Add `cp_round_trips_a_small_file` — upload a host file, run `TYPE`,
          then download the file and compare the bytes.
        - Add `screenshot_writes_a_file` — run `screenshot` and check the PPM
          or PNG header.
        - Add `engines_and_doctor_report_qemu` — both commands report the
          pinned version, the executable path, and the capability list.
        - Add `logs_show_engine_and_serial_output` — `logs --engine` has the
          rendered command and `logs --serial` contains the `HELLO` command.
        - Add `state_and_logs_live_under_the_data_root` — `state.json`,
          `engine.log`, `serial.log`, and `container.log` exist in the
          container directory.
    - **Use the existing test helpers unchanged**
        - `crates/dosker-cli/tests/common/mod.rs` already provides
          `run_with_qemu`, `make_disk_image`, and `CliRoot` on Windows. Do not
          change it in this task.
    - **Verify (Windows)**:
      `cargo test -p dosker-cli --features qemu-engine --test windows_qemu_runtime -- --nocapture`
      with `DOSKER_INTEGRATION=1`, `DOSKER_QEMU`, `DOSKER_QEMU_IMAGE`, and
      `DOSKER_QEMU_SHA256` set.
    - **Verify (Windows, clippy)**:
      `cargo clippy -p dosker-cli --all-targets --features mock-engine,qemu-engine -- -D warnings`
    - **Verify (Windows, DACL proof)**:
      `cargo test -p dosker-test-support --test transport_contract`
    - **Verify (Windows, mock binary)**:
      `cargo build -p dosker-test-support --bin mock-engine`
    - **Verify (Windows, named pipe)**:
      `cargo test -p dosker-cli --features mock-engine --test detached_lifecycle`

- [ ] **Task 8: Windows QEMU CI job**

    - **Implements**: FR-021, FR-022, FR-023, design Testing Strategy and
      M1-W definition of done.
    - **Modify `.github/workflows/ci.yml`**
        - Add a `windows-qemu` job with `runs-on: windows-latest` and
          `timeout-minutes: 45`.
        - Add `actions/checkout@v4`, `dtolnay/rust-toolchain@1.98.1`, and
          `Swatinem/rust-cache@v2`.
        - Add a prerequisite step that fails with a clear message when
          `DOSKER_WINDOWS_QEMU_URL`, `DOSKER_WINDOWS_QEMU_SHA256`,
          `DOSKER_WINDOWS_QEMU_VERSION`, or `DOSKER_QEMU_IMAGE_URL` is empty.
          `DOSKER_WINDOWS_QEMU_VERSION` is the bare `X.Y.Z` form.
        - Add a QEMU install step in PowerShell. The manual prerequisite
          supplies a ZIP archive. Download `DOSKER_WINDOWS_QEMU_URL` to
          `$env:RUNNER_TEMP\qemu.zip`. Check the archive SHA-256 against
          `DOSKER_WINDOWS_QEMU_SHA256`. Extract it. Find
          `qemu-system-i386.exe`. Append `DOSKER_QEMU`,
          `DOSKER_QEMU_SHA256`, and `DOSKER_QEMU_VERSION` to
          `$env:GITHUB_ENV`. The binary checksum comes from `Get-FileHash` and
          is lowercased with `ToLowerInvariant()`.
        - Add an image step. Download `DOSKER_QEMU_IMAGE_URL`. If it ends in
          `.zip`, extract it and select the first `.img` file. Otherwise save
          it as `dosker-agent.img`. Append `DOSKER_QEMU_IMAGE` to
          `$env:GITHUB_ENV`.
        - Add a smoke step with `DOSKER_INTEGRATION: "1"` that runs
          `cargo test -p dosker-cli --features qemu-engine --test windows_qemu_runtime -- --nocapture`.
        - Keep the existing `test (windows-latest)` job. Its build and clippy
          steps enable `dosker-cli/qemu-engine`, so they compile and lint
          `windows_qemu_runtime`. Its `cargo test` step enables only
          `dosker-cli/mock-engine`, so the smoke tests run only in the
          `windows-qemu` job. Keep the smoke file clippy-clean.
    - **Verify**: `ruby -e 'require "yaml"; YAML.load_file(".github/workflows/ci.yml")'`

- [ ] **Task 9: Windows runtime documentation**

    - **Implements**: FR-021, FR-022, FR-023, design Non-Functional
      Requirements and M1-W definition of done.
    - **Create `docs/windows.md`**
        - State the supported host: Windows x86_64. State that Windows arm64 is
          out of scope.
        - State the data root `%LOCALAPPDATA%\dosker` and the `DOSKER_ROOT`
          override.
        - State the named-pipe control endpoint and its current-user DACL.
        - State that engine endpoints bind to `127.0.0.1` and that VNC uses a
          loopback port at or above `5900`.
        - State that the engine runs in a Job Object with kill-on-close.
        - List the QEMU search order: `DOSKER_QEMU`, `PATH`,
          `%ProgramFiles%\qemu`, `%ProgramFiles(x86)%\qemu`, and
          `%LOCALAPPDATA%\Programs\qemu`.
        - List `DOSKER_QEMU_SHA256`, `DOSKER_QEMU_VERSION` (format `X.Y.Z`),
          and the minimum version `7.1.0`.
        - List the CI variables `DOSKER_WINDOWS_QEMU_URL`,
          `DOSKER_WINDOWS_QEMU_SHA256`, `DOSKER_WINDOWS_QEMU_VERSION`, and
          `DOSKER_QEMU_IMAGE_URL`.
        - List the smoke command with `DOSKER_INTEGRATION=1`.
        - State that `--display window` is not supported on Windows at present.
          Use `--display none` or `--display vnc`.
        - State that NAT is not yet supported.
        - State that DOSker does not sandbox untrusted images.
    - **Modify `README.md`**
        - Update the introduction to name the QEMU runtime and the Windows
          x86_64 host.
        - Add a Windows runtime section. Link to `docs/windows.md`.
        - List the Windows CI repository variables.
        - Keep the existing FreeDOS, QEMU, and nightly sections.
    - **Verify**: `git add -N docs/windows.md && git diff --check`
    - **Verify**: `rg -n "DOSKER_WINDOWS_QEMU|DOSKER_QEMU_SHA256|Job Object|127\\.0\\.0\\.1" README.md docs/windows.md`

## Approval Handoff

When the user approves this plan:

1. Set this plan's **Status** to `Approved`.
2. In `.sdd/specs/2026-11-01-dosker/roadmap.md`, set the
   `windows-runtime-track` slice **Status** to `Planned`.
3. Set that slice's **Plan** field to
   [`plan-windows-runtime-track.md`](plan-windows-runtime-track.md).

Do not start implementation before these three updates. The plan stays
`Draft` until the user approves it.

