# DOSBox-X Adapter Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-19
- **Status**: Approved
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Roadmap**: `.sdd/specs/2026-11-01-dosker/roadmap.md` — slice: `dosbox-x-adapter`
- **Depends on**: `qemu-runtime-slice`, `volumes-and-nat`, `development-mode`
- **Manual prerequisites**: the DOSBox-X `BOOT`, headless, and debugger
  spike. Official DOSBox-X builds. Before declaring NAT on DOSBox-X, the
  NAT spike. The spike record at
  `.sdd/specs/2026-11-01-dosker/assets/dosbox-x-spike.md` must have a
  completed Results table whose `boot`, `serial`, and `headless` rows are
  `proven`, and a completed Decisions table. A prepared DOSBox-X test
  image for the integration suite, supplied through
  `DOSKER_DOSBOX_X_IMAGE`. The boot, host-directory, and
  native-debugger tests need an image with `MACHINE net=none` when `nat`
  is not provided; the test helper rewrites the imported config to
  `net=none`. The reference build is DOSBox-X `2026.10.01` SDL2.
- **Goal**: Deliver the DOSBox-X adapter behind the capability model:
  discovery, config generation, the engine adapter and session, runtime
  engine selection, the `ENGINE`/`ENGINE-CONFIG`/`REQUIRES` path,
  `engines`/`doctor`/`debug` reporting, and the spike-gated capability
  set.

## Constraints

- The engine names are `qemu`, `dosbox-x`, and `bochs`.
- The capability names are `boot`, `serial`, `stop`, `headless`,
  `host_dir_live`, `debug_native`, `gdbstub`, `screenshot`, `vnc`, `nat`,
  and `audio`.
- A capability is declared only when its spike row says `proven`. An
  unproven row stays undeclared, and the adapter rejects a `REQUIRES`
  for it.
- `GdbStub`, `Screenshot`, and `Vnc` are never declared in this slice.
- If the spike marks `boot` unproven, stop the slice. If the spike marks
  `serial` or `headless` unproven, the slice cannot meet its acceptance
  bar; stop and report it.
- If the spike marks `host_dir_live` or `nat` unproven, the adapter
  rejects that feature through the capability gate and the docs record
  why.
- The capability set is exactly the rows the completed spike record
  marks `proven`, in `Capability::ALL` declaration order.
- `crates/dosker-engine/src/provider.rs` is the single source of truth
  for implemented engine names and capability sets. The QEMU and
  DOSBox-X `capabilities()` functions and the CLI read it.
- `--engine` on the CLI overrides the `ENGINE` pin for one command.
- The image config records the engine pin.
- `ENGINE-CONFIG` raw text is passed to the named adapter. QEMU splits it
  into command-line arguments with shell-like quoting. DOSBox-X and Bochs
  treat it as a configuration line.
- A QEMU `ENGINE-CONFIG` line is split into arguments with shell-like
  quoting: whitespace separates tokens; single and double quotes group; a
  backslash escapes the next character outside single quotes; an
  unterminated quote is `EngineError::Setup`.
- QEMU `ENGINE-CONFIG` arguments are appended to the QEMU argument
  vector in instruction order.
- `ENGINE-CONFIG` does not change the declared capability set. A file
  that needs a feature must still use `REQUIRES`.
- Repeated `ENGINE-CONFIG` lines are appended in order.
- `REQUIRES` is checked after engine selection and before any VM starts.
- The final selected engine must satisfy every `REQUIRES` in the
  Doskerfile. A requirement is not checked against the engine in effect
  at its own step.
- The `check_requires` helper takes the engine's declared
  `CapabilitySet` as an argument. The provider table is used only to
  format the message, so a test mock's declared set is honored.
- Build-time `REQUIRES` is checked after the final engine selection and
  before the step loop. The error is `BuildError::Capability { message }`.
- A missing capability is an error that names the missing capability and
  the engines that provide it.
- The missing-capability message is `engine {engine} does not provide the
  {capability} capability; the {provider} engine provides it` for one
  provider.
- The two-provider form is `engine {engine} does not provide the
  {capability} capability; the {first} and {second} engines provide it`.
- The zero-provider form is `engine {engine} does not provide the
  {capability} capability; no engine provides it`.
- The three-or-more-provider form is `engine {engine} does not provide
  the {capability} capability; the {first}, {second}, and {third} engines
  provide it`.
- The provider table names only engines that have an adapter: QEMU and
  DOSBox-X in this slice. Bochs joins when its adapter lands. The test
  mock is not a provider.
- A known engine name without an adapter prints `engine {engine} is not
  available; supported engines are {list}`. An arbitrary string prints
  `unknown engine: {engine}`.
- The supported-engine list is `qemu and dosbox-x` in this slice.
- A `DebugKind::Native` request uses the native debugger. A
  `DebugKind::Gdb` request uses the gdbstub. QEMU maps both kinds to its
  gdb endpoint.
- The default display is `none`.
- `start` waits up to 60 seconds for the agent `HELLO`.
- The default machine is a 486DX2 with 16 MB of RAM, VGA, SB16, NE2000,
  two serial ports, and an IDE disk.
- The DOSBox-X adapter maps the design's default machine to
  `machine = svga_s3`, `memsize = 16`, and `cputype = 486`. `MACHINE`
  overrides are not applied on DOSBox-X in this slice.
- The control agent owns COM2. COM1 stays free for the guest.
- The agent owns `serial{spec.control_port}`. When `control_port == 1`,
  the agent line is `serial1` and the guest line is omitted. Otherwise
  `serial1` is the guest line and `serial{spec.control_port}` is the
  agent line.
- The installed reference build reports `DOSBox-X version 2026.10.01
  SDL2`.
- `DOSKER_DOSBOX_X` overrides discovery, `DOSKER_DOSBOX_X_VERSION` pins
  the exact version, and `DOSKER_DOSBOX_X_SHA256` pins the checksum.
- The default program name is `dosbox-x`.
- The working boot command is the form recorded in the spike.
- The working serial line is the form recorded in the spike.
- The serial and debug endpoint kinds are the kinds recorded in the
  spike. TCP (`sock:0`) is the candidate.
- The working headless flags are the form recorded in the spike.
- The stop mechanism and exit code are the form recorded in the spike.
- The host-directory mechanism is the form recorded in the spike.
- The NAT backend and port-forward key are the form recorded in the
  spike.
- The DOSBox-X config file is `dosbox-x.conf` in the container directory.
- The DOSBox-X config pins `[dosbox] machine = svga_s3`, `[dosbox]
  memsize = 16`, `[cpu] cputype = 486`, `[sblaster] sbtype = sb16`, and
  `[ide, primary] enable = true`.
- The DOSBox-X config writes `serial1` as the guest serial line recorded
  in the spike (the reference default is `dummy`) and
  `serial{spec.control_port}` as the agent serial line. The default
  control port is 2.
- The DOSBox-X config writes `[ne2000]` and `[ethernet, slirp]` only when
  `spec.net == "ne2000"` and `Nat` is proven. It omits them when
  `spec.net == "none"`.
- A DOSBox-X spec with `spec.net == "ne2000"` and an unproven `Nat`
  is rejected with `EngineError::Unsupported("network address
  translation")` and the provider message. Do not silently omit the NIC.
- Each raw `ENGINE-CONFIG dosbox-x` line is appended verbatim to the
  generated config before `[autoexec]`. A line may be a `[section]`
  header or a `key=value` pair.
- The build-time `ENGINE-CONFIG` for the selected engine is carried in
  `InDosRunRequest.engine_config` and reaches the build `EngineSpec`.
- Every proven optional capability has a real integration test:
  `host_dir_live`, `debug_native`, and `nat`. A capability that cannot be
  tested is marked `unproven` in the docs table.
- `dosker engines` prints one line per known engine. A missing engine
  prints `{name} not found: {error}`. It exits 1 only when no engine is
  found.
- `dosker doctor` prints one line per engine. It exits 0 when the data
  root is writable and at least one engine is found.
- The `doctor` warning is `development mode: --bind uses QEMU vvfat
  (beta) and can corrupt host files. Use it for development only, never
  in CI.` Keep it verbatim.
- The docs capability table marks a capability the spike tested but did
  not prove as `unproven`. `gdbstub`, `screenshot`, and `vnc` are `no`
  because the spike does not test them and the slice never declares them.
- The `validate_network` helper keeps the `CapabilitySet` parameter. The
  engine name is used only for the provider message.
- The supervisor passes `record.engine_config` to
  `EngineSpec.engine_config`.
- Build-time DOSBox-X `RUN`, screenshot, VNC, gdbstub, Bochs, a shared L2
  network, and any capability the spike does not prove are out of scope.
- The QEMU build resolver and the in-DOS `RUN` path stay QEMU-only.
- The DOSBox-X crate must compile on Linux x86_64, macOS arm64, and
  Windows x86_64. The real-engine integration test runs on Unix.
- No transient references in code, identifiers, test names, strings, or
  comments. Do not name slices, tasks, requirement IDs, milestones, or
  plan paths in product code.

## Interfaces

The following names and signatures are fixed across tasks. Adjust none of
them without changing every consumer in the same task.

`crates/dosker-engine/src/provider.rs` is the single source of truth for
implemented engine names and capability sets. `dosker-engine-qemu` and
`dosker-engine-dosbox-x` return `dosker_engine::declared_capabilities`
from their `capabilities()` functions. The CLI and build use
`providers`, `capability_message`, and `check_requires` from
`dosker-engine`.

```rust
// dosker-engine
pub struct EngineSpec { /* existing */
    pub engine_config: BTreeMap<String, Vec<String>>,
}

pub enum DebugKind { Gdb, Native } // serde: "gdb", "native"

pub struct EngineProvider {
    pub name: &'static str,
    pub capabilities: &'static [Capability],
}
pub const IMPLEMENTED_ENGINES: &[EngineProvider];

pub fn declared_capabilities(engine: &str) -> Option<CapabilitySet>;
pub fn is_known(engine: &str) -> bool;
pub fn implemented_engine_names() -> Vec<&'static str>;
pub fn supported_engines_message() -> String;
pub fn providers(capability: Capability) -> Vec<&'static str>;
pub fn capability_message(engine: &str, capability: Capability) -> String;
pub fn check_requires(engine: &str, capabilities: CapabilitySet,
    requires: &[String]) -> Result<(), String>;

// dosker-build
pub struct InDosRunRequest<'a> { /* existing */
    pub engine_config: &'a BTreeMap<String, Vec<String>>,
}

// dosker-image
pub struct Image { /* existing */
    pub engine_pin: Option<String>,
    pub engine_config: BTreeMap<String, Vec<String>>,
    pub requires: Vec<String>,
}

// dosker-runtime
pub struct ContainerRecord { /* existing */
    #[serde(default)]
    pub engine_config: BTreeMap<String, Vec<String>>,
}
impl ContainerRecord {
    pub fn with_engine_config(self, engine_config: BTreeMap<String, Vec<String>>) -> Self;
}

// dosker-engine-qemu
pub fn split_engine_config(line: &str) -> Result<Vec<String>, EngineError>;
pub fn capabilities() -> CapabilitySet;
// declared_capabilities("qemu").expect("qemu is an implemented engine")

// dosker-engine-dosbox-x
pub const ENGINE_NAME: &str = "dosbox-x";
pub const DOSBOX_X_ENV: &str = "DOSKER_DOSBOX_X";
pub const DOSBOX_X_VERSION_ENV: &str = "DOSKER_DOSBOX_X_VERSION";
pub const DOSBOX_X_SHA256_ENV: &str = "DOSKER_DOSBOX_X_SHA256";
pub const DEFAULT_DOSBOX_X: &str = "dosbox-x";
pub const CONFIG_FILE_NAME: &str = "dosbox-x.conf";
pub const HEADLESS_FLAGS: &[&str];

pub struct DosboxXBinary {
    pub path: PathBuf,
    pub version: String,
}
impl DosboxXBinary {
    pub fn discover() -> Result<Self, EngineError>;
    pub fn from_path(path: &Path) -> Result<Self, EngineError>;
    pub fn version_string(&self) -> &str;
    pub fn path(&self) -> &Path;
}

pub fn capabilities() -> CapabilitySet;
// declared_capabilities(ENGINE_NAME).expect("dosbox-x is an implemented engine")

pub struct DosboxXAdapter {
    binary: DosboxXBinary,
    version: String,
    processes: Arc<dyn ProcessSupervisor>,
}
impl DosboxXAdapter {
    pub fn new(binary: DosboxXBinary, processes: Arc<dyn ProcessSupervisor>) -> Self;
    pub fn discover(processes: Arc<dyn ProcessSupervisor>) -> Result<Self, EngineError>;
}

pub struct DosboxXEndpoints {
    pub agent_serial: Option<EngineEndpointKind>,
    pub debug: Option<EngineEndpointKind>,
}
impl DosboxXEndpoints {
    pub fn for_spec(spec: &EngineSpec) -> Result<Self, EngineError>;
    pub fn from_command(command: &EngineCommand) -> Result<Self, EngineError>;
    pub fn engine_endpoints(&self) -> Vec<EngineEndpoint>;
    pub fn remove_stale(&self) -> Result<(), EngineError>;
}

pub fn render_config(spec: &EngineSpec, endpoints: &DosboxXEndpoints)
    -> Result<String, EngineError>;
pub fn write_config(spec: &EngineSpec, endpoints: &DosboxXEndpoints)
    -> Result<PathBuf, EngineError>;

pub struct DosboxXSession {
    child: Child,
    spec: EngineSpec,
    command: EngineCommand,
    endpoints: DosboxXEndpoints,
    agent: Option<AgentClient<SerialLog<DosboxXStream>>>,
    exit_code: Option<i32>,
}
impl DosboxXSession {
    pub fn spawn(spec: &EngineSpec, processes: &dyn ProcessSupervisor,
        command: EngineCommand) -> Result<Self, EngineError>;
}

// dosker-cli
pub struct CreateOptions { /* existing */ pub engine: Option<String> }
pub fn is_engine_name(engine: &str) -> bool;
pub fn discover(engine: &str) -> Result<DiscoveredEngine, EngineError>;
pub struct DiscoveredEngine {
    pub name: &'static str,
    pub version: String,
    pub path: PathBuf,
    pub capabilities: CapabilitySet,
}
pub fn check_requires(engine: &str, requires: &[String]) -> Result<(), CliError>;
```

`DosboxXStream` and `SerialLog<T>` are crate-private types in
`dosker-engine-dosbox-x`; they do not appear in a public signature.

## Tasks

- [ ] **Task 1: Provider table, spike gate, and QEMU capability source**
    - **Implements:** FR-011, FR-016; design §Engine Strategy, §Engine
      Capability Matrix, §Error Handling, §Testing Strategy; DOSBox-X
      spike Ground Rules.
    - **Precondition before any code:** read the completed Results table
      in `assets/dosbox-x-spike.md`.
        - If `boot` is not `proven`, stop the slice and report the spike
          result. Do not continue.
        - If `serial` or `headless` is not `proven`, stop and report that
          the slice cannot meet its acceptance bar.
        - Record the `proven` rows and every row of the Decisions table.
          Tasks 2, 5, 6, 12, and 13 consume them.
    - **Create `crates/dosker-engine/tests/provider.rs`**
        - `declared_capabilities_match_the_qemu_set` — pins the QEMU
          entry to `boot,serial,stop,headless,host_dir_live,debug_native,
          gdbstub,screenshot,vnc,nat`.
        - `declared_capabilities_match_the_dosbox_x_proven_rows` — pins
          the DOSBox-X entry to a literal `[Capability; N]` slice copied
          from the spike's `proven` rows. Do not compare to a production
          constant.
        - `providers_are_in_table_order` — `providers(Capability::
          Headless)` returns `qemu` before `dosbox-x`.
        - `capability_message_uses_the_one_provider_form` — `gdbstub` on
          `dosbox-x` is `engine dosbox-x does not provide the gdbstub
          capability; the qemu engine provides it`.
        - `capability_message_uses_the_two_provider_form` — `headless`
          on a synthetic `mock` engine is `engine mock does not provide
          the headless capability; the qemu and dosbox-x engines provide
          it`.
        - `capability_message_uses_the_zero_provider_form` — `audio` is
          `engine dosbox-x does not provide the audio capability; no
          engine provides it`.
        - `check_requires_rejects_a_missing_capability` — calls
          `check_requires("dosbox-x",
          declared_capabilities("dosbox-x").expect("dosbox-x is an
          implemented engine"), &["gdbstub".to_owned()])` and pins the
          message.
        - `check_requires_rejects_an_unknown_capability` — pins
          `unknown capability: {name}`.
        - `check_requires_honors_the_supplied_capability_set` — builds
          the mock set `[Boot, Serial, Stop, Nat]`, calls
          `check_requires("mock", mock_set, &["nat".to_owned()])` and
          asserts success; calls the same set with `gdbstub` and asserts
          the provider message.
        - `supported_engines_message_lists_the_implemented_engines` —
          pins `qemu and dosbox-x`.
    - **Create `crates/dosker-engine/src/provider.rs`**
        - `EngineProvider { pub name: &'static str, pub capabilities:
          &'static [Capability] }`.
        - `IMPLEMENTED_ENGINES` — the QEMU entry is the existing set; the
          DOSBox-X entry is exactly the rows the completed spike record
          marks `proven`, in `Capability::ALL` order, excluding
          `GdbStub`, `Screenshot`, and `Vnc`.
        - `declared_capabilities`, `is_known`, `implemented_engine_names`,
          `supported_engines_message`, `providers`, `capability_message`,
          and `check_requires(engine, capabilities, requires)`.
        - `check_requires` uses the supplied `capabilities` for the check
          and the provider table only for the message.
        - `format_capability_message(engine, capability, providers)` —
          implements the zero-, one-, two-, and three-or-more-provider
          forms.
        - Add a `#[cfg(test)] mod tests` in `src/provider.rs`:
          `capability_message_uses_the_three_provider_form` calls
          `format_capability_message("mock", Capability::GdbStub,
          &["qemu", "dosbox-x", "bochs"])` and pins `engine mock does not
          provide the gdbstub capability; the qemu, dosbox-x, and bochs
          engines provide it`.
    - **Modify `crates/dosker-engine/src/lib.rs`**
        - Declare the `provider` module and re-export the new items.
    - **Modify `crates/dosker-engine-qemu/src/adapter.rs`**
        - `capabilities()` returns
          `dosker_engine::declared_capabilities("qemu").expect("qemu is
          an implemented engine")`.
    - **Verify:** `cargo test -p dosker-engine -p dosker-engine-qemu`

- [ ] **Task 2: DOSBox-X crate scaffold, discovery, and capability identity**
    - **Implements:** FR-011; design §Engine Strategy, §Testing Strategy;
      the spike Decisions table.
    - **Create `crates/dosker-engine-dosbox-x/tests/discovery.rs`**
        - `parses_the_version_from_the_version_line` — pins
          `DOSBox-X version 2026.10.01 SDL2, ...` to version
          `2026.10.01`.
        - `rejects_a_missing_binary` — pins `EngineError::NotFound`.
        - `checksum_pin_accepts_a_matching_file` — pins the
          `DOSKER_DOSBOX_X_SHA256` path.
        - `checksum_pin_rejects_a_modified_file` — pins the mismatch
          error.
        - `version_pin_rejects_a_mismatch` — pins the expected and found
          versions in the error.
        - `discovers_the_binary_named_by_the_environment` — pins
          `DOSKER_DOSBOX_X` over `PATH`.
        - Add `static ENV_LOCK: Mutex<()>` and an `EnvRestore` guard;
          every test that sets `DOSKER_DOSBOX_X`,
          `DOSKER_DOSBOX_X_VERSION`, or `DOSKER_DOSBOX_X_SHA256` holds
          the lock and restores the prior value on drop.
        - Add the Windows `program_candidates` test as a
          `#[cfg(all(test, windows))]` unit test in `src/discovery.rs`;
          it calls the private `program_candidates` and pins the
          `PATHEXT` candidate list.
    - **Create `crates/dosker-engine-dosbox-x/tests/capability.rs`**
        - `declares_exactly_the_proven_rows` — pins `capabilities()` to
          a literal `[Capability; N]` slice copied from the spike's
          `proven` rows. Do not compare to a production constant.
        - `declares_boot_serial_and_headless` — pins the three rows the
          slice gate requires.
        - `never_declares_gdbstub_screenshot_or_vnc` — pins the three
          rows the slice excludes.
    - **Create `crates/dosker-engine-dosbox-x/Cargo.toml`**
        - `[dependencies]`: `dosker-core.workspace = true`,
          `dosker-engine.workspace = true`,
          `dosker-platform.workspace = true`,
          `dosker-agent-proto.workspace = true`,
          `sha2.workspace = true`.
        - `[dev-dependencies]`: `tempfile.workspace = true`.
    - **Modify `Cargo.toml` (workspace)**
        - Add `"crates/dosker-engine-dosbox-x"` to `members`.
        - Add `dosker-engine-dosbox-x = { path =
          "crates/dosker-engine-dosbox-x" }` to
          `[workspace.dependencies]`.
    - **Create `crates/dosker-engine-dosbox-x/src/lib.rs`**
        - Declare the `adapter` and `discovery` modules.
        - Re-export `capabilities`, `DosboxXBinary`, and the four
          discovery constants. Later tasks extend this list as their
          modules land.
    - **Create `crates/dosker-engine-dosbox-x/src/discovery.rs`**
        - `DOSBOX_X_ENV`, `DOSBOX_X_VERSION_ENV`,
          `DOSBOX_X_SHA256_ENV`, `DEFAULT_DOSBOX_X`, and the version
          marker `"DOSBox-X version "`.
        - `DosboxXBinary { pub path: PathBuf, pub version: String }`.
        - `DosboxXBinary::discover()` — reads `DOSKER_DOSBOX_X` or
          `dosbox-x`, resolves a bare name on `PATH`, and adds `PATHEXT`
          candidates on Windows.
        - `DosboxXBinary::from_path(path: &Path)` — checks the file,
          verifies `DOSKER_DOSBOX_X_SHA256` when set, runs `--version`,
          parses the token after the version marker, and checks
          `DOSKER_DOSBOX_X_VERSION` when set.
        - `DosboxXBinary::version_string()` and `DosboxXBinary::path()`.
    - **Create `crates/dosker-engine-dosbox-x/src/adapter.rs`**
        - `ENGINE_NAME`.
        - `capabilities()` — returns
          `dosker_engine::declared_capabilities(ENGINE_NAME)
          .expect("dosbox-x is an implemented engine")`.
    - **Verify:** `cargo test -p dosker-engine-dosbox-x`

- [ ] **Task 3: Persist the engine configuration on the container record**
    - **Implements:** FR-011; design §Build And Runtime Semantics.
    - **Modify `crates/dosker-runtime/tests/record.rs`**
        - `new_record_starts_with_empty_engine_config` — pins the
          default.
        - `with_engine_config_records_the_map` — pins the builder.
        - `serde_round_trip_preserves_every_field` — sets and restores
          `engine_config`.
        - `an_existing_state_file_loads_with_empty_engine_config` —
          removes the field from the JSON and pins the empty default.
    - **Modify `crates/dosker-runtime/src/record.rs`**
        - `ContainerRecord` gains
          `#[serde(default)] pub engine_config: BTreeMap<String,
          Vec<String>>`.
        - `new` sets `engine_config: BTreeMap::new()`.
        - Add `with_engine_config`.
        - Keep `RECORD_SCHEMA_VERSION = 1`; the field is additive.
    - **Verify:** `cargo test -p dosker-runtime`

- [ ] **Task 4: Engine spec and QEMU engine configuration**
    - **Implements:** FR-011; design §Component Interfaces (Engine
      Adapter); doskerfile §ENGINE-CONFIG.
    - **Modify `crates/dosker-engine/tests/adapter.rs`**
        - `engine_config_reaches_the_adapter` — a recording adapter
          captures `spec.engine_config` and asserts the exact map.
    - **Create `crates/dosker-engine-qemu/tests/config.rs`**
        - `splits_plain_tokens` — `-smp 2` becomes `["-smp", "2"]`.
        - `splits_quoted_tokens` — `-drive "file=a b.img"` becomes
          `["-drive", "file=a b.img"]`.
        - `splits_single_quoted_tokens` — `-drive 'file=a b.img'`
          becomes `["-drive", "file=a b.img"]`.
        - `escapes_a_space_outside_quotes` — `-drive file=a\ b.img`
          becomes `["-drive", "file=a b.img"]`.
        - `escapes_a_quote_inside_double_quotes` — `-drive "file=a\"b"`
          becomes `["-drive", "file=a\"b"]`.
        - `rejects_an_unterminated_quote` — pins `EngineError::Setup`
          with `unterminated quote`.
        - `rejects_a_trailing_backslash` — pins `EngineError::Setup`.
    - **Modify `crates/dosker-engine-qemu/tests/command.rs`**
        - `appends_engine_config_arguments_in_order` — a spec with two
          `qemu` config lines appends the split arguments after the
          built-in arguments, in order.
    - **Modify `crates/dosker-engine/src/spec.rs`**
        - `EngineSpec` gains `pub engine_config: BTreeMap<String,
          Vec<String>>`.
    - **Modify `crates/dosker-runtime/src/supervisor.rs`**
        - The `EngineSpec` literal sets
          `engine_config: record.engine_config.clone()`.
    - **Modify `crates/dosker-build/src/run_in_dos.rs`**
        - `InDosRunRequest` gains `pub engine_config: &'a
          BTreeMap<String, Vec<String>>`.
    - **Modify `crates/dosker-build/src/build.rs`**
        - The `InDosRunRequest` literal sets `engine_config:
          &state.config.engine_config`.
    - **Modify `crates/dosker-build/src/run_in_dos/engine.rs`**
        - The `EngineSpec` literal sets `engine_config:
          request.engine_config.clone()`; no `BTreeMap` import is needed.
    - **Modify `crates/dosker-build/tests/run_in_dos_engine.rs`**
        - Add an `engine_config: BTreeMap<String, Vec<String>>` field to
          the fixture; `Fixture::request` sets `engine_config:
          &fixture.engine_config` so the reference outlives the request.
        - `an_engine_config_line_reaches_the_run_spec` — a fixture with
          `engine_config["qemu"] = ["-smp 2"]` records a spec whose
          `engine_config["qemu"]` is `["-smp 2"]`.
    - **Modify every other `EngineSpec` literal in tests**
        - `crates/dosker-engine-qemu/src/session.rs` (`session_spec`).
        - `crates/dosker-engine-qemu/tests/command.rs` (`sample_spec`).
        - `crates/dosker-engine-qemu/tests/endpoints.rs` (`spec_for`).
        - `crates/dosker-engine-qemu/tests/qemu_session.rs`.
        - `crates/dosker-engine-qemu/tests/session_endpoints.rs`
          (`spec`).
        - `crates/dosker-engine/tests/adapter.rs` (`sample_spec`).
        - `crates/dosker-test-support/tests/mock_adapter.rs` (`spec`).
        - Each new literal sets `engine_config: BTreeMap::new()`.
    - **Create `crates/dosker-engine-qemu/src/config.rs`**
        - `split_engine_config(line: &str) -> Result<Vec<String>,
          EngineError>` — implements the shell-like quoting rules.
    - **Modify `crates/dosker-engine-qemu/src/command.rs`**
        - Append every split `spec.engine_config["qemu"]` line to the
          argument vector after the built-in arguments, in order.
    - **Modify `crates/dosker-engine-qemu/src/lib.rs`**
        - Declare the `config` module and re-export
          `split_engine_config`.
    - **Verify:** `cargo test --workspace --features dosker-cli/mock-engine`

- [ ] **Task 5: DOSBox-X config generation and endpoints**
    - **Implements:** FR-011; design §Component Interfaces (Engine
      Adapter), §Build And Runtime Semantics; doskerfile
      §ENGINE-CONFIG; the spike Decisions table.
    - **Create `crates/dosker-engine-dosbox-x/tests/endpoints.rs`**
        - `uses_the_recorded_agent_endpoint_kind` — pins the agent
          endpoint kind to the kind the spike records.
        - `allocates_distinct_tcp_ports_when_the_spike_records_tcp` —
          when the recorded kind is TCP, pins two distinct loopback
          ports.
        - `uses_the_container_socket_path_on_unix_when_the_spike_records_a_pipe`
          — when the recorded kind is a pipe, pins the
          `engine_socket_path` fallback.
        - `a_pipe_endpoint_is_unsupported_on_windows` — write this test
          only when the spike records a pipe endpoint; pins
          `EngineError::Unsupported` on Windows.
        - `removes_stale_socket_files` — construct a `DosboxXEndpoints`
          with an explicit `EngineEndpointKind::UnixSocket` path on Unix,
          create the stale file, call `remove_stale`, and assert removal.
          Do not depend on the recorded endpoint kind.
        - `a_booter_has_no_agent_endpoint` — pins the booter case.
    - **Create `crates/dosker-engine-dosbox-x/tests/config.rs`**
        - `renders_the_default_machine_keys` — pins `[dosbox] machine =
          svga_s3`, `[dosbox] memsize = 16`, `[cpu] cputype = 486`,
          `[sblaster] sbtype = sb16`, and `[ide, primary] enable = true`.
        - `renders_both_serial_ports` — with `control_port = 2`, pins
          `serial1` to the guest line recorded in the spike and `serial2`
          to the agent line.
        - `renders_the_agent_on_serial1_when_the_control_port_is_one` —
          with `control_port = 1`, pins the agent line at `serial1` and
          asserts no separate guest line.
        - `renders_the_boot_command` — pins the boot command form
          recorded in the spike with `spec.disk_path` substituted.
        - `appends_engine_config_lines_before_autoexec` — pins verbatim
          lines, order, and position before `[autoexec]`.
        - `tracks_an_engine_config_autoexec_header` — pins that a raw
          `[autoexec]` header does not produce a second header.
        - Write only the test that matches the spike row. Do not write
          both sides of a capability-conditional pair.
        - `writes_host_directory_mounts_when_declared` — write this test
          only when `HostDirLive` is proven; pins the mount line recorded
          in the spike for each `EngineVolumeKind::HostDir`, sorted by
          drive.
        - `rejects_host_directories_without_the_capability` — write this
          test only when `HostDirLive` is not proven; pins
          `EngineError::Unsupported("host directory")`.
        - `writes_slirp_port_forwards_when_declared` — write this test
          only when `Nat` is proven and `spec.net == "ne2000"`; pins the
          `[ne2000]` and `[ethernet, slirp]` keys and one
          `tcp_port_forwards` entry per `spec.ports`.
        - `rejects_ne2000_without_the_capability` — write this test only
          when `Nat` is not proven; a spec with `spec.net == "ne2000"`
          pins `EngineError::Unsupported("network address translation")`
          and the provider message, even with no published ports.
        - `omits_the_nic_sections_when_the_network_is_none` — pins the
          absence of `[ne2000]` and `[ethernet, slirp]` when
          `spec.net == "none"`.
        - `writes_the_debug_key_when_declared` — write this test only
          when `DebugNative` is proven; pins the debug key recorded in the
          spike.
        - `omits_the_agent_serial_line_for_a_booter` — pins the booter
          config.
        - `writes_a_deterministic_config` — pins byte-for-byte
          repeatability for the same spec.
    - **Create `crates/dosker-engine-dosbox-x/src/endpoints.rs`**
        - `DosboxXEndpoints { pub agent_serial: Option<EngineEndpointKind>,
          pub debug: Option<EngineEndpointKind> }`.
        - `for_spec` — uses the recorded agent and debug endpoint kinds;
          allocates a loopback TCP port or a container socket path;
          returns `None` for a booter agent.
        - `from_command` — reads the `serial_agent` and `debug`
          endpoints from `EngineCommand.endpoints`.
        - `engine_endpoints` — returns `EngineEndpoint` values named
          `serial_agent` and `debug`.
        - `remove_stale` — removes stale Unix socket files; TCP has no
          files.
    - **Create `crates/dosker-engine-dosbox-x/src/config.rs`**
        - `CONFIG_FILE_NAME = "dosbox-x.conf"`.
        - `render_config(spec, endpoints) -> Result<String,
          EngineError>` — writes the default machine keys, both serial
          ports, the `[ne2000]` and `[ethernet, slirp]` sections only
          when `spec.net == "ne2000"` and `Nat` is proven, the debug key
          when `DebugNative` is proven, each raw `spec.engine_config`
          `dosbox-x` line verbatim in order, and `[autoexec]` with the
          host-directory mounts and the boot command form recorded in the
          spike. A `spec.net == "ne2000"` with an unproven `Nat` returns
          `EngineError::Unsupported("network address translation")`; it
          never silently omits the NIC.
        - `write_config(spec, endpoints) -> Result<PathBuf,
          EngineError>` — creates `spec.log_dir` and writes the rendered
          text to `spec.log_dir.join(CONFIG_FILE_NAME)`.
    - **Modify `crates/dosker-engine-dosbox-x/src/lib.rs`**
        - Declare the `config` and `endpoints` modules.
        - Re-export `CONFIG_FILE_NAME`, `render_config`, `write_config`,
          and `DosboxXEndpoints`.
    - **Verify:** `cargo test -p dosker-engine-dosbox-x`

- [ ] **Task 6: DOSBox-X adapter and session**
    - **Implements:** FR-011; design §Component Interfaces (Engine
      Adapter), §Build And Runtime Semantics, §Error Handling; the spike
      Decisions table.
    - **Create `crates/dosker-engine-dosbox-x/tests/adapter.rs`**
        - `build_command_writes_the_config_and_returns_the_conf_argument`
          — pins the `-conf <path>` argument and the written file.
        - `build_command_adds_the_headless_flags_for_display_none` —
          pins `HEADLESS_FLAGS` set to the form recorded in the spike.
        - `build_command_omits_the_headless_flags_for_display_window` —
          pins the windowed case.
        - `build_command_rejects_vnc` — pins
          `EngineError::Unsupported("vnc")` because `Vnc` is never
          declared.
        - `probe_rejects_a_missing_binary` — pins
          `EngineError::NotFound`.
        - `start_spawns_the_process_with_the_generated_command` — pins
          the child pid and the `engine.log` capture.
    - **Create `crates/dosker-engine-dosbox-x/src/session.rs` test
      module**
        - `ready_performs_the_agent_handshake` — a fake TCP agent sends
          `HELLO`; pins the handshake.
        - `exec_returns_the_agent_output_and_status` — pins `HELLO` and
          status 0 from a fake `EXEC`.
        - `put_and_get_round_trip_a_file` — pins the agent transfer.
        - `stop_sends_shutdown_and_reports_the_exit` — pins the agent
          `SHUTDOWN` and the recorded exit code.
        - `stop_times_out_and_kills_a_guest_that_does_not_exit` — pins
          `EngineError::StopTimeout`.
        - `kill_ends_the_process` — pins the exit code.
        - `ready_times_out_without_an_agent` — pins
          `EngineError::ReadyTimeout`.
        - `booter_ready_accepts_a_live_process` and
          `booter_ready_reports_an_exited_process` — pin the booter
          readiness rule.
        - `booter_exec_returns_no_agent` — pins `EngineError::NoAgent`.
        - `booter_transfer_returns_no_agent` — pins `EngineError::NoAgent`
          for `put_file` and `get_file`.
    - **Create `crates/dosker-engine-dosbox-x/src/stream.rs`**
        - Crate-private `DosboxXStream` with `Tcp(TcpStream)` and, on
          Unix, `Unix(UnixStream)`; implements `Read`, `Write`, and
          `dosker_agent_proto::Transport`.
        - `connect(kind, deadline)` and `probe(kind, timeout)`; a TCP
          attempt retries every 20 milliseconds until the deadline.
    - **Create `crates/dosker-engine-dosbox-x/src/serial_log.rs`**
        - Crate-private `SerialLog<T: Transport>` that appends every
          transferred byte to `serial.log`; a log failure must not fail a
          protocol read or write.
    - **Modify `crates/dosker-engine-dosbox-x/src/adapter.rs`**
        - `HEADLESS_FLAGS` — set to the headless flags recorded in the
          spike.
        - `DosboxXAdapter { binary, version, processes }`.
        - `new` and `discover`.
        - `EngineAdapter::name`, `version`, `capabilities`, `probe`,
          `build_command`, `start`, and `start_with_command`.
        - `build_command` builds `DosboxXEndpoints::for_spec`, calls
          `write_config`, returns `EngineCommand` with `-conf <path>`
          plus `HEADLESS_FLAGS` for `DisplayMode::None`, and rejects
          `DisplayMode::Vnc`.
    - **Create `crates/dosker-engine-dosbox-x/src/session.rs`**
        - `DosboxXSession::spawn` — creates `spec.log_dir`, reads the
          endpoints from the command, removes stale sockets, opens
          `engine.log`, spawns the process through
          `ProcessSupervisor::prepare_engine` with stdin null and
          stdout/stderr on the log, and assigns the Windows
          `JobObject::kill_on_close`.
        - `EngineSession::pid`, `process_group`, `ready`, `exec`,
          `stop`, `kill`, `try_exit`, `command`, `endpoints`,
          `put_file`, `get_file`, and `debug_endpoint`.
        - `ready` connects to `endpoints.agent_serial`, wraps the stream
          in `SerialLog`, runs `AgentClient::handshake` with the
          remaining deadline, and returns `Exited` or `ReadyTimeout` on
          failure. For a booter, it sleeps 250 milliseconds and returns
          `Exited` when the process has exited, otherwise `Ok(())`.
        - `exec` maps `ExecRequest` to the agent protocol and
          `ProtoError` to `EngineError`: `ProtoError::Timeout` becomes
          `EngineError::ExecTimeout(timeout)`, `GuestRebooted` becomes
          `GuestRebooted`, `Agent` becomes `Agent`, `Io` becomes `Io`,
          and other errors become `Protocol`.
        - `exec`, `put_file`, and `get_file` return
          `EngineError::NoAgent` when `spec.booter` is true or no agent is
          connected.
        - `stop` sends the agent `SHUTDOWN` when an agent is present,
          applies the stop mechanism recorded in the spike, waits for
          the process until the grace deadline, and returns
          `StopTimeout` after a kill.
        - At this task `DebugKind` has only `Gdb`; `debug_endpoint`
          maps `Gdb` to `Unsupported("debugger")`. Task 11 adds
          `DebugKind::Native` and maps it to the native endpoint.
        - A `Drop` that kills a live child, reaps it, and drops the
          Windows job after the child is reaped.
    - **Modify `crates/dosker-engine-dosbox-x/src/lib.rs`**
        - Declare the `serial_log`, `session`, and `stream` modules.
        - Re-export `DosboxXAdapter`, `HEADLESS_FLAGS`, and
          `DosboxXSession`.
    - **Verify:** `cargo test -p dosker-engine-dosbox-x`

- [ ] **Task 7: Expose the image engine pin, config, and requirements**
    - **Implements:** FR-011, FR-016; design §Build And Runtime
      Semantics; doskerfile §ENGINE, §ENGINE-CONFIG, §REQUIRES.
    - **Modify `crates/dosker-image/tests/oci.rs`**
        - `load_returns_the_engine_pin_config_and_requires` — commits
          an image with `engine_pin: Some("dosbox-x")`, a `dosbox-x`
          config line, and `requires: ["boot", "headless"]`; pins the
          `Image` fields.
        - `an_imported_image_has_no_engine_pin_or_requires` — pins the
          import defaults.
    - **Modify `crates/dosker-image/src/oci.rs`**
        - `Image` gains `pub engine_pin: Option<String>`, `pub
          engine_config: BTreeMap<String, Vec<String>>`, and `pub
          requires: Vec<String>`.
        - `load` copies `config.engine_pin`, `config.engine_config`, and
          `config.requires`.
    - **Verify:** `cargo test -p dosker-image`

- [ ] **Task 8: CLI engine registry, discovery, and reporting**
    - **Implements:** FR-011; design §Component Interfaces (CLI),
      §Error Handling.
    - **Modify `crates/dosker-cli/tests/cli_engines.rs`**
        - `engines_reports_both_engines` — replaces
          `engines_reports_the_qemu_version_and_path`; pins the QEMU and
          DOSBox-X lines.
        - `engines_lists_the_dosbox_x_proven_capabilities` — replaces
          `engines_lists_the_proven_capabilities`; pins the DOSBox-X
          capability field to the spike rows and keeps a QEMU
          capability assertion.
        - `engines_exits_zero_when_only_one_engine_is_found` — replaces
          `engines_exits_one_when_qemu_is_missing`; QEMU missing,
          DOSBox-X found.
        - `engines_exits_one_when_no_engine_is_found` — both missing;
          pins both `not found` lines.
        - `engines_reports_an_unavailable_engine` — replaces
          `engines_exits_one_for_a_broken_qemu`; a broken QEMU and a
          found DOSBox-X; pins `qemu unavailable:` and exit 0.
        - `doctor_reports_the_data_root_and_both_engines` — replaces
          `doctor_reports_the_data_root_and_qemu`; pins the data-root
          lines and both engine lines.
        - `doctor_lists_each_engine_with_its_capabilities` — pins
          `{name}: {version} ({path}) [{capabilities}]`.
        - `doctor_exits_zero_when_only_one_engine_is_found` — replaces
          `doctor_exits_one_when_qemu_is_missing`; QEMU missing,
          DOSBox-X found.
        - `doctor_exits_one_when_no_engine_is_found` — both missing.
        - `doctor_reports_unavailable_when_an_engine_fails` — replaces
          `doctor_reports_unavailable_when_qemu_exits_nonzero`; a broken
          QEMU and a found DOSBox-X.
        - `doctor_reports_unavailable_when_an_engine_version_is_unparsable`
          — replaces
          `doctor_reports_unavailable_when_the_qemu_version_is_unparsable`.
        - `doctor_reports_an_unwritable_root` — keeps the test; pins
          both engines.
        - `doctor_prints_the_security_statement` — keeps the test; pins
          both engines.
        - `doctor_prints_the_development_mode_warning` — keeps the test;
          pins both engines and the verbatim warning string.
        - Pin `DOSKER_DOSBOX_X` in every engine and doctor test. Add
          `write_dosbox_x_script` and `run_with_engines(root, qemu,
          dosbox_x, args)` so every test names both binaries explicitly.
          A test that expects only QEMU found sets `DOSKER_DOSBOX_X` to
          a missing path.
    - **Modify `crates/dosker-cli/Cargo.toml`**
        - Add `dosker-engine-dosbox-x.workspace = true`.
    - **Modify `crates/dosker-cli/src/adapter.rs`**
        - `is_known` and `declared_capabilities` use
          `dosker_engine::is_known` and
          `dosker_engine::declared_capabilities`; the `mock` feature
          branch stays.
        - `build` adds `dosbox-x`; the `mock` and `qemu` branches stay.
        - `is_engine_name` uses `dosker_core::EngineName::parse`.
        - `supported_engines_message` delegates to
          `dosker_engine::supported_engines_message`.
        - `DiscoveredEngine` and `discover(engine)`.
        - `default_engine` stays QEMU.
        - Do not add a local provider table. Use the `dosker-engine`
          items. The CLI keeps only the engine-name dispatch (`is_known`,
          `declared_capabilities`, `build`, `discover`).
    - **Modify `crates/dosker-cli/src/commands/engines.rs`**
        - Iterate `dosker_engine::IMPLEMENTED_ENGINES`; print
          `{name} {version} {path} {capabilities}` for a found engine,
          `{name} not found: {error}` for `NotFound`, and
          `{name} unavailable: {error}` for another error.
        - Exit `EXIT_OK` when at least one engine is found; otherwise
          `EXIT_ERROR`.
    - **Modify `crates/dosker-cli/src/commands/doctor.rs`**
        - Iterate `dosker_engine::IMPLEMENTED_ENGINES`; print one line
          per engine: `{name}: {version} ({path}) [{capabilities}]`,
          `{name}: not found ({error})`, or
          `{name}: unavailable ({error})`.
        - Remove the single `capabilities:` line.
        - Keep `DEVELOPMENT_MODE_WARNING` verbatim.
        - Exit `EXIT_OK` when the data root is writable and at least one
          engine is found; otherwise `EXIT_ERROR`.
    - **Verify:** `cargo test -p dosker-cli --features qemu-engine,mock-engine --lib && cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_engines`

- [ ] **Task 9: Runtime engine selection and the unavailable-engine message**
    - **Implements:** FR-011; design §Build And Runtime Semantics;
      doskerfile §ENGINE, §ENGINE-CONFIG.
    - **Modify `crates/dosker-cli/tests/common/mod.rs`**
        - Add `import_image_with_config(root: &CliRoot, source: &Path,
          reference: &str, engine_pin: Option<&str>, engine_config:
          BTreeMap<String, Vec<String>>, requires: Vec<&str>) -> String`:
          imports the given disk image, loads its config and manifest,
          replaces `engine_pin`, `engine_config`, and `requires`, and
          commits the image with the original manifest annotations.
        - Add `import_image_for_dosbox_x(root: &CliRoot, source: &Path,
          reference: &str) -> String`: imports the disk image, rewrites
          `machine.net = "none"`, and commits it. Use it for the
          DOSBox-X tests that must boot when `nat` is not provided.
        - Add `command_with_dosbox_x` and `run_with_dosbox_x`, setting
          `DOSKER_DOSBOX_X` from
          `std::env::var("DOSKER_DOSBOX_X").unwrap_or_else(|_|
          "dosbox-x".to_owned())` and using a 180-second timeout. Do not
          depend on a test-support fixture for the binary.
    - **Create `crates/dosker-cli/tests/cli_create.rs`**
        - Add `mod common;` and the `#[cfg(unix)]` items the helper uses.
        - `create_uses_the_image_engine_pin` — an image pinned to
          `dosbox-x` creates a `dosbox-x` record without `--engine`.
        - `create_prefers_the_engine_flag_over_the_image_pin` — the
          same image with `--engine qemu` creates a `qemu` record.
        - `create_falls_back_to_the_default_engine` — an image with no
          pin and no `--engine` creates a `qemu` record.
        - `create_copies_the_engine_config` — the record carries the
          image's `engine_config` map.
        - `create_rejects_bochs_with_an_unavailable_message` — exits 2
          with `engine bochs is not available; supported engines are qemu
          and dosbox-x`.
        - `create_rejects_an_unknown_engine_string` — exits 2 with
          `unknown engine: nope`.
        - The tests generate their disk image with `make_disk_image` and
          pass it to `import_image_with_config`.
    - **Modify `crates/dosker-cli/src/commands/create.rs`**
        - `CreateOptions.engine` becomes `Option<String>`.
        - `execute` passes `args.engine` without resolving it.
        - `create_container` loads the image, then resolves the engine:
          `options.engine`, then `image.engine_pin`, then
          `adapter::default_engine()`.
        - After resolution, `is_known` accepts the engine. If it is not
          known and `adapter::is_engine_name` is true, return
          `engine {engine} is not available; supported engines are
          {adapter::supported_engines_message()}`. Otherwise return
          `unknown engine: {engine}`.
        - The record chains
          `.with_engine_config(image.engine_config.clone())`.
    - **Modify `crates/dosker-cli/src/commands/run.rs`**
        - Remove the pre-resolution of the engine and the `is_known`
          check; pass `args.engine` to `CreateOptions`.
        - Remove the now-unused `crate::adapter` import.
    - **Modify `crates/dosker-cli/tests/cli_lifecycle.rs`**
        - Update `create_rejects_an_unknown_engine` to call
          `root.import_image()` before `create --engine nope`, so the
          engine check runs after the image loads and the exit code stays
          2. Alternatively, remove the test in favor of the
          `cli_create::create_rejects_an_unknown_engine_string` case.
    - **Verify:** `cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_create && cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_lifecycle`

- [ ] **Task 10: Enforce `REQUIRES` in `create`, `run`, and `dosker build`**
    - **Implements:** FR-016; design §Error Handling; doskerfile
      §REQUIRES.
    - **Create `crates/dosker-cli/tests/cli_requires.rs`**
        - Add `mod common;` and the `#[cfg(unix)]` items the helper uses.
        - `create_rejects_a_required_capability_the_engine_lacks` — an
          image pinned to `dosbox-x` with `REQUIRES gdbstub` exits 2
          with `engine dosbox-x does not provide the gdbstub capability;
          the qemu engine provides it`.
        - `create_accepts_a_required_capability_the_engine_declares` —
          `REQUIRES headless` on `dosbox-x` exits 0.
        - `run_rejects_a_required_capability_before_the_disk_is_created`
          — `run -d` exits 2 and the state store has no record.
        - `create_rejects_an_unknown_required_capability` — an unknown
          name exits 2 with `unknown capability: {name}`.
    - **Create `crates/dosker-build/tests/build_requires.rs`**
        - `build_rejects_a_missing_required_capability` — a Doskerfile
          with `ENGINE qemu` and `REQUIRES audio` fails with
          `engine qemu does not provide the audio capability; no engine
          provides it`, before any step runs.
        - `build_accepts_a_required_capability` — `REQUIRES gdbstub`
          passes the check.
        - `build_checks_requires_before_a_run_step` — a `RUN` before a
          later `REQUIRES audio` still fails before the VM starts. Use
          `Builder::with_in_dos_runner` with a fake `InDosRunner` that
          records a call, and assert the runner was not called.
    - **Modify `crates/dosker-cli/src/commands/create.rs`**
        - Add `check_requires(engine, requires)` — resolves
          `adapter::declared_capabilities(engine)`, calls
          `dosker_engine::check_requires(engine, capabilities, requires)`,
          and maps the error to `CliError::Usage`.
        - Call `check_requires` after engine resolution and image load,
          before `validate_binds`, `resolve_volumes`, and
          `resolve_ports`.
        - `validate_binds` uses
          `dosker_engine::capability_message(engine,
          Capability::HostDirLive)`.
        - `validate_network` keeps its `CapabilitySet` parameter and
          adds the engine name only for
          `dosker_engine::capability_message(engine, Capability::Nat)`.
    - **Modify `crates/dosker-cli/src/commands/create.rs` tests**
        - The `validate_network` tests pass the engine name and a
          `CapabilitySet`; the missing-NAT assertion contains `nat` and
          the provider name.
    - **Modify `crates/dosker-cli/tests/cli_binds.rs`**
        - Keep the `host_dir_live` assertion; it now names the providers.
    - **Modify `crates/dosker-build/src/error.rs`**
        - Add `Capability { message: String }` with `#[error("{message}")]`.
    - **Modify `crates/dosker-build/src/build.rs`**
        - After `final_engine` and before the step loop, collect
          `base.config.requires` plus every `InstructionKind::Requires`
          from the Doskerfile, merge and sort/dedup, and resolve the
          final engine's declared set. A `None` returns
          `BuildError::Engine { engine, reason: "unsupported engine" }`.
        - Call `dosker_engine::check_requires(final_engine.as_str(),
          capabilities, &requires)` and map a failure to
          `BuildError::Capability { message }`.
        - The check uses the final engine for every requirement; it
          never checks a requirement against the engine in effect at its
          own step.
    - **Verify:** `cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_requires && cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_binds && cargo test -p dosker-cli --features qemu-engine,mock-engine --lib && cargo test -p dosker-build --test build_requires`

- [ ] **Task 11: Native debugger kind and `dosker debug`**
    - **Implements:** FR-011; design §Component Interfaces (CLI),
      §Error Handling; doskerfile §REQUIRES.
    - **Modify `crates/dosker-runtime/tests/protocol.rs`**
        - `parse_request_accepts_debug_native` — pins
          `{"type":"debug","kind":"native"}` to `DebugKind::Native`.
        - `response_round_trip_for_attach_cp_debug_screenshot` — includes
          `DebugKind::Native`.
    - **Modify `crates/dosker-engine-qemu/src/session.rs` test module**
        - `debug_endpoint_native_maps_to_the_gdbstub` — pins
          `DebugKind::Native` to the same endpoint as `DebugKind::Gdb`.
        - Keep `debug_endpoint_reports_unsupported_for_a_missing_gdb_path`.
    - **Modify `crates/dosker-engine-dosbox-x/src/session.rs` test
      module**
        - `debug_endpoint_returns_the_native_endpoint` — uses
          `DebugKind::Native`; returns early when `DebugNative` is not
          declared.
        - `debug_endpoint_rejects_gdb_without_a_gdbstub` — pins
          `EngineError::Unsupported("debugger")` for `DebugKind::Gdb`.
    - **Modify `crates/dosker-cli/tests/cli_debug.rs`**
        - `debug_reports_unsupported_for_the_mock_engine` — plain
          `debug` on a `mock` record exits 1. Assert the message
          contains `debug_native` and `qemu`; when the DOSBox-X table
          entry includes `DebugNative`, also assert `dosbox-x`. Do not
          assume DOSBox-X declares it.
        - `debug_rejects_gdb_when_the_engine_has_no_gdbstub` — a
          running `dosbox-x` record with `--gdb` exits 1 with `engine
          dosbox-x does not provide the gdbstub capability; the qemu
          engine provides it`.
        - `debug_native_prints_the_endpoint` — returns early when
          `DebugNative` is not declared; otherwise a scripted control
          endpoint replies with a debug endpoint and the output contains
          `native debugger`.
        - `debug_gdb_prints_a_target_command` — seeds a `qemu` record
          and keeps the `target remote` output.
        - `debug_rejects_a_stopped_container` — keeps the exit code.
        - Change `seed_running` to take an engine name and bind the
          control endpoint with `dosker_platform::control::bind`. Return
          the listener to the caller so it stays alive for the test; the
          scripted tests serve the reply from it, and the rejection tests
          hold it without serving. Without a live endpoint,
          `recover_stale` marks the record exited and the rejection tests
          get exit 125 instead of exit 1.
        - Mark `debug_gdb_prints_a_target_command`,
          `debug_native_prints_the_endpoint`, and
          `debug_rejects_gdb_when_the_engine_has_no_gdbstub`
          `#[cfg(unix)]`; they use `seed_running`.
        - Keep `debug_reports_unsupported_for_the_mock_engine` on
          `root.start()` so it stays cross-platform; do not use
          `seed_running` there.
        - The gdb test seeds `qemu`, and the native test seeds
          `dosbox-x`.
    - **Modify `crates/dosker-engine/src/spec.rs`**
        - `DebugKind` gains `Native`; `#[serde(rename_all =
          "snake_case")]` serializes it as `"native"`.
    - **Modify `crates/dosker-engine-qemu/src/session.rs`**
        - `debug_endpoint` maps both `DebugKind::Gdb` and
          `DebugKind::Native` to the gdb endpoint.
    - **Modify `crates/dosker-engine-dosbox-x/src/session.rs`**
        - `debug_endpoint` maps `DebugKind::Native` to the native
          endpoint and returns `Unsupported("debugger")` for
          `DebugKind::Gdb`.
    - **Modify `crates/dosker-cli/src/commands/debug.rs`**
        - Resolve the record and recover stale.
        - Check the status; a non-running container returns
          `EXIT_NOT_RUNNING`.
        - Check the control endpoint; a missing endpoint returns
          `EXIT_NOT_RUNNING`.
        - Choose the kind: `DebugKind::Native` for plain `debug`, and
          `DebugKind::Gdb` for `--gdb`.
        - Check the capability after the status and endpoint checks,
          using `adapter::declared_capabilities(&record.engine)`:
          `Capability::DebugNative` for `Native`, and
          `Capability::GdbStub` for `Gdb`. A missing capability returns
          `CliError::Message(dosker_engine::capability_message(
          &record.engine, capability))`.
        - Send `Request::Debug { kind }`.
        - Print `target remote {endpoint}` for `Gdb` and
          `native debugger {endpoint}` for `Native`.
        - Replace the hardcoded QEMU reject message with
          `dosker_engine::capability_message`.
    - **Verify:** `cargo test -p dosker-runtime --test protocol && cargo test -p dosker-engine-qemu && cargo test -p dosker-engine-dosbox-x && cargo test -p dosker-cli --features qemu-engine,mock-engine --test cli_debug`

- [ ] **Task 12: Document the DOSBox-X capability set and engine behavior**
    - **Implements:** FR-011, FR-016; design §Engine Capability Matrix,
      §Error Handling, §Testing Strategy.
    - **Create `docs/engines.md`**
        - `## Engine selection` — `--engine`, `ENGINE` pin, default
          QEMU.
        - `## ENGINE-CONFIG` — QEMU splits the raw line into arguments
          with shell-like quoting; DOSBox-X appends raw lines verbatim
          before `[autoexec]`; a line may be a `[section]` header or a
          `key=value` pair.
        - `## REQUIRES` — enforced by `create`, `run`, and `dosker
          build` after engine selection and before any VM starts. The
          final selected engine must satisfy every `REQUIRES` in the
          Doskerfile; a requirement is not checked against the engine in
          effect at its own step. Include the exact provider message
          forms.
        - `## Capability table` — QEMU and DOSBox-X columns. The
          DOSBox-X column is `yes` for a capability the installed build
          provides, `unproven` for a capability that is not confirmed,
          and `no` for `gdbstub`, `screenshot`, and `vnc`, which DOSBox-X
          does not provide.
        - `## Machine` — DOSBox-X uses fixed default machine keys and
          does not apply `MACHINE` overrides.
        - `## Discovery` — `DOSKER_DOSBOX_X`,
          `DOSKER_DOSBOX_X_VERSION`, `DOSKER_DOSBOX_X_SHA256`, and the
          official-build source.
        - `## Reporting` — the `dosker engines` and `doctor` output and
          exit rules.
        - A `REQUIRES headless` example and an `ENGINE-CONFIG dosbox-x`
          example.
        - A statement that in-DOS `RUN` and the build path stay
          QEMU-only.
        - A separate sentence on DOSBox-X host directories; do not
          change the `doctor` warning quote.
        - A statement that an image with `MACHINE net=ne2000` is
          rejected on DOSBox-X when `nat` is not provided.
    - **Modify `docs/builds.md`**
        - Replace the "Engine support" paragraph: `qemu` is the only
          engine for in-DOS `RUN` and the build path. `dosbox-x` is
          available for `run` and `create` with a prepared image.
          `bochs` is not implemented. A `RUN` on a DOSBox-X image fails
          with a message naming `qemu`.
        - State that the final engine must satisfy every `REQUIRES` and
          that the check runs before the step loop.
    - **Modify `docs/volumes-and-network.md`**
        - Replace the `--bind` QEMU-only sentence with: `--bind` needs
          an engine that declares `host_dir_live`. QEMU declares it.
          DOSBox-X declares it when the installed build provides the host
          directory mechanism; otherwise the capability gate rejects
          `--bind` on DOSBox-X.
        - Replace the NAT QEMU-only sentence with: NAT needs an engine
          that declares `nat`. QEMU declares it. DOSBox-X declares it
          when the installed build provides the NAT backend; otherwise
          the capability gate rejects `-p` on DOSBox-X. An image with
          `MACHINE net=ne2000` is rejected on DOSBox-X when `nat` is not
          provided.
        - Keep the `doctor` warning quote unchanged.
    - **Modify `README.md`**
        - Add an `### Engines` section after `### Images` that links
          `docs/engines.md` and says DOSBox-X is a runtime engine for the
          capabilities listed in the capability table.
    - **Verify:** `test -f docs/engines.md && rg -q "^## Engine selection" docs/engines.md && rg -q "^## Capability table" docs/engines.md && rg -q "^## Machine" docs/engines.md && rg -q "^## Discovery" docs/engines.md && rg -q "^## Reporting" docs/engines.md && rg -q "DOSKER_DOSBOX_X_SHA256" docs/engines.md && rg -q "final selected engine must satisfy every" docs/engines.md && rg -q "docs/engines.md" README.md && ! rg -q "is QEMU-only" docs/volumes-and-network.md && ! rg -q "NAT is a QEMU capability" docs/volumes-and-network.md && rg -q "needs an engine that declares .host_dir_live." docs/volumes-and-network.md && rg -q "needs an engine that declares .nat." docs/volumes-and-network.md && rg -q "development mode: --bind uses QEMU vvfat .beta. and can corrupt host files. Use it for development only, never in CI." docs/volumes-and-network.md && rg -q "build path" docs/builds.md`

- [ ] **Task 13: Real DOSBox-X runtime integration test**
    - **Implements:** FR-011; design §Testing Strategy, SC-003; roadmap
      `Done when`.
    - **Modify `crates/dosker-test-support/src/fixtures.rs`**
        - `DOSBOX_X_IMAGE_ENV = "DOSKER_DOSBOX_X_IMAGE"`.
        - `DOSBOX_X_NET_IMAGE_ENV = "DOSKER_DOSBOX_X_NET_IMAGE"`.
        - `dosbox_x_image()` and `dosbox_x_net_image()` — resolve the
          variables against the workspace root and require a regular
          file.
    - **Create `crates/dosker-cli/tests/dosbox_x_runtime.rs`**
        - Put `#![cfg(unix)]` first, then `mod common;`.
        - Add a module-level `INTEGRATION_LOCK` to serialize the tests.
        - `dosbox_x_boots_the_prepared_image_and_answers_exec` — gates
          on `DOSKER_INTEGRATION=1` and `DOSKER_DOSBOX_X_IMAGE`; imports
          the prepared image with `import_image_for_dosbox_x`, runs
          `run -d --engine dosbox-x --boot-timeout 120`, runs
          `exec <id> ECHO HELLO`, asserts `HELLO` and status 0, and runs
          `stop`.
        - `dosbox_x_reports_its_capabilities` — gates on
          `DOSKER_INTEGRATION=1` only; pins the `dosker engines`
          DOSBox-X line to the installed build's capabilities.
        - `dosbox_x_rejects_a_required_capability_it_does_not_declare` —
          gates on `DOSKER_INTEGRATION=1` only; generates a disk image
          with `make_disk_image`, imports it with
          `import_image_with_config` and `REQUIRES gdbstub`, and asserts
          the create/run failure names `qemu`. It does not need
          `DOSKER_DOSBOX_X_IMAGE`.
        - `dosbox_x_binds_a_live_host_directory` — write this test only
          when `HostDirLive` is proven. Import the image with
          `import_image_for_dosbox_x`, start a container with
          `--bind D:<host>`, run `exec TYPE D:\HOST.TXT`, edit the host
          file, run `exec TYPE D:\HOST.TXT` again, and assert the new
          content.
        - `dosbox_x_exposes_a_native_debugger_endpoint` — write this
          test only when `DebugNative` is proven. Import the image with
          `import_image_for_dosbox_x`, start a container, run plain
          `debug <id>`, parse the printed endpoint, connect to it, and
          send the sample debugger command recorded in the installed-build
          result.
        - `dosbox_x_forwards_a_published_port` — write this test only
          when `Nat` is proven. Import the net image with
          `machine.net = "ne2000"`, start it with `-p 8080:80`, request
          `http://127.0.0.1:8080/` from the host, and assert the
          response. Supply the net image through
          `DOSKER_DOSBOX_X_NET_IMAGE`.
        - If a proven optional capability cannot be tested on the
          available host, do not write its test; the docs table marks the
          cell `unproven` and the reason is recorded with the
          installed-build result.
    - **Verify:** `DOSKER_INTEGRATION=1 DOSKER_DOSBOX_X=dosbox-x DOSKER_DOSBOX_X_IMAGE=agent/fixtures/dosker-agent.img DOSKER_DOSBOX_X_NET_IMAGE=agent/fixtures/dosker-net.img cargo test -p dosker-cli --features qemu-engine --test dosbox_x_runtime -- --nocapture`
