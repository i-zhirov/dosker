# Bochs Adapter Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-20
- **Status**: Implemented
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Roadmap**: `.sdd/specs/2026-11-01-dosker/roadmap.md` — slice: `bochs-adapter`
- **Depends on**: `qemu-runtime-slice`, `volumes-and-nat`, `development-mode`
- **Manual prerequisites**: the Bochs headless, serial, and debugger spike,
  recorded complete at `.sdd/specs/2026-11-01-dosker/assets/bochs-spike.md`
  with every `boot`, `serial`, `stop`, `headless`, and `debug_native` row
  `proven`, and the pty and `SIGINT` evidence in
  `assets/bochs-spike/debugger/pty-session.txt`, `pty-bochsrc`, and
  `pty-bochs-out.txt`. The reference build is Homebrew Bochs `3.1` at
  `/opt/homebrew/bin/bochs` (SHA-256
  `8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c`),
  built with `--enable-debugger --enable-debugger-gui --with-nogui
  --with-sdl2 --with-term`. The optional gdbstub build is Bochs
  `REL_3_1_FINAL` at `$HOME/bochs-gdbstub/bin/bochs` (SHA-256
  `56aa939652578a81bb2e0f915fc48b80643053e24a5e046ba7af86c65643ae95`),
  built with `--enable-gdb-stub --with-nogui --enable-cpu-level=6`. The
  prepared image is `agent/fixtures/dosker-agent.img` (SHA-256
  `17a621ab7e3658091104f367e5b4bb23f81b608bbd8760c4be6b5cf46c1131df`),
  named by `DOSKER_BOCHS_IMAGE` for the integration suite; the integration
  import also needs the built agent at `agent/build/DOSKER.EXE`. Declaring
  `nat` on Bochs is out of scope in this slice: it requires its own NAT
  spike, which has not run. A Unix host is needed for the pty endpoint and
  the real-engine tests.
- **Goal**: Deliver the Bochs adapter behind the capability model —
  discovery and version probing, `bochsrc` generation with the geometry
  rule, the serial socket, `nogui`, the internal debugger pty bridged to a
  Unix socket, runtime engine selection, `engines`/`doctor`/`debug`
  reporting, and the spike-gated capability set.

## Constraints

- The engine names are `qemu`, `dosbox-x`, and `bochs`.
- The capability names are `boot`, `serial`, `stop`, `headless`,
  `host_dir_live`, `debug_native`, `gdbstub`, `screenshot`, `vnc`, `nat`,
  and `audio`.
- A capability is declared only when its spike row says `proven`. An
  unproven row stays undeclared, and the adapter rejects a `REQUIRES` for
  it.
- The Bochs capability set is exactly `boot`, `serial`, `stop`,
  `headless`, and `debug_native`, in `Capability::ALL` order.
- Bochs never declares `gdbstub`, `host_dir_live`, `screenshot`, `vnc`, or
  `nat` in this slice. The gdbstub is a build-dependent, optional
  alternative and not part of the reference capability set.
- `crates/dosker-engine/src/provider.rs` is the single source of truth for
  implemented engine names and capability sets. The QEMU, DOSBox-X, and
  Bochs `capabilities()` functions and the CLI read it.
- The provider table order is `qemu`, `dosbox-x`, `bochs`.
- The supported-engine list is `qemu, dosbox-x, and bochs`.
- `--engine` on the CLI overrides the `ENGINE` pin for one command.
- The image config records the engine pin.
- `ENGINE-CONFIG` raw text is passed to the named adapter. QEMU splits it
  into command-line arguments with shell-like quoting. DOSBox-X treats it
  as configuration lines before `[autoexec]`. Bochs treats it as
  `bochsrc` lines.
- Each raw `ENGINE-CONFIG bochs` line is appended verbatim after the fixed
  `bochsrc` keys, in order. A later key overrides an earlier one.
- `ENGINE-CONFIG` does not change the declared capability set. A file that
  needs a feature must still use `REQUIRES`.
- `REQUIRES` is checked after engine selection and before any VM starts.
  The final selected engine must satisfy every `REQUIRES` in the
  Doskerfile.
- The missing-capability message forms are the existing ones:
  `engine {engine} does not provide the {capability} capability; the
  {provider} engine provides it` for one provider, the `{first} and
  {second}` form for two, `no engine provides it` for zero, and the
  `{first}, {second}, and {third}` form for three or more.
- A known engine name without an adapter prints `engine {engine} is not
  available; supported engines are {list}`. An arbitrary string prints
  `unknown engine: {engine}`.
- A `DebugKind::Native` request uses the native debugger. A
  `DebugKind::Gdb` request uses the gdbstub. Bochs maps `Native` to the
  internal debugger and rejects `Gdb`.
- The default display is `none`.
- `start` waits up to 60 seconds for the agent `HELLO`.
- The default machine is a 486DX2 with 16 MB of RAM, VGA, SB16, NE2000,
  two serial ports, and an IDE disk.
- The Bochs adapter maps the design's default machine to `cpu:
  model=i486dx4` and `megs: 16`, with an IDE disk, VGA, and two serial
  ports. `MACHINE` overrides are not applied on Bochs in this slice.
- The control agent owns COM2. COM1 stays free for the guest.
- The agent owns `com{spec.control_port}`. When `control_port == 2`, the
  agent line is `com2` and the guest line is `com1`. When
  `control_port == 1`, the agent line is `com1` and the guest line is
  `com2`. The non-agent line is `enabled=1, mode=null`.
- The image size must equal `C*H*S*512`. The adapter uses 16 heads, 32
  sectors per track, and 512-byte sectors, and derives the cylinder count
  from the disk size. The 32 MiB fixture is `cylinders=128, heads=16,
  spt=32`.
- The cylinder count must be at least 1 and at most 16383. Any other size
  is an error before boot.
- The boot command is `bochs -q -f <bochsrc>`.
- The `bochsrc` carries `romimage: file=$BXSHARE/BIOS-bochs-latest`,
  `vgaromimage: file=$BXSHARE/VGABIOS-lgpl-latest.bin`, `cpu:
  model=i486dx4`, `megs: 16`, `ata0-master: type=disk,
  path="<image>", mode=flat, cylinders=<C>, heads=<H>, spt=<S>`,
  `boot: disk`, `display_library: nogui`, `com1: enabled=1, mode=null`,
  `com2: enabled=1, mode=socket-server, dev=localhost:<port>`, the quoted
  `log: "<log_dir>/bochs.log"`, and the pinned `panic`, `error`, `info`,
  and `debug` actions.
- `cpu: model=486` is invalid; the model is `i486dx4`.
- The serial line is `com2: enabled=1, mode=socket-server,
  dev=localhost:<port>`. Bochs listens and the host connects.
- The headless display is `display_library: nogui`. A window display is
  not supported: the spike never opened a window, so `render_config` and
  `build_command` reject `DisplayMode::Window` with
  `EngineError::Unsupported("window display")`, and no SDL display line is
  rendered.
- The stop mechanism is the agent `SHUTDOWN`, then `SIGTERM` to the engine
  process group. The recorded exit code is 143.
- The internal debugger is started with `-dbg`. The child gets the pty
  slave as stdin and stdout. The parent holds the master, is the only
  reader of it, and a client cannot share the child's slave side.
- One reader thread drains the master from spawn. It appends every drained
  byte to `engine.log` and forwards the bytes to a connected debug client.
  Output that arrives before a client connects is held in a bounded buffer
  and replayed when the client attaches.
- The native debugger is exposed as a Unix socket at
  `engine_socket_path(root, container_id, "debug")`. The bridge binds it
  lazily on the first `debug_endpoint(DebugKind::Native)` call; the client
  connects to the socket; the bridge forwards client bytes to the master
  and the reader's output to the client.
- `dosker debug` sends `SIGINT` to the engine process group to break the
  running guest at the debugger prompt, then prints `native debugger
  <socket>`. The session writes `c` to the master after spawn so the guest
  boots.
- `-dbg` is added only on Unix and only when discovery found the internal
  debugger in the build.
- With `-dbg`, the child's stdout is the pty and the pty reader appends the
  drained output to `engine.log`; stderr goes to `engine.log` directly.
  The `log:` key writes `bochs.log`. Without `-dbg`, stdout and stderr go
  to `engine.log` and no pty exists.
- `crates/dosker-platform/src/endpoint.rs` accepts `debug` as an engine
  socket name, beside `agent`, `qmp`, `monitor`, and `gdb`.
- The gdbstub, when present in an optional build, is configured with
  `gdbstub: enabled=1, port=<port>, text_base=0, data_base=0,
  bss_base=0`. It is never declared as a capability.
- `bochs --version` is unusable as a version probe: the startup banner is
  printed before argument parsing, so `--version` prints the banner and
  then fails on the unknown option. Discovery runs the binary with a
  minimal config, writes `quit` to its stdin, and parses the
  `Bochs x86 Emulator <version>` banner and the internal-debugger line
  from the same spawn. The probe config sets `log: -` so the probe writes
  no `bochsout.txt` into the working directory.
- The reference build reports version `3.1`.
- `DOSKER_BOCHS` overrides discovery, `DOSKER_BOCHS_VERSION` pins the
  exact version, and `DOSKER_BOCHS_SHA256` pins the checksum.
- The default program name is `bochs`.
- The Bochs config file is `bochsrc` in the container directory.
- Named volumes and host directories are rejected on Bochs at create time
  and at config render time.
- An image with `MACHINE net=ne2000` is rejected on Bochs because `nat` is
  not declared. The NIC is never silently omitted.
- Bochs leaves a `<disk>.lock` file beside a disk image when it is killed.
  Each container uses a private disk, and the session removes that lock
  file before spawning the engine.
- `dosker engines` prints one line per known engine. A missing engine
  prints `{name} not found: {error}`. It exits 1 only when no engine is
  found.
- `dosker doctor` prints one line per engine. It exits 0 when the data
  root is writable and at least one engine is found.
- The docs capability table marks a capability the spike tested but did
  not prove as `unproven`. For Bochs, `gdbstub` is `no`, `screenshot` and
  `vnc` are `unproven`, `host_dir_live` and `nat` are `unproven`, and
  `audio` is `no`.
- In-DOS `RUN`, the build path, screenshot, VNC, host directories, NAT, a
  gdbstub capability, and any capability the spike does not prove are out
  of scope.
- The QEMU build resolver and the in-DOS `RUN` path stay QEMU-only.
- The Bochs crate must compile on Linux x86_64, macOS arm64, and Windows
  x86_64. The pty, the debug socket, and the real-engine integration tests
  run on Unix.
- No transient references in code, identifiers, test names, strings, or
  comments. Do not name slices, tasks, requirement IDs, milestones, or
  plan paths in product code.

## Interfaces

The following names and signatures are fixed across tasks. Adjust none of
them without changing every consumer in the same task.

`crates/dosker-engine/src/provider.rs` gains the Bochs entry. The QEMU,
DOSBox-X, and Bochs `capabilities()` functions return
`dosker_engine::declared_capabilities`.

```rust
// dosker-engine
pub const IMPLEMENTED_ENGINES: &[EngineProvider]; // qemu, dosbox-x, bochs

// dosker-engine-bochs
pub const ENGINE_NAME: &str = "bochs";
pub const BOCHS_ENV: &str = "DOSKER_BOCHS";
pub const BOCHS_VERSION_ENV: &str = "DOSKER_BOCHS_VERSION";
pub const BOCHS_SHA256_ENV: &str = "DOSKER_BOCHS_SHA256";
pub const DEFAULT_BOCHS: &str = "bochs";

// config.rs
pub const CONFIG_FILE_NAME: &str = "bochsrc";
pub const HEADLESS_DISPLAY: &str = "nogui";
pub const CPU_MODEL: &str = "i486dx4";
pub const MEMORY_MB: u32 = 16;

pub enum Debugger {
    Internal,
    None,
}

pub struct BochsBinary {
    pub path: PathBuf,
    pub version: String,
    pub debugger: Debugger,
}
impl BochsBinary {
    pub fn discover() -> Result<Self, EngineError>;
    pub fn from_path(path: &Path) -> Result<Self, EngineError>;
    pub fn version_string(&self) -> &str;
    pub fn path(&self) -> &Path;
    pub fn has_internal_debugger(&self) -> bool;
}

pub struct DiskGeometry {
    pub cylinders: u32,
    pub heads: u8,
    pub sectors: u8,
}
impl DiskGeometry {
    pub fn from_size(bytes: u64) -> Result<Self, EngineError>;
}

pub struct BochsEndpoints {
    pub agent_serial: Option<EngineEndpointKind>,
}
impl BochsEndpoints {
    pub fn for_spec(spec: &EngineSpec) -> Result<Self, EngineError>;
    pub fn from_command(command: &EngineCommand) -> Result<Self, EngineError>;
    pub fn engine_endpoints(&self) -> Vec<EngineEndpoint>;
    pub fn remove_stale(&self) -> Result<(), EngineError>;
}

pub fn render_config(spec: &EngineSpec, endpoints: &BochsEndpoints)
    -> Result<String, EngineError>;
pub fn write_config(spec: &EngineSpec, endpoints: &BochsEndpoints)
    -> Result<PathBuf, EngineError>;
pub fn capabilities() -> CapabilitySet;

pub struct BochsAdapter {
    binary: BochsBinary,
    version: String,
    processes: Arc<dyn ProcessSupervisor>,
}
impl BochsAdapter {
    pub fn new(binary: BochsBinary, processes: Arc<dyn ProcessSupervisor>) -> Self;
    pub fn discover(processes: Arc<dyn ProcessSupervisor>) -> Result<Self, EngineError>;
}

pub struct BochsSession {
    // fields are crate-private
}
impl BochsSession {
    pub fn spawn(spec: &EngineSpec, processes: &dyn ProcessSupervisor,
        command: EngineCommand) -> Result<Self, EngineError>;
}

// dosker-cli
pub fn discover(engine: &str) -> Result<DiscoveredEngine, EngineError>;
pub fn build(engine: &str) -> Result<Box<dyn EngineAdapter>, CliError>;
```

`PtyPair`, `DebugChannel`, `DebugChannelState`, `DebugReader`,
`DebugBridge`, `BochsStream`, and `SerialLog<T>` are crate-private types in
`dosker-engine-bochs`; they do not appear in a public signature.

## Tasks

- [x] **Task 1: Provider table and capability messages for Bochs**
    - **Implements:** FR-011, FR-016; design §Engine Strategy, §Engine
      Capability Matrix, §Error Handling.
    - **Precondition before any code:** read the completed Results table in
      `assets/bochs-spike.md`. If `boot`, `serial`, `headless`, `stop`, or
      `debug_native` is not `proven`, stop and report. Record the Decisions
      table; every later task consumes it.
    - **Modify `crates/dosker-engine/src/provider.rs`**
        - Add the `EngineProvider { name: "bochs", capabilities: &[Boot,
          Serial, Stop, Headless, DebugNative] }` entry after `dosbox-x`.
        - Add a `#[cfg(test)] mod tests` case
          `capability_message_uses_the_two_provider_form` that calls
          `format_capability_message("mock", Capability::Headless,
          &["qemu", "dosbox-x"])` and pins `engine mock does not provide the
          headless capability; the qemu and dosbox-x engines provide it`.
    - **Modify `crates/dosker-engine/tests/provider.rs`**
        - `declared_capabilities_match_the_bochs_proven_rows` — pins the
          Bochs entry to a literal `[Capability; 5]` slice
          `[Boot, Serial, Stop, Headless, DebugNative]`. Do not compare to a
          production constant.
        - `providers_are_in_table_order` — `providers(Capability::Headless)`
          returns `["qemu", "dosbox-x", "bochs"]`.
        - `capability_message_uses_the_three_provider_form` — replaces
          `capability_message_uses_the_two_provider_form`;
          `capability_message("mock", Capability::Headless)` pins `engine
          mock does not provide the headless capability; the qemu,
          dosbox-x, and bochs engines provide it`.
        - `supported_engines_message_lists_the_implemented_engines` — pins
          `qemu, dosbox-x, and bochs`.
        - Keep `capability_message_uses_the_one_provider_form`,
          `capability_message_uses_the_zero_provider_form`,
          `check_requires_rejects_a_missing_capability`,
          `check_requires_rejects_an_unknown_capability`, and
          `check_requires_honors_the_supplied_capability_set` unchanged.
    - **Expected red window:** the workspace suite is red from this task
      until Task 9. The CLI tests still pin the two-engine messages and the
      `engine bochs is not available` message. Only this task's
      `cargo test -p dosker-engine` is expected to pass in that window;
      `cargo test --workspace` passes again at Task 9.
    - **Verify:** `cargo test -p dosker-engine`

- [x] **Task 2: Bochs crate scaffold, discovery, version banner, and
  debugger detection**
    - **Implements:** FR-011; design §Engine Strategy, §Testing Strategy;
      the spike Environment and Decisions tables.
    - **Create `crates/dosker-engine-bochs/Cargo.toml`**
        - `[dependencies]`: `dosker-core.workspace = true`,
          `dosker-engine.workspace = true`,
          `dosker-platform.workspace = true`,
          `dosker-agent-proto.workspace = true`, `sha2.workspace = true`.
        - `[target.'cfg(unix)'.dependencies]`: `libc.workspace = true`.
        - `[dev-dependencies]`: `tempfile.workspace = true`.
    - **Modify `Cargo.toml` (workspace)**
        - Add `"crates/dosker-engine-bochs"` to `members`.
        - Add `dosker-engine-bochs = { path =
          "crates/dosker-engine-bochs" }` to `[workspace.dependencies]`.
    - **Create `crates/dosker-engine-bochs/src/lib.rs`**
        - Declare the `adapter` and `discovery` modules. Later tasks extend
          this list as their modules land.
        - Re-export `capabilities`, `ENGINE_NAME`, `BochsBinary`,
          `Debugger`, and the four discovery constants.
    - **Create `crates/dosker-engine-bochs/src/discovery.rs`**
        - `BOCHS_ENV`, `BOCHS_VERSION_ENV`, `BOCHS_SHA256_ENV`,
          `DEFAULT_BOCHS`, `PROBE_TIMEOUT = Duration::from_secs(5)`, and
          `VERSION_MARKER = "Bochs x86 Emulator "`.
        - `Debugger { Internal, None }` with `Clone, Copy, Debug, PartialEq,
          Eq`.
        - `BochsBinary { pub path, pub version, pub debugger }`.
        - `BochsBinary::discover()` — reads `DOSKER_BOCHS` or `bochs`,
          resolves a bare name on `PATH`, and adds `PATHEXT` candidates on
          Windows. Mirror `crates/dosker-engine-dosbox-x/src/discovery.rs`.
        - `BochsBinary::from_path(path)` — checks the file, verifies
          `DOSKER_BOCHS_SHA256` when set, runs the probe, and checks
          `DOSKER_BOCHS_VERSION` against the returned version when set.
        - `BochsBinary::version_string`, `BochsBinary::path`, and
          `BochsBinary::has_internal_debugger`.
        - `probe(path, timeout) -> Result<(String, Debugger), EngineError>`
          — writes a minimal `bochsrc` to
          `std::env::temp_dir().join(format!("dosker-bochs-probe-{}-{}.bochsrc",
          std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)))`, with a
          `static COUNTER: AtomicU64` so parallel tests never share a file, and
          the keys
          `romimage: file=$BXSHARE/BIOS-bochs-latest`,
          `vgaromimage: file=$BXSHARE/VGABIOS-lgpl-latest.bin`,
          `cpu: model=i486dx4`, `megs: 16`, `display_library: nogui`,
          `log: -`, and the four action keys. `log: -` sends the Bochs log to
          stdout, so the probe writes no `bochsout.txt` into the working
          directory and the log lines join the piped output.
        - The probe never runs `bochs --version`: the startup banner is
          printed before argument parsing, so `--version` prints the banner
          and then fails on the unknown option. The probe spawns
          `bochs -dbg -q -f <config>` with stdin piped and stdout/stderr
          piped; one spawn therefore yields both the banner and the
          internal-debugger line.
        - The probe writes `quit\n` and drops stdin, ignoring a broken pipe;
          polls `try_wait` every 20 milliseconds until exit or the deadline;
          kills and reaps on timeout; parses the banner from the combined
          output and returns the version with `Debugger::Internal` when the
          output contains `Bochs internal debugger`, otherwise
          `Debugger::None`; ignores the exit status when the banner is
          present, so a build without the internal debugger (which rejects
          `-dbg`) still reports its version and `Debugger::None`; a timeout
          with no banner returns `EngineError::Setup("bochs version probe
          timed out")`; removes the temporary config.
        - `parse_version(output)` — takes the token after the first
          `VERSION_MARKER` occurrence on a line. It stays private; the
          banner tests reach it through `BochsBinary::from_path`.
        - Add a `#[cfg(test)] mod tests` with
          `probe_times_out_when_the_binary_hangs` calling the private
          `probe` with a 200-millisecond timeout and pinning the timeout
          error.
    - **Create `crates/dosker-engine-bochs/src/adapter.rs`**
        - `ENGINE_NAME`.
        - `capabilities()` — returns
          `dosker_engine::declared_capabilities(ENGINE_NAME)
          .expect("bochs is an implemented engine")`.
    - **Create `crates/dosker-engine-bochs/tests/discovery.rs`**
        - Add `write_fake_bochs(dir, body) -> PathBuf` — writes an executable
          shell script that ignores its arguments and runs `body`; every
          probe test below uses it.
        - `parses_the_version_from_the_banner` — the fake binary prints
          `Bochs x86 Emulator 3.1`; `BochsBinary::from_path` pins
          `version_string()` to `3.1`.
        - `parses_the_version_from_the_log_line` — the fake binary prints
          `00000000000i[      ] Bochs x86 Emulator 3.1`;
          `BochsBinary::from_path` pins `3.1`.
        - `reports_the_internal_debugger_from_the_debugger_line` — the fake
          binary prints the banner and `Bochs internal debugger, type
          'help' for help or 'c' to continue`; pins
          `has_internal_debugger()`.
        - `reports_no_internal_debugger_when_the_banner_has_no_debugger_line`
          — a fake binary prints only the banner and exits 1; pins the
          version and `Debugger::None`.
        - `probe_writes_quit_to_the_process_stdin` — the fake binary reads
          one line and writes it to `<script>.stdin`; pins `quit`.
        - `probe_passes_a_minimal_config` — the fake binary copies the `-f`
          argument's file to `<script>.config`; pins `cpu:
          model=i486dx4`, `display_library: nogui`, and `log: -`.
        - `rejects_a_missing_binary` — pins `EngineError::NotFound`.
        - `checksum_pin_accepts_a_matching_file` — pins the
          `DOSKER_BOCHS_SHA256` path.
        - `checksum_pin_rejects_a_modified_file` — pins the mismatch error.
        - `version_pin_rejects_a_mismatch` — pins the expected and found
          versions in the error.
        - `discovers_the_binary_named_by_the_environment` — pins
          `DOSKER_BOCHS` over `PATH`.
        - Add `static ENV_LOCK: Mutex<()>` and an `EnvRestore` guard; every
          test that sets `DOSKER_BOCHS`, `DOSKER_BOCHS_VERSION`, or
          `DOSKER_BOCHS_SHA256` holds the lock and restores the prior value
          on drop.
        - Add a `#[cfg(all(test, windows))]` unit test in
          `src/discovery.rs` that pins the `program_candidates` `PATHEXT`
          list for `bochs`.
    - **Create `crates/dosker-engine-bochs/tests/capability.rs`**
        - `declares_exactly_the_proven_rows` — pins `capabilities()` to
          `[Boot, Serial, Stop, Headless, DebugNative]`.
        - `declares_boot_serial_stop_headless_and_debug_native` — pins the
          five rows.
        - `never_declares_the_six_capabilities_outside_the_proven_set` —
          pins `gdbstub`, `host_dir_live`, `screenshot`, `vnc`, `nat`, and
          `audio` as absent.
    - **Verify:** `cargo test -p dosker-engine-bochs`

- [x] **Task 3: Disk geometry rule**
    - **Implements:** FR-011; design §Build And Runtime Semantics; the
      spike Disk geometry section.
    - **Create `crates/dosker-engine-bochs/src/geometry.rs`**
        - `HEADS: u8 = 16`, `SECTORS_PER_TRACK: u8 = 32`, `SECTOR_BYTES:
          u64 = 512`, `CHS_BYTES: u64 = 262_144`, `MAX_CYLINDERS: u32 =
          16_383`.
        - `DiskGeometry { pub cylinders: u32, pub heads: u8, pub sectors:
          u8 }`.
        - `DiskGeometry::from_size(bytes)` — rejects zero, a size that is
          not a multiple of `CHS_BYTES`, and a cylinder count above
          `MAX_CYLINDERS` with `EngineError::Setup`. The message names the
          size and the required multiple.
    - **Create `crates/dosker-engine-bochs/tests/geometry.rs`**
        - `thirty_two_mib_is_128_16_32` — 33 554 432 bytes.
        - `one_mib_is_4_16_32` — 1 048 576 bytes.
        - `rejects_a_size_that_is_not_a_multiple_of_262144` — 300 000
          bytes pins `EngineError::Setup` and the size in the message.
        - `rejects_an_empty_image` — 0 bytes pins `EngineError::Setup`.
        - `rejects_more_than_16383_cylinders` — 4 294 967 296 bytes pins
          `EngineError::Setup`.
    - **Modify `crates/dosker-engine-bochs/src/lib.rs`**
        - Declare the `geometry` module and re-export `DiskGeometry`.
    - **Verify:** `cargo test -p dosker-engine-bochs --test geometry`

- [x] **Task 4: Bochs config generation and endpoints**
    - **Implements:** FR-011; design §Component Interfaces (Engine
      Adapter), §Build And Runtime Semantics; doskerfile §ENGINE-CONFIG;
      the spike Decisions table.
    - **Create `crates/dosker-engine-bochs/src/endpoints.rs`**
        - `BochsEndpoints { pub agent_serial: Option<EngineEndpointKind> }`.
        - `for_spec` — reserves one loopback TCP port for a non-booter and
          returns `None` for a booter. The endpoint is `Tcp { host:
          Ipv4Addr::LOCALHOST, port }`.
        - `from_command` — reads the `serial_agent` endpoint from
          `EngineCommand.endpoints`.
        - `engine_endpoints` — returns the `serial_agent` endpoint.
        - `remove_stale` — returns `Ok(())`; TCP has no files. The debug
          socket is session-local and is not an `EngineEndpoint`.
    - **Create `crates/dosker-engine-bochs/src/config.rs`**
        - `pub const CONFIG_FILE_NAME: &str = "bochsrc"`, `pub const
          HEADLESS_DISPLAY: &str = "nogui"`, `pub const CPU_MODEL: &str =
          "i486dx4"`, and `pub const MEMORY_MB: u32 = 16`.
        - `render_config(spec, endpoints) -> Result<String, EngineError>`
          — rejects a `HostDir` volume with
          `EngineError::Unsupported("host directory")` and any other
          volume with `EngineError::Unsupported("volume image")`; rejects
          `spec.net == "ne2000"` with
          `EngineError::Unsupported("network address translation")` and
          never omits the NIC; accepts `spec.net == "none"`; any other value
          returns `EngineError::Setup("unknown network device {net}")`;
          rejects `DisplayMode::Window` with
          `EngineError::Unsupported("window display")` because the spike
          never opened a window, and `DisplayMode::Vnc` with
          `EngineError::Unsupported("vnc")`; computes
          `DiskGeometry::from_size` from `spec.disk_path` metadata; writes
          the fixed keys in this exact order:
          `romimage: file=$BXSHARE/BIOS-bochs-latest`,
          `vgaromimage: file=$BXSHARE/VGABIOS-lgpl-latest.bin`,
          `cpu: model=i486dx4`, `megs: 16`, `ata0-master: type=disk,
          path="<disk_path>", mode=flat, cylinders=<C>, heads=<H>,
          spt=<S>`, `boot: disk`, `display_library: nogui`; the two serial
          lines, the quoted `log: "<log_dir>/bochs.log"`,
          `panic: action=fatal`, `error: action=report`,
          `info: action=report`, and `debug: action=ignore`; then appends
          every raw `spec.engine_config["bochs"]` line verbatim, in order.
        - The serial lines are `com{control_port}: enabled=1,
          mode=socket-server, dev=localhost:<port>` for a non-booter and
          `com{control_port}: enabled=1, mode=null` for a booter. The
          other line is `com1: enabled=1, mode=null` or `com2: enabled=1,
          mode=null`. Emit `com1` before `com2` in every case.
        - `write_config(spec, endpoints) -> Result<PathBuf,
          EngineError>` — creates `spec.log_dir` and writes the rendered
          text to `spec.log_dir.join(CONFIG_FILE_NAME)`.
    - **Create `crates/dosker-engine-bochs/tests/config.rs`**
        - `renders_the_reference_boot_keys` — pins `romimage`,
          `vgaromimage`, `cpu: model=i486dx4`, `megs: 16`, and `boot:
          disk`.
        - `renders_the_ata_line_with_the_computed_geometry` — a 32 MiB
          disk pins `path="<disk>", mode=flat, cylinders=128, heads=16,
          spt=32`.
        - `renders_the_nogui_display_library` — pins `display_library:
          nogui` for `DisplayMode::None`.
        - `rejects_a_window_display` — pins
          `EngineError::Unsupported("window display")` for
          `DisplayMode::Window`.
        - `renders_both_serial_ports` — with `control_port = 2`, pins
          `com1: enabled=1, mode=null` before `com2: enabled=1,
          mode=socket-server, dev=localhost:<port>`.
        - `renders_the_agent_on_com1_when_the_control_port_is_one` — with
          `control_port = 1`, pins the agent on `com1` and `com2` as the
          guest line.
        - `renders_the_log_path_quoted` — a `log_dir` containing a space
          pins `log: "<log_dir>/bochs.log"` with the quotes.
        - `appends_engine_config_lines_in_order` — two raw `bochs` lines
          appear after `debug: action=ignore`, in order.
        - `omits_the_agent_serial_line_for_a_booter` — pins both serial
          lines as `mode=null`.
        - `rejects_host_directories` — pins
          `EngineError::Unsupported("host directory")`.
        - `rejects_named_volumes` — pins
          `EngineError::Unsupported("volume image")`.
        - `rejects_ne2000_without_the_nat_capability` — pins only
          `EngineError::Unsupported("network address translation")`, even
          with no published ports.
        - `the_nat_capability_message_names_qemu` — pins
          `dosker_engine::capability_message("bochs", Capability::Nat)` to
          `engine bochs does not provide the nat capability; the qemu
          engine provides it`.
        - `omits_the_nic_for_a_none_network` — pins the absence of any
          network key.
        - `writes_a_deterministic_config` — two renders of the same spec
          are byte-identical.
        - `writes_the_config_to_the_container_directory` — pins
          `<log_dir>/bochsrc`.
    - **Create `crates/dosker-engine-bochs/tests/endpoints.rs`**
        - `uses_a_loopback_tcp_agent_endpoint` — pins
          `EngineEndpointKind::Tcp` with `127.0.0.1`.
        - `a_booter_has_no_agent_endpoint` — pins `None`.
        - `from_command_reads_the_agent_endpoint` — pins the endpoint
          recovered from `EngineCommand.endpoints`.
        - `remove_stale_removes_nothing` — pins `Ok(())`.
    - **Modify `crates/dosker-engine-bochs/src/lib.rs`**
        - Declare the `config` and `endpoints` modules and re-export
          `CONFIG_FILE_NAME`, `HEADLESS_DISPLAY`, `CPU_MODEL`, `MEMORY_MB`,
          `render_config`, `write_config`, and `BochsEndpoints`.
    - **Verify:** `cargo test -p dosker-engine-bochs --test config && cargo test -p dosker-engine-bochs --test endpoints`

- [x] **Task 5: Internal-debugger pseudo-terminal**
    - **Implements:** FR-011; design §Component Interfaces (Engine
      Adapter), §Testing Strategy; the spike Debugger section and
      `assets/bochs-spike/debugger/pty-session.txt`.
    - **Create `crates/dosker-engine-bochs/src/pty.rs`** (`#[cfg(unix)]`)
        - `PtyPair { master: File, slave: Option<File>, slave_path:
          PathBuf }`, crate-private.
        - `PtyPair::open() -> Result<Self, EngineError>` — calls
          `libc::posix_openpt(O_RDWR | O_NOCTTY)`, `grantpt`, `unlockpt`,
          and `ptsname`; guards the `ptsname` static buffer with a
          `static PTY_LOCK: Mutex<()>` and copies the `CStr` name
          immediately; opens the slave with `O_RDWR | O_NOCTTY`; reads the
          termios with
          `tcgetattr`, applies `cfmakeraw`, and writes it back with
          `tcsetattr(TCSANOW)`; wraps the master fd in a `File`.
        - `PtyPair::slave_path() -> &Path`, `PtyPair::take_slave() ->
          Option<File>`, `PtyPair::master() -> &File`,
          `PtyPair::master_clone() -> Result<File, EngineError>`, and
          `PtyPair::write_line(&self, line: &str) -> Result<(),
          EngineError>` writing `line` and a newline to the **master**
          through `self.master()` and flushing. The pty tests read the
          private `slave` field; no `slave()` accessor exists.
        - The child gets the slave as stdin and stdout; the parent keeps
          the master and drains it. Writing to the master reaches the
          child's stdin, and what the child writes to the slave is read
          from the master. A client cannot share the child's slave side;
          the session bridges a debug client to the master instead.
        - The master is blocking at spawn. The debug bridge sets it
          non-blocking when the first client attaches; a `dup` from
          `master_clone` shares the status flags, so the reader and every
          master writer poll after that point. `write_line` is called only
          before the bridge starts, while the master is still blocking.
          The debug reader polls the master with `libc::poll` and a 20 ms
          timeout, checks its stop flag between polls, and reads only after
          a readable poll.
        - Add a `#[cfg(all(test, unix))] mod tests`:
          `pty_opens_a_pair_with_a_slave_path` — the path exists and is a
          character device; `pty_master_reads_what_the_slave_writes` — a
          write to the slave is read from the master; `pty_writes_a_line_to_the_child`
          — `write_line("c")` is read from the slave as `b"c\n"`;
          `pty_sets_the_slave_to_raw_mode` — `write_line("x")` yields exactly
          `b"x\n"` on the slave, with no carriage return and no echo.
    - **Modify `crates/dosker-engine-bochs/src/lib.rs`**
        - Declare the Unix-only `pty` module.
    - **Verify:** `cargo test -p dosker-engine-bochs pty_`

- [x] **Task 6: Bochs session: serial, exec, stop, and the pty debugger**
    - **Implements:** FR-011; design §Component Interfaces (Engine
      Adapter), §Build And Runtime Semantics, §Error Handling; the spike
      Decisions table and `assets/bochs-spike/debugger/pty-session.txt`.
    - **Create `crates/dosker-engine-bochs/src/stream.rs`**
        - Crate-private `BochsStream(TcpStream)` implementing `Read`,
          `Write`, and `dosker_agent_proto::Transport`.
        - `connect(kind, deadline)` — a TCP attempt retries every 20
          milliseconds until the deadline; a `UnixSocket` endpoint returns
          `EngineError::Setup`.
    - **Create `crates/dosker-engine-bochs/src/serial_log.rs`**
        - Crate-private `SerialLog<T: Transport>` that appends every
          transferred byte to `serial.log`; a log failure must not fail a
          protocol read or write. Mirror the DOSBox-X type.
    - **Create `crates/dosker-engine-bochs/src/session.rs`**
        - `BochsSession { child, spec, command, endpoints, agent,
          #[cfg(unix)] debug: Option<DebugSession>, exit_code,
          #[cfg(windows)] job }`.
        - Crate-private constants: `BOOTER_ALIVE_PROBE =
          Duration::from_millis(250)`, `READY_CONNECT_POLL =
          Duration::from_millis(20)`, `STOP_POLL =
          Duration::from_millis(20)`, `DEBUG_BRIDGE_POLL =
          Duration::from_millis(20)`, `DEBUG_BRIDGE_WRITE =
          Duration::from_secs(1)`, `DEBUG_BRIDGE_BUFFER: usize = 8192`, and
          `DEBUG_PENDING_LIMIT: usize = 65_536`.
        - `#[cfg(unix)] struct DebugSession { pty: PtyPair, channel:
          Arc<DebugChannel>, reader: DebugReader, bridge:
          Option<DebugBridge>, socket: PathBuf }`.
        - `#[cfg(unix)] struct DebugChannel { state:
          Mutex<DebugChannelState> }` and `DebugChannelState { client:
          Option<UnixStream>, pending: Vec<u8> }`, with `publish(&self,
          bytes: &[u8])` — writes to the connected client and drops it on
          error, otherwise appends to `pending` and keeps only the last
          `DEBUG_PENDING_LIMIT` bytes; `attach(&self, client: UnixStream)`
          — writes the pending bytes, clears them, and stores the client;
          and `detach(&self)` — drops the client. One mutex covers both
          fields, so a client attach and a reader publish cannot
          interleave.
        - `#[cfg(unix)] struct DebugReader { stop: Arc<AtomicBool>, handle:
          Option<JoinHandle<()>> }` with `start(master: File, log: File,
          channel: Arc<DebugChannel>) -> Self` spawning a thread that polls
          the blocking master with `libc::poll` and a `DEBUG_BRIDGE_POLL`
          timeout, checks `stop` between polls, and after a readable poll
          reads `DEBUG_BRIDGE_BUFFER`-sized chunks, appends each chunk to
          the log, and calls `channel.publish`; `WouldBlock` and
          `Interrupted` sleep `DEBUG_BRIDGE_POLL`; EOF or any other error
          ends the thread; `Drop` sets `stop` and joins.
        - `#[cfg(unix)] struct DebugBridge { stop: Arc<AtomicBool>, handle:
          Option<JoinHandle<()>> }` with `start(socket: &Path, channel:
          Arc<DebugChannel>, master: File) -> Result<Self, EngineError>` —
          calls `dosker_platform::fs::ensure_dir_mode` on the socket parent
          with `0o700`, removes a stale socket file ignoring `NotFound`,
          binds a `UnixListener`, sets it non-blocking and the socket file
          to `0o600`, and spawns a thread that accepts one client at a time
          (non-blocking accept polled every `DEBUG_BRIDGE_POLL`), calls
          `channel.attach` with a `try_clone` of the client, sets the
          client read timeout to `DEBUG_BRIDGE_POLL`, forwards each read
          chunk to the master with `write_all` and `flush`, and calls
          `channel.detach` when the client ends; `Drop` stops and joins the
          thread and removes the socket file, ignoring `NotFound`.
        - `BochsSession::spawn` — creates `spec.log_dir`; removes
          `<disk_path>.lock` when it exists, ignoring `NotFound`; reads the
          endpoints from the command; opens `engine.log` for append. On
          Unix, when the command args contain `-dbg`, it opens a `PtyPair`,
          stores `engine_socket_path(&spec.root, &spec.container_id,
          "debug")`, starts a `DebugReader` on a master clone with a log
          clone and the channel, and gives the child the slave as stdin and
          stdout through two `Stdio` values built from `take_slave` and
          `try_clone`, with stderr on the log. Without `-dbg`, stdin is
          null and stdout and stderr go to the log. It spawns through
          `ProcessSupervisor::prepare_engine`, assigns the Windows
          `JobObject::kill_on_close`, and after a successful spawn writes
          `c` to the master and `debug pty <slave_path>` to `engine.log`
          when a pty exists.
        - `EngineSession::pid`, `process_group`, `ready`, `exec`, `stop`,
          `kill`, `try_exit`, `command`, `endpoints`, `put_file`,
          `get_file`, and `debug_endpoint`.
        - `ready` connects to `endpoints.agent_serial`, wraps the stream
          in `SerialLog`, runs `AgentClient::handshake` with the remaining
          deadline, and returns `Exited` or `ReadyTimeout` on failure. For
          a booter, it sleeps `BOOTER_ALIVE_PROBE` and returns `Exited`
          when the process has exited, otherwise `Ok(())`.
        - `exec` maps `ExecRequest` to the agent protocol and `ProtoError`
          to `EngineError`: `Timeout` becomes `ExecTimeout(timeout)`,
          `GuestRebooted` becomes `GuestRebooted`, `Agent` becomes `Agent`,
          `Io` becomes `Io`, and every other error becomes `Protocol`.
        - `stop` sends the agent `SHUTDOWN` when an agent is present, sends
          `SIGTERM` to the engine process group on Unix and kills the child
          on Windows, waits for the process until the grace deadline, and
          returns `StopTimeout` after a kill. A
          `terminate_process_group(pid)` helper sends
          `libc::kill(-pid, SIGTERM)` and ignores `ESRCH`.
        - `status_code` on Unix returns `status.code()` or `128 +
          status.signal()`. `kill` therefore returns 137 and a `SIGTERM`
          stop returns 143. On Windows it returns `status.code()` or -1.
        - `debug_endpoint(DebugKind::Native)` returns
          `EngineError::Unsupported("debugger")` when there is no debug
          session or when the process has already exited; otherwise it
          starts the `DebugBridge` on the first call with the stored
          socket path, the channel, and a master clone, sends `SIGINT` to
          the engine process group through an `interrupt_process_group(pid)`
          helper (`libc::kill(-pid, SIGINT)`, ignoring `ESRCH`), and
          returns the stored socket path as a string. `DebugKind::Gdb`
          returns `EngineError::Unsupported("debugger")`.
        - `endpoints` returns the serial endpoint only.
        - A `Drop` that kills a live child, reaps it, stops the bridge and
          the reader, removes the debug socket, and drops the Windows job
          after the child is reaped.
        - Add a `#[cfg(all(test, unix))] mod tests` with a fake TCP agent
          copied from the DOSBox-X session tests, plus:
          `ready_performs_the_agent_handshake`,
          `exec_returns_the_agent_output_and_status`,
          `put_and_get_round_trip_a_file`,
          `stop_sends_shutdown_then_sigterm_and_records_143`,
          `stop_times_out_and_kills_a_guest_that_ignores_sigterm`,
          `kill_ends_the_process`,
          `status_code_maps_a_signal_to_128_plus_the_signal`,
          `ready_reports_an_early_exit`, `ready_times_out_without_an_agent`,
          `booter_ready_accepts_a_live_process`,
          `booter_ready_reports_an_exited_process`,
          `booter_exec_returns_no_agent`,
          `booter_transfer_returns_no_agent`,
          `spawn_writes_the_continue_command_to_the_debugger`,
          `spawn_removes_a_stale_image_lock`,
          `spawn_drains_the_debugger_output_to_the_engine_log`,
          `debug_endpoint_returns_the_debug_socket_and_signals_the_engine`,
          `debug_bridge_forwards_a_client_to_the_debugger`,
          `debug_bridge_replays_output_buffered_before_the_client_connects`,
          `debug_socket_is_removed_when_the_session_drops`,
          `debug_endpoint_rejects_native_without_a_pty`, and
          `debug_endpoint_rejects_gdb_without_a_gdbstub`.
        - `spawn_writes_the_continue_command_to_the_debugger` uses a shell
          script that reads one line from stdin, writes it to a marker
          file, and sleeps; a command with `-dbg` makes the session create
          the pty; the test waits for the marker and pins `c`.
        - `spawn_drains_the_debugger_output_to_the_engine_log` uses a
          script that prints `debugger ready` to stdout and sleeps; the
          test waits for `engine.log` to contain `debugger ready`.
        - `debug_endpoint_returns_the_debug_socket_and_signals_the_engine`
          uses a script with `trap 'touch <marker>' INT`; the test asserts
          the returned path equals
          `engine_socket_path(&spec.root, &spec.container_id, "debug")` and
          the marker appears within five seconds.
        - `debug_bridge_forwards_a_client_to_the_debugger` uses a script
          that reads one line and appends it to a marker; a `UnixStream`
          client writes `r\n`; the marker contains `r`.
        - `debug_bridge_replays_output_buffered_before_the_client_connects`
          uses a script that prints `debugger ready` and sleeps; after the
          reader has drained it, a client connects and reads the buffered
          text within two seconds.
        - `debug_socket_is_removed_when_the_session_drops` asserts the
          socket path no longer exists after the session is dropped.
    - **Modify `crates/dosker-engine-bochs/src/lib.rs`**
        - Declare the `serial_log`, `session`, and `stream` modules and
          re-export `BochsSession`.
    - **Modify `crates/dosker-platform/src/endpoint.rs`**
        - Accept `debug` in the `engine_socket_path` name list and doc
          comment, beside `agent`, `qmp`, `monitor`, and `gdb`.
    - **Modify `crates/dosker-platform/tests/engine_socket.rs`**
        - Add `"debug"` to the names in `engine_socket_names_are_unique`.
    - **Verify:** `cargo test -p dosker-engine-bochs && cargo test -p dosker-platform --test engine_socket`

- [x] **Task 7: Bochs adapter and command line**
    - **Implements:** FR-011; design §Component Interfaces (Engine
      Adapter), §Build And Runtime Semantics, §Error Handling; the spike
      Decisions table.
    - **Modify `crates/dosker-engine-bochs/src/adapter.rs`**
        - `BochsAdapter { binary, version, processes }`, `new`, and
          `discover`.
        - `EngineAdapter::name`, `version`, `capabilities`, `probe`,
          `build_command`, `start`, and `start_with_command`.
        - `build_command` builds `BochsEndpoints::for_spec`, rejects
          `DisplayMode::Vnc` with `EngineError::Unsupported("vnc")` and
          `DisplayMode::Window` with
          `EngineError::Unsupported("window display")`, calls
          `write_config`, returns `-q -f <path>`, and appends `-dbg` only
          when `cfg!(unix)` and `self.binary.has_internal_debugger()`.
        - `probe` checks that the binary path is a file.
    - **Create `crates/dosker-engine-bochs/tests/adapter.rs`**
        - `build_command_writes_the_config_and_returns_the_fixed_arguments`
          — a `BochsBinary` with `Debugger::Internal` pins `["-q", "-f",
          "<log_dir>/bochsrc"]` plus `-dbg` on Unix.
        - `build_command_omits_the_debug_flag_without_an_internal_debugger`
          — a `BochsBinary` with `Debugger::None` pins no `-dbg`.
        - `build_command_rejects_vnc` — pins
          `EngineError::Unsupported("vnc")`.
        - `build_command_rejects_a_window_display` — pins
          `EngineError::Unsupported("window display")`.
        - `probe_rejects_a_missing_binary` — pins `EngineError::NotFound`.
        - `start_spawns_the_process_with_the_generated_command` — a
          `BochsBinary` with `Debugger::Internal` and a fake binary that
          writes its arguments to a file and prints `bochs started`; pins
          the child pid, the written `bochsrc`, and `engine.log` containing
          the drained `bochs started` output. The rendered `bochsrc` names
          `<log_dir>/bochs.log` for the `log:` key.
        - `adapter_reports_the_discovered_version` — pins `3.1`.
    - **Modify `crates/dosker-engine-bochs/src/lib.rs`**
        - Re-export `BochsAdapter`.
    - **Verify:** `cargo test -p dosker-engine-bochs --test adapter`

- [x] **Task 8: CLI engine registry, discovery, and reporting**
    - **Implements:** FR-011; design §Component Interfaces (CLI),
      §Error Handling.
    - **Modify `crates/dosker-cli/Cargo.toml`**
        - Add `dosker-engine-bochs.workspace = true`.
    - **Modify `crates/dosker-cli/src/adapter.rs`**
        - `discover` adds `"bochs"` returning
          `BochsBinary::discover()`.
        - `build` adds `"bochs"` returning `BochsAdapter::discover`.
        - `default_engine` stays `qemu`. Do not add a local provider
          table.
    - **Modify `crates/dosker-cli/tests/cli_engines.rs`**
        - Add `BOCHS_VERSION_LINE = "Bochs x86 Emulator 3.1"` and
          `BOCHS_CAPABILITIES = "boot,serial,stop,headless,debug_native"`.
        - Add `write_bochs_script` and `write_bochs`; the fake binary
          prints the banner and `Bochs internal debugger, type 'help' for
          help or 'c' to continue`.
        - Change `run_with_engines` to take the qemu, dosbox-x, and bochs
          paths and set `DOSKER_QEMU`, `DOSKER_DOSBOX_X`, and
          `DOSKER_BOCHS`.
        - `engines_reports_three_engines` — replaces
          `engines_reports_both_engines`; pins the three lines.
        - `engines_lists_the_bochs_proven_capabilities` — pins the Bochs
          capability field to `BOCHS_CAPABILITIES`.
        - `engines_exits_one_when_no_engine_is_found` — sets all three to
          missing paths and pins the three `not found` lines.
        - `engines_exits_zero_when_only_one_engine_is_found` — sets the
          missing qemu and bochs paths and the found dosbox-x path.
        - `engines_reports_an_unavailable_engine` — sets a broken qemu and
          found dosbox-x and bochs paths.
        - `doctor_reports_the_data_root_and_three_engines` — replaces
          `doctor_reports_the_data_root_and_both_engines`; pins the three
          engine lines.
        - `doctor_lists_each_engine_with_its_capabilities` — pins
          `{name}: {version} ({path}) [{capabilities}]` for all three.
        - `doctor_exits_one_when_no_engine_is_found` — sets all three to
          missing paths.
        - `doctor_exits_zero_when_only_one_engine_is_found`,
          `doctor_reports_unavailable_when_an_engine_fails`,
          `doctor_reports_unavailable_when_an_engine_version_is_unparsable`,
          `doctor_reports_an_unwritable_root`,
          `doctor_prints_the_security_statement`, and
          `doctor_prints_the_development_mode_warning` — name all three
          binaries explicitly.
    - **Verify:** `cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_engines`

- [x] **Task 9: Create-time gates for Bochs**
    - **Implements:** FR-011, FR-016; design §Error Handling; doskerfile
      §REQUIRES.
    - **Modify `crates/dosker-cli/src/commands/create.rs`**
        - Replace `if engine == "dosbox-x" && (!specs.is_empty() ||
          !image.volumes.is_empty())` with a check for
          `matches!(engine.as_str(), "dosbox-x" | "bochs")` and the message
          `named volumes are not supported by the {engine} engine; use
          qemu`.
        - The existing `nat`, `vnc`, and `host_dir_live` capability checks
          already cover Bochs through the provider table.
    - **Modify `crates/dosker-cli/tests/cli_create.rs`**
        - Remove `create_rejects_bochs_with_an_unavailable_message`.
        - `create_records_the_bochs_engine` — a network-disabled image
          pinned to `bochs` creates a `bochs` record without `--engine`.
        - `create_rejects_a_ne2000_image_on_bochs` — the only Bochs test
          that keeps the imported image's default `ne2000` network; a
          default image with `--engine bochs` exits 2 with `engine bochs
          does not provide the nat capability; the qemu engine provides
          it`.
        - `create_rejects_vnc_on_bochs` — a network-disabled image with
          `--display vnc` exits 2 with `engine bochs does not provide the
          vnc capability; the qemu engine provides it`.
        - `create_rejects_named_volumes_on_bochs` — a network-disabled
          image with a `-v` mount and an image `VOLUME` each exit 2 with
          `named volumes are not supported by the bochs engine; use qemu`.
        - `create_rejects_a_bind_on_bochs` — a network-disabled image with
          `--bind D:<host>` exits 2 with `engine bochs does not provide
          the host_dir_live capability; the qemu engine provides it`.
        - `create_records_the_bochs_engine`,
          `create_rejects_vnc_on_bochs`,
          `create_rejects_named_volumes_on_bochs`, and
          `create_rejects_a_bind_on_bochs` call
          `disable_image_network(&root, &reference)` after the import;
          otherwise the imported fixture's `ne2000` default trips the NAT
          gate before the assertion under test.
    - **Modify `crates/dosker-cli/tests/cli_requires.rs`**
        - `create_rejects_a_required_capability_the_bochs_engine_lacks` —
          an image pinned to `bochs` with `REQUIRES gdbstub` exits 2 with
          `engine bochs does not provide the gdbstub capability; the qemu
          engine provides it`.
        - `create_accepts_a_required_capability_the_bochs_engine_declares`
          — `REQUIRES headless` on a network-disabled image exits 0.
    - **Modify `crates/dosker-cli/tests/cli_debug.rs`**
        - `debug_rejects_gdb_when_the_bochs_engine_has_no_gdbstub` — a
          running `bochs` record with `--gdb` exits 1 with `engine bochs
          does not provide the gdbstub capability; the qemu engine
          provides it`.
        - Keep `debug_reports_unsupported_for_the_mock_engine`; its
          provider list now includes `bochs`.
    - **Verify:** `cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_create && cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_requires && cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_debug && cargo test -p dosker-cli --features qemu-engine,mock-engine --lib`

- [x] **Task 10: Document the Bochs capability set and engine behavior**
    - **Implements:** FR-011, FR-016; design §Engine Capability Matrix,
      §Error Handling, §Testing Strategy.
    - **Modify `docs/engines.md`**
        - The lead sentence — replace "and the DOSBox-X runtime engine" with
          "and the QEMU, DOSBox-X, and Bochs runtime engines".
        - `## Engine selection` — qemu, dosbox-x, and bochs have adapters.
          The build path stays QEMU-only; a build whose final engine is
          `dosbox-x` or `bochs` fails with `engine dosbox-x: unsupported
          engine` or `engine bochs: unsupported engine` before the step
          loop.
        - `## ENGINE-CONFIG` — add that Bochs appends raw lines verbatim
          after the fixed `bochsrc` keys, and that a later key overrides an
          earlier one. Add an `ENGINE-CONFIG bochs` example.
        - `## REQUIRES` — keep the four message forms; state that an image
          with `MACHINE net=ne2000` is rejected on Bochs because `nat` is
          not provided.
        - `## Capability table` — add the Bochs column: `yes` for `boot`,
          `serial`, `stop`, `headless`, and `debug_native`; `no` for
          `gdbstub`; `unproven` for `host_dir_live`, `screenshot`, `vnc`,
          and `nat`; `no` for `audio`. State that Bochs does not declare
          `host_dir_live` or `nat`, so `--bind` and `-p` are rejected, and
          that named volumes are rejected.
        - `## Machine` — add the Bochs keys `cpu: model=i486dx4`,
          `megs: 16`, the IDE disk, VGA, and two serial ports, and that
          `MACHINE` overrides are not applied.
        - `## Display` — Bochs runs with `display_library: nogui`. It does
          not support a window display in this release; `--display window`
          fails with `unsupported engine feature: window display`.
        - `## Debugger` — Bochs starts with `-dbg` when the build has the
          internal debugger; the guest boots after the adapter sends `c`;
          `dosker debug` sends `SIGINT` to stop the guest and prints
          `native debugger <socket>`; attach a terminal to the Unix socket
          path; `r` prints registers, `c` continues, `quit` exits. A build
          without the internal debugger starts without `-dbg` and the
          endpoint is unavailable. The optional gdbstub build is not
          declared as a capability.
        - `## Discovery` — `DOSKER_BOCHS`, `DOSKER_BOCHS_VERSION`,
          `DOSKER_BOCHS_SHA256`, the default `bochs`, the reference build
          `3.1`, and the checksum
          `8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c`.
          State that the version comes from the startup banner, not
          `--version`.
        - `## Integration tests` — the `bochs_runtime` command with
          `DOSKER_INTEGRATION=1`, `DOSKER_BOCHS`,
          `DOSKER_BOCHS_VERSION=3.1`, `DOSKER_BOCHS_SHA256`, and
          `DOSKER_BOCHS_IMAGE=agent/fixtures/dosker-agent.img`, plus the
          optional `gdbstub_build` command with `DOSKER_BOCHS_GDBSTUB`.
    - **Modify `docs/builds.md`**
        - Replace the "Engine support" paragraph: `qemu` is the only
          engine for in-DOS `RUN` and the build path. dosbox-x and bochs
          are available for `run` and `create` with a prepared image. A
          build whose final engine is `dosbox-x` or `bochs` fails with
          `engine dosbox-x: unsupported engine` or `engine bochs:
          unsupported engine` before the step loop. The final engine must
          satisfy every `REQUIRES`; the check runs before the step loop.
    - **Modify `docs/volumes-and-network.md`**
        - `--bind` needs an engine that declares `host_dir_live`. QEMU
          declares it. DOSBox-X and Bochs do not declare it in this
          release; the capability gate rejects `--bind` on both.
        - NAT needs an engine that declares `nat`. QEMU declares it.
          DOSBox-X and Bochs do not declare it in this release; the
          capability gate rejects `-p` on both. An image with `MACHINE
          net=ne2000` is rejected on both when `nat` is not provided.
        - Keep the `doctor` warning quote unchanged.
    - **Modify `README.md`**
        - The `### Engines` section: `qemu`, `dosbox-x`, and `bochs` have
          runtime adapters. Bochs is a runtime engine for the capabilities
          listed in the capability table.
    - **Verify:** `test -f docs/engines.md && rg -Fq "and the QEMU, DOSBox-X, and Bochs runtime engines" docs/engines.md && rg -Fq "qemu, dosbox-x, and bochs have adapters" docs/engines.md && rg -Fq "## Capability table" docs/engines.md && rg -Fq "| \`gdbstub\` | yes | no | no |" docs/engines.md && rg -Fq "DOSKER_BOCHS_SHA256" docs/engines.md && rg -Fq "8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c" docs/engines.md && rg -Fq "native debugger <socket>" docs/engines.md && rg -Fq "SIGINT" docs/engines.md && rg -Fq "does not support a window display" docs/engines.md && rg -Fq "bochs are available for \`run\` and \`create\`" docs/builds.md && rg -Fq "engine bochs: unsupported engine" docs/builds.md && rg -Fq "DOSBox-X and Bochs do not declare it in this release" docs/volumes-and-network.md && rg -Fq "docs/engines.md" README.md && ! rg -Fq "\`bochs\` is a known engine name" docs/engines.md && ! rg -Fq "\`bochs\` is not implemented" docs/builds.md`

- [x] **Task 11: Real Bochs runtime integration tests**
    - **Implements:** FR-011; design §Testing Strategy, SC-005, SC-006;
      roadmap `Done when`.
    - **Modify `crates/dosker-test-support/src/fixtures.rs`**
        - `BOCHS_IMAGE_ENV = "DOSKER_BOCHS_IMAGE"` and `bochs_image()` —
          resolves the variable against the workspace root and requires a
          regular file.
        - `BOCHS_GDBSTUB_ENV = "DOSKER_BOCHS_GDBSTUB"` and
          `bochs_gdbstub()` — resolves the optional binary path.
    - **Modify `crates/dosker-cli/tests/common/mod.rs`**
        - `command_with_bochs` — sets `DOSKER_BOCHS` from the environment
          or `bochs` and `DOSKER_AGENT_EXE`.
        - `run_with_bochs` — a 180-second timeout.
        - `import_image_for_bochs` — imports the disk image with the real
          agent at `agent/build/DOSKER.EXE` and rewrites `machine.net =
          "none"` before committing it.
    - **Create `crates/dosker-cli/tests/bochs_runtime.rs`**
        - Put `#![cfg(unix)]` first, then `mod common;`.
        - Add a module-level `INTEGRATION_LOCK` to serialize the tests.
        - `bochs_boots_the_prepared_image_and_answers_exec` — gates on
          `DOSKER_INTEGRATION=1` and `DOSKER_BOCHS_IMAGE`; imports with
          `import_image_for_bochs`, runs `run -d --engine bochs
          --boot-timeout 120`, runs `exec <id> ECHO HELLO`, asserts
          `HELLO` and status 0, and runs `stop`.
        - `bochs_stops_with_the_recorded_exit_code` — starts a container
          with the prepared image, runs `stop`, and asserts the record has
          `status == Stopped` and `exit_code == Some(143)`.
        - `bochs_reports_its_capabilities` — gates on
          `DOSKER_INTEGRATION=1` only; pins the `dosker engines` Bochs
          line to version `3.1` and `boot,serial,stop,headless,
          debug_native`.
        - `bochs_exposes_a_working_internal_debugger_endpoint` — imports
          the image, starts a container, runs plain `debug <id>`, parses
          the `native debugger <path>` line, connects a `UnixStream` to
          the socket path, sets a two-second read timeout, writes `r\n`,
          reads until `rax:` within ten seconds, writes `c\n`, and stops
          the container.
        - `bochs_rejects_a_required_capability_it_does_not_declare` —
          gates on `DOSKER_INTEGRATION=1` only; generates a disk image
          with `make_disk_image`, imports it with `REQUIRES gdbstub`, and
          asserts the create failure names `qemu`.
        - The debugger test reads the socket with
          `std::os::unix::net::UnixStream` and a read timeout; it does not
          open the pty.
    - **Verify:** `DOSKER_INTEGRATION=1 DOSKER_BOCHS=/opt/homebrew/bin/bochs DOSKER_BOCHS_VERSION=3.1 DOSKER_BOCHS_SHA256=8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c DOSKER_BOCHS_IMAGE=agent/fixtures/dosker-agent.img cargo test -p dosker-cli --features qemu-engine --test bochs_runtime -- --nocapture`

- [x] **Task 12: Optional gdbstub build test**
    - **Implements:** FR-011; design §Testing Strategy; the spike gdbstub
      row.
    - **Modify `crates/dosker-engine-bochs/Cargo.toml`**
        - Add `dosker-test-support.workspace = true` under
          `[dev-dependencies]`.
    - **Create `crates/dosker-engine-bochs/tests/gdbstub_build.rs`**
        - Put `#![cfg(unix)]` first.
        - Gate on `dosker_test_support::integration_enabled()`,
          `bochs_gdbstub()`, and `bochs_image()`; a missing gate returns
          early.
        - `gdbstub_build_has_no_internal_debugger` —
          `BochsBinary::from_path` on `bochs_gdbstub()` pins
          `Debugger::None` and version `3.1`.
        - `gdbstub_build_accepts_a_gdb_connection` — copies
          `bochs_image()` to a temporary directory; binds a loopback
          listener on port 0, reads the port, and drops it; builds an
          `EngineSpec` with `booter: true`, the copied disk, a temporary
          `log_dir`, and `engine_config["bochs"] = ["gdbstub: enabled=1,
          port=<port>, text_base=0, data_base=0, bss_base=0"]`; builds the
          adapter from the gdbstub binary; runs `build_command` and
          `start_with_command`; connects to `127.0.0.1:<port>` with a
          two-second read timeout; sends `$qSupported#37`; asserts the
          reply starts with `+` or contains `$`; kills the session.
    - **Verify:** `DOSKER_INTEGRATION=1 DOSKER_BOCHS_GDBSTUB=$HOME/bochs-gdbstub/bin/bochs DOSKER_BOCHS_IMAGE=agent/fixtures/dosker-agent.img cargo test -p dosker-engine-bochs --test gdbstub_build -- --nocapture`

## Deviations

Each ruling below is a plan-local correction made during execution. No
public interface in `Interfaces` changed.

- **The pty master is blocking at spawn and non-blocking once the bridge
  starts.** A `dup` from `master_clone` shares the open file description, so
  the bridge's `O_NONBLOCK` also applies to the reader and to
  `PtyPair::master()`. The reader and every master writer poll after that
  point; `write_line` is called only at spawn, before the bridge. Reason: the
  review found that the earlier "stays blocking" wording hid the shared-flag
  hazard; the code was already polling on both sides.
- **The pty accessors are the ones the session uses.** `PtyPair::slave()` was
  removed (the tests read the private field); `write_line` uses
  `PtyPair::master()`; the session logs `debug pty <slave_path>` so
  `slave_path()` is consumed. Reason: the unused accessors would fail the
  workspace `-D warnings` build.
- **A control port outside COM1/COM2 is rejected.** `render_config` returns
  `EngineError::Setup` when `spec.control_port` is not 1 or 2. Reason: the
  machine profile accepts 1..=4, and the Bochs serial renderer only has two
  ports; a 3 or 4 would boot with no agent line and time out.
- **The version probe drains both pipes.** The probe reads stdout and stderr
  concurrently while polling, so a verbose build cannot fill a pipe and block
  before it reads `quit`; a verbose-output test pins it. Reason: the reference
  build fits the pipe, but a build with more plugins would produce a false
  timeout.
- **The debug endpoint breaks the guest on every call.** `debug_endpoint`
  sends `SIGINT` each time, not only when the bridge starts, so a second
  `dosker debug` breaks a guest that was continued with `c`. Reason: repeated
  debug sessions must re-enter the debugger.
- **The pending-restore test forces a write failure.** The test shuts down the
  client's write side before attaching, because a write to a Unix socket with
  a dropped peer can still succeed on macOS. Reason: the original test was
  platform-flaky.
- **The capability-report test does not need the agent.** It builds `engines`
  from `base_command()` and sets `DOSKER_BOCHS`, like the DOSBox-X
  counterpart, instead of `command_with_bochs`, which eagerly resolves
  `agent/build/DOSKER.EXE`.
- **The docs state the reference build's capability set.** The declared set is
  static; a Bochs build without the internal debugger still declares
  `debug_native` in the table but has no endpoint. The docs also name the
  gdbstub gates and checksum separately, say `SIGINT` breaks the guest, give
  `nc -U` for the socket, and note the suites are Unix-only and need the built
  agent.
- **The workspace is expected red from Task 1 until Task 9.** The provider
  table gains Bochs in Task 1 and the CLI dispatch lands in Task 8/9; only the
  per-task verify is expected to pass in that window.
