# Development Mode Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-19
- **Status**: Approved
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Roadmap**: `.sdd/specs/2026-11-01-dosker/roadmap.md` — slice: development-mode
- **Depends on**: Slices 3 and 9.
- **Manual prerequisites**: QEMU. A host filesystem for the vvfat test.
- **Goal**: `--bind D:/host/path[:ro]` mounts a validated host directory into a QEMU guest, live through `vvfat` by default and through a two-way FAT-image sync when `vvfat` is disabled, with a read-only guard and a beta warning in `doctor` and the docs.

## Constraints

- The CLI form is `dosker run ... [--bind drive:path[:ro]] IMAGE`; `create` accepts the same flag. The canonical value is `D:/host/path`; `:ro` is a literal suffix.
- `--bind` accepts `D:`, `E:`, and `F:` only. `C:` is the boot disk; `A:` and `B:` are floppy drives. Any other drive is a usage error (exit `2`) naming the supported set.
- The parser accepts a lowercase drive and an optional drive colon before the path; it uppercases the drive and keeps the path as written. A missing path or a missing drive separator is a usage error.
- The host source must exist, be a directory, and contain only entries whose names are valid 8.3 after uppercase normalization. Symlinks, non-regular entries, and names that collide after normalization are refused before boot; the error names the offending host path.
- A bind mount is recorded as `VolumeKind::Bind` with `name: None`, `host_path: Some(<absolute host path>)`, and the parsed `read_only` flag. `ContainerRecord` and its schema version do not change.
- A bind drive must not collide with a `-v` drive, another bind, or an image `VOLUME` declaration. Collisions are usage errors.
- QEMU declares `Capability::HostDirLive` (stable name `host_dir_live`). An engine without it rejects a non-empty `--bind` list with the usage error `engine does not provide the host_dir_live capability; --bind needs the QEMU engine`. `--bind` is QEMU-only in this slice.
- QEMU `vvfat` is the default. `DOSKER_NO_VVFAT=1` selects the sync fallback; only the exact value `1` disables `vvfat`.
- A host-directory engine volume is attached as `file=fat:rw:<dir>,format=raw,if=ide,index=<letter - 'C'>`. It is always read-write: QEMU rejects a read-only vvfat node on an IDE disk with `Block node is read-only`, so a read-only bind uses the fallback image instead. `D:`, `E:`, and `F:` map to IDE index `1`, `2`, and `3`. A host path containing a comma is rejected with `EngineError::Setup` because the comma separates QEMU drive options.
- The fallback image is `containers/<id>/bind-<lowercase drive letter>.img`, exactly `67_108_864` bytes (64 MiB). It is created with `create_volume` and projected with `project_tree`.
- The fallback is two-way: at start the host tree is projected into the image and the image is attached; on a clean stop the guest tree is read with `read_tree` and mirrored back with `write_host_tree`; the image is then removed. A failed or killed stop removes the image without writing back. A failed write-back logs a warning and keeps the image.
- The fallback writes names in uppercase 8.3 form and mirrors the guest tree, including deletions. A host directory whose file bytes exceed `67_108_864` fails to start with an error that names the host directory.
- A read-only bind always uses the fallback image, attached with the existing read-only IDE overlay (`snapshot=on`). The guest writes are discarded and the host directory is never modified. `vvfat` is used only for a read-write bind.
- Host-projected entries use the fixed timestamp `1980-01-01T00:00:00Z` (`dosker_image::fixed_timestamp()`).
- `doctor` prints `development mode: --bind uses QEMU vvfat (beta) and can corrupt host files. Use it for development only, never in CI.` after the capability line.
- No CI or nightly workflow change. The QEMU integration tests are gated by `DOSKER_INTEGRATION=1` and run locally.
- Out of scope: a shared L2 network and any engine other than QEMU.

## Interfaces

These signatures are shared across tasks; later tasks consume exactly what
earlier tasks define.

```rust
// dosker-image
pub enum ImageError {
    // existing variants ...
    InvalidHostPath { path: PathBuf, reason: String }, // "invalid host path {path}: {reason}"
}
pub fn validate_host_tree(root: &Path) -> Result<(), ImageError>;
pub fn read_host_tree(root: &Path) -> Result<Tree, ImageError>;
pub fn write_host_tree(root: &Path, tree: &Tree) -> Result<(), ImageError>;

// dosker-engine
pub enum EngineVolumeKind { Image, HostDir }   // Clone, Copy, Debug, PartialEq, Eq
pub struct EngineVolume {
    pub drive: String,
    pub path: PathBuf,        // disk image for Image; host directory for HostDir
    pub read_only: bool,
    pub kind: EngineVolumeKind,
}

// dosker-runtime
pub const BIND_IMAGE_SIZE: u64 = 67_108_864;
pub const VVFAT_DISABLED_ENV: &str = "DOSKER_NO_VVFAT";
pub enum BindError {
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error("bind mount {drive} has no host path")]
    MissingHostPath { drive: String },
}
pub fn vvfat_disabled(value: Option<&str>) -> bool;
pub fn use_vvfat() -> bool;
pub fn prepare_binds(
    record: &ContainerRecord,
    paths: &RuntimePaths,
    use_vvfat: bool,
) -> Result<Vec<EngineVolume>, BindError>;
pub fn sync_binds(record: &ContainerRecord, paths: &RuntimePaths) -> Result<(), BindError>;
pub fn discard_binds(record: &ContainerRecord, paths: &RuntimePaths);
impl RuntimePaths {
    pub fn bind_image(&self, id: &ContainerId, drive: &str) -> PathBuf;
}

// dosker-cli
pub struct BindSpec {
    pub drive: String,
    pub path: PathBuf,
    pub read_only: bool,
}
impl BindSpec {
    pub fn parse(value: &str) -> Result<Self, SpecError>;
}
pub enum SpecError {
    // existing variants ...
    Bind { value: String, reason: String }, // "invalid bind mapping {value}: {reason}"
}
pub struct CreateOptions {
    // existing fields ...
    pub binds: Vec<String>,
}
fn validate_binds(
    image: &Image,
    engine: &str,
    volumes: &[VolumeSpec],
    binds: &[BindSpec],
) -> Result<Vec<VolumeMount>, CliError>;
```

## Tasks

- [ ] **Task 1: Host directory 8.3 validation**
    - **Implements:** FR-017; design §Error Handling ("Invalid 8.3 path", "Symlink or traversal"), data-model §Validation Rules ("A bind mount source must exist and pass 8.3 validation").
    - **Create `crates/dosker-image/tests/host.rs` first**
        - `a_valid_host_directory_becomes_a_tree` — `read_host_tree` on a directory with `APP/MAIN.EXE` and `README.TXT` produces the paths `APP`, `APP/MAIN.EXE`, and `README.TXT`, with the fixed timestamp and the exact file bytes.
        - `a_valid_host_directory_passes_validation` — `validate_host_tree` returns `Ok(())` for the same directory.
        - `a_name_longer_than_eight_characters_is_rejected_and_names_the_path` — `validate_host_tree` on `TOOLONGNAME.TXT` fails with an error whose display contains the offending host path and `TOOLONGNAME.TXT`.
        - `an_extension_longer_than_three_characters_is_rejected` — `MAIN.LONG`.
        - `a_name_with_an_invalid_character_is_rejected` — `BAD?NAME.TXT`.
        - `a_leading_period_is_rejected` — `.HIDDEN`.
        - `a_symlinked_file_is_rejected` — `#[cfg(unix)]`; a symlink entry fails and names the symlink path.
        - `a_symlinked_directory_is_rejected` — `#[cfg(unix)]`.
        - `a_symlinked_root_is_rejected` — `#[cfg(unix)]`.
        - `a_non_directory_source_is_rejected` — passing a regular file path fails and names it.
        - `a_missing_source_is_rejected` — a missing path fails and names it.
        - `names_are_compared_after_normalization` — `#[cfg(target_os = "linux")]`; `FOO.TXT` and `foo.txt` in one directory fail with an error that names both host paths and contains `duplicate`.
    - **Create `crates/dosker-image/src/host.rs`**
        - `pub fn validate_host_tree(root: &Path) -> Result<(), ImageError>` — run the shared walk with `read_contents` false and discard the tree; this is the check `create` uses, so it never reads file bytes.
        - `pub fn read_host_tree(root: &Path) -> Result<Tree, ImageError>` — run the shared walk with `read_contents` true and return the tree.
        - Private `walk(root, read_contents)` — read the root with `symlink_metadata`; require a directory and reject a symlink; walk recursively with `read_dir`; reject a symlink or a non-file/non-directory entry; normalize each name with `DosName::normalize`; join the component onto a `DosPath`; record each host path in a `BTreeMap<DosPath, PathBuf>` and reject a second entry that normalizes to an existing path with reason `duplicate of <first host path>`; read file bytes only when `read_contents` is true and otherwise record empty bytes; build a `LayerDelta` of `TreeDir` and `TreeFile` with `Attributes::default()` and `fixed_timestamp()`; apply it to a `Tree`.
        - Private `invalid_host_path` helper.
    - **Modify `crates/dosker-image/src/error.rs`**
        - `ImageError::InvalidHostPath { path: PathBuf, reason: String }` with the display `invalid host path {path}: {reason}`.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - `pub mod host;` and re-export `validate_host_tree` and `read_host_tree`.
    - **Verify:** `cargo test -p dosker-image --test host`

- [ ] **Task 2: Host tree write-back**
    - **Implements:** design §Risks ("Sync fallback"), §Runtime (the fallback writes guest changes back on a clean stop).
    - **Modify `crates/dosker-image/tests/host.rs` first**
        - `write_host_tree_mirrors_the_tree` — a destination containing `STALE.TXT` and `STALEDIR/OLD.TXT` gains the tree's files and directories, and the stale entries are gone.
        - `write_host_tree_writes_the_exact_bytes` — a nested `APP/MAIN.EXE` is created with the tree bytes.
        - `write_host_tree_replaces_a_stale_symlink_without_following_it` — `#[cfg(unix)]`; a destination symlink at a tree file path is replaced by a regular file and the symlink target is unchanged.
        - `write_host_tree_replaces_a_file_where_the_tree_has_a_directory` — a destination regular file at a tree directory path is replaced by a directory.
    - **Modify `crates/dosker-image/src/host.rs`**
        - `pub fn write_host_tree(root: &Path, tree: &Tree) -> Result<(), ImageError>` — build the expected set of relative paths from `tree.dirs()` and `tree.files()` plus every ancestor directory of those paths; walk the destination with `symlink_metadata` and remove every entry not in the set (directories with `remove_dir_all`, files and symlinks with `remove_file`) without following symlinks; for each tree directory remove an existing non-directory entry at the path and create it with `create_dir_all`; for each file remove an existing entry at the path (including a directory) and write the bytes; do not set attributes or timestamps.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Re-export `write_host_tree`.
    - **Verify:** `cargo test -p dosker-image --test host`

- [ ] **Task 3: Bind value parser**
    - **Implements:** FR-017; design §CLI (`dosker run ... [--bind drive:path[:ro]]`).
    - **Modify `crates/dosker-cli/src/spec.rs` with tests first**
        - `bind_spec_accepts_the_canonical_form` — `D:/host/path` → drive `D:`, path `/host/path`, `read_only` false.
        - `bind_spec_normalizes_a_lowercase_drive` — `d:/host/path` → `D:`.
        - `bind_spec_parses_read_only` — `D:/host/path:ro`.
        - `bind_spec_accepts_the_other_supported_drives` — `E:` and `F:`.
        - `bind_spec_rejects_the_boot_drive` — `C:/host`; the message contains `D:, E:, and F:`.
        - `bind_spec_rejects_a_floppy_drive` — `A:/host` and `B:/host`; the messages contain the supported set.
        - `bind_spec_rejects_a_drive_outside_the_supported_set` — `G:/host`; the message contains `D:, E:, and F:`.
        - `bind_spec_rejects_a_missing_path` — `D:`.
        - `bind_spec_rejects_a_missing_drive_separator` — `/host/path`.
        - `bind_spec_keeps_a_windows_style_path` — `D:C:\host\path` → path `C:\host\path`.
    - **Modify `crates/dosker-cli/src/spec.rs`**
        - `BindSpec` and `BindSpec::parse` — strip a literal `:ro` suffix; split at the first `:`; require a one-letter drive and a non-empty path; uppercase the drive; accept only `D`, `E`, and `F`; return `D:`-style drives and the path as written.
        - `SpecError::Bind { value, reason }` displaying `invalid bind mapping {value}: {reason}`.
        - Drive messages: `the boot drive C: cannot hold a bind mount; supported drives are D:, E:, and F:`, `the floppy drive {drive} cannot hold a bind mount; supported drives are D:, E:, and F:`, and `unsupported bind drive {drive}; supported drives are D:, E:, and F:`.
    - **Verify:** `cargo test -p dosker-cli --lib spec`

- [ ] **Task 4: QEMU declares the capability and attaches vvfat directories**
    - **Implements:** FR-015 (the capability gate); design §Engine Capability Matrix (`host_dir_live`: QEMU, `vvfat`), §Runtime.
    - **Modify `crates/dosker-engine-qemu/tests/command.rs` first**
        - `capabilities_are_the_proven_set` — the expected set gains `Capability::HostDirLive` after `Headless`.
        - `capabilities_include_host_dir_live` — the adapter declares it.
        - `command_attaches_a_host_directory_as_vvfat` — a `D:` host-directory volume with `read_only` false gives the `-drive` value `file=fat:rw:<dir>,format=raw,if=ide,index=1`; the value contains no `readonly=on` or `snapshot=on`.
        - `command_rejects_a_read_only_host_directory` — a `HostDir` volume with `read_only` true returns `EngineError::Setup`.
        - `command_rejects_a_host_directory_with_a_comma` — a host path containing `,` returns `EngineError::Setup`.
        - `command_attaches_a_host_directory_on_e_and_f` — `E:` gives `index=2`, `F:` gives `index=3`.
        - `command_rejects_a_host_directory_on_a_floppy_drive` — `A:` and `B:` return `EngineError::Setup`.
        - The `volume` helper gains `kind: EngineVolumeKind::Image` so the existing image-volume tests keep compiling.
    - **Modify `crates/dosker-cli/tests/cli_engines.rs` first**
        - `CAPABILITIES` becomes `boot,serial,stop,headless,host_dir_live,debug_native,gdbstub,screenshot,vnc,nat`.
        - `engines_lists_the_proven_capabilities` asserts `host_dir_live` is present instead of absent.
    - **Modify `crates/dosker-engine/src/spec.rs`**
        - `EngineVolumeKind` as pinned; `EngineVolume` gains `pub kind: EngineVolumeKind`.
    - **Modify `crates/dosker-engine/src/lib.rs`**
        - Re-export `EngineVolumeKind`.
    - **Modify `crates/dosker-engine-qemu/src/command.rs`**
        - `volume_drives` branches on `volume.kind`: a `HostDir` volume on `A:`/`B:`, a `HostDir` volume with `read_only` true, or a `HostDir` path containing `,` is `EngineError::Setup`; otherwise the drive value is `file=fat:rw:{path},format=raw,{placement}` with the existing IDE placement; a `HostDir` volume never gets `readonly=on` or `snapshot=on`. An `Image` volume keeps the existing `format=raw` and read-only overlay behavior.
    - **Modify `crates/dosker-engine-qemu/src/adapter.rs`**
        - Add `Capability::HostDirLive` to `capabilities()` after `Headless`.
    - **Modify `crates/dosker-runtime/src/supervisor.rs` and `crates/dosker-runtime/tests/supervisor.rs`**
        - Add `kind: EngineVolumeKind::Image` to the existing `EngineVolume` literals.
    - **Verify:** `cargo test -p dosker-engine-qemu --test command && cargo test -p dosker-cli --features qemu-engine --test cli_engines`

- [ ] **Task 5: `create` records bind mounts and gates the capability**
    - **Implements:** FR-015 (the capability gate), FR-017; design §CLI, §Runtime, data-model §VolumeMount; the binding decisions on the supported drives and the QEMU-only gate.
    - **Create `crates/dosker-cli/tests/cli_binds.rs` first**
        - Every test imports `freedos:latest` through `CliRoot::import_image_as` before `create`; the image-declaration test uses a local `build_image` helper mirroring `cli_volumes.rs`.
        - `create_records_a_bind_mount` — `create --engine qemu --bind D:<dir> freedos:latest`; `inspect` shows one mount with `kind` `bind`, `name` null, `host_path` equal to the absolute host directory, and `read_only` false.
        - `create_records_a_read_only_bind` — `--bind D:<dir>:ro` records `read_only` true.
        - `create_records_the_host_path_as_absolute` — the recorded `host_path` is absolute.
        - `create_rejects_a_bind_on_the_mock_engine` — `create --engine mock --bind D:<dir>` exits `2`; the message contains `host_dir_live`.
        - `create_rejects_a_bind_on_the_boot_drive` — `--bind C:<dir>`; exit `2`; the message contains `D:, E:, and F:`.
        - `create_rejects_a_bind_on_a_floppy_drive` — `A:` and `B:`; exit `2`.
        - `create_rejects_a_bind_outside_the_supported_set` — `G:`; exit `2`.
        - `create_rejects_a_bind_that_collides_with_a_volume_drive` — seed `data`, `-v data:D:` and `--bind D:<dir>`; exit `2`; the message contains `mounted more than once`.
        - `create_rejects_a_bind_on_a_drive_declared_by_the_image` — build an image with `VOLUME D:` and use `--bind D:<dir>`; exit `2`; the message contains `declared by the image`; no `anon-` volume exists afterwards.
        - `create_rejects_a_duplicate_bind_drive` — two `--bind D:` values; exit `2`.
        - `create_rejects_an_invalid_8_3_name_and_names_the_path` — a host directory with `TOOLONGNAME.TXT`; exit `2`; the message contains the host path and the name.
        - `create_rejects_a_missing_host_directory` — exit `2`; the message contains the path.
        - `create_rejects_a_symlinked_host_entry` — `#[cfg(unix)]`; exit `2`; the message names the symlink.
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - `RunArgs` and `CreateArgs` gain `#[arg(long = "bind", value_name = "drive:path[:ro]")] pub bind: Vec<String>`.
    - **Modify `crates/dosker-cli/src/commands/create.rs`**
        - `CreateOptions` gains `pub binds: Vec<String>`; `execute` copies `args.bind`.
        - `create_container` parses `VolumeSpec` and `BindSpec`, then calls `validate_binds(&image, &options.engine, &specs, &binds)?` before `resolve_volumes`, then extends the named mounts with the bind mounts.
        - `validate_binds` — an empty list returns an empty vector; resolve `adapter::declared_capabilities(engine)` and reject a non-empty list without `Capability::HostDirLive` with the pinned message; seed the drive set from the `-v` specs; reject a duplicate drive with `drive {drive} is mounted more than once`; reject a drive in `image.volumes` with `drive {drive} is declared by the image and cannot hold a bind mount`; resolve `std::path::absolute(&bind.path)` and map a failure to `CliError::Usage(format!("bind source {}: {error}", bind.path.display()))`; call `validate_host_tree(&host)` and map every error to `CliError::Usage(error.to_string())`; push a `VolumeMount` with `VolumeKind::Bind`, `name: None`, `host_path: Some(host)`, and the parsed `read_only`.
    - **Modify `crates/dosker-cli/src/commands/run.rs`**
        - Pass `binds: args.bind` in `CreateOptions`.
    - **Verify:** `cargo test -p dosker-cli --features mock-engine --test cli_binds`

- [ ] **Task 6: Runtime bind preparation, fallback projection, sync, and discard**
    - **Implements:** FR-015 (the capability gate); design §Risks ("Sync fallback"), §Runtime; the binding decisions on `DOSKER_NO_VVFAT=1` and the two-way fallback.
    - **Create `crates/dosker-runtime/tests/binds.rs` first**
        - The record helper creates a `ContainerRecord` with one `VolumeKind::Bind` mount and creates `containers/<id>` under the test root before `prepare_binds`.
        - `vvfat_is_the_default_and_only_the_exact_value_one_disables_it` — `vvfat_disabled(None)`, `Some("0")`, and `Some("true")` are false; `Some("1")` is true.
        - `prepare_binds_uses_the_host_directory_for_vvfat` — `use_vvfat` true gives one `EngineVolume` with `kind` `HostDir`, `path` equal to the host directory, and the recorded `read_only`; no `bind-d.img` exists.
        - `prepare_binds_projects_the_host_directory_into_a_fallback_image` — `use_vvfat` false gives `kind` `Image` and `path` equal to `<root>/containers/<id>/bind-d.img`; the image exists and `read_tree` contains `HOST.TXT` with the host bytes.
        - `prepare_binds_uses_a_read_only_fallback_for_a_read_only_bind` — with `use_vvfat` true and a read-only mount, the result is `kind` `Image`, `read_only` true, and the image exists.
        - `prepare_binds_rejects_a_host_directory_larger_than_the_image` — a host directory whose file bytes exceed `BIND_IMAGE_SIZE` fails and names the host path.
        - `prepare_binds_rejects_an_invalid_host_name_and_names_the_path` — the error display contains the host path and the invalid name.
        - `prepare_binds_rejects_a_bind_without_a_host_path` — `BindError::MissingHostPath` names the drive.
        - `prepare_binds_removes_a_stale_fallback_image` — a pre-existing file at the image path is replaced with the host content.
        - `sync_binds_copies_a_guest_change_back_to_the_host` — after the fallback image is reprojected with a tree containing `FROMDOS.TXT` and no `HOST.TXT`, `sync_binds` leaves the host with `FROMDOS.TXT` and removes `HOST.TXT`.
        - `sync_binds_skips_a_read_only_mount` — a read-only fallback does not change the host.
        - `sync_binds_does_nothing_for_a_host_directory_bind` — a `vvfat` bind leaves the host unchanged.
        - `discard_binds_removes_the_fallback_image` — the image is gone and a `vvfat` bind is untouched.
    - **Modify `crates/dosker-runtime/src/paths.rs`**
        - `RuntimePaths::bind_image(id, drive)` returns `containers/<id>/bind-<lowercase drive letter>.img`.
    - **Create `crates/dosker-runtime/src/binds.rs`**
        - `BIND_IMAGE_SIZE = dosker_image::DEFAULT_VOLUME_SIZE` (`67_108_864`), `VVFAT_DISABLED_ENV`, `BindError` as pinned.
        - `vvfat_disabled(value)` and `use_vvfat()` — only the exact value `1` disables `vvfat`.
        - `prepare_binds(record, paths, use_vvfat)` — for each `VolumeKind::Bind` mount: require `host_path`; remove any existing `paths.bind_image` file first; use `HostDir` only when `use_vvfat` is true and the mount is read-write, calling `validate_host_tree` and pushing `EngineVolume { kind: HostDir, path: host, read_only: false }`; otherwise call `read_host_tree`, reject when the sum of the file bytes exceeds `BIND_IMAGE_SIZE` with an `InvalidHostPath` error that names the host path, call `create_volume(&image, BIND_IMAGE_SIZE, "bind")`, `project_tree(&image, &tree)`, and push `EngineVolume { kind: Image, path: image, read_only: mount.read_only }`; on any failure remove the fallback images this call created and return the error.
        - `sync_binds(record, paths)` — for each read-write bind whose `paths.bind_image` exists, `read_tree` it and `write_host_tree` the host directory; skip read-only mounts and `vvfat` mounts.
        - `discard_binds(record, paths)` — remove each `paths.bind_image` file, ignoring a missing file and any error.
    - **Modify `crates/dosker-runtime/src/lib.rs`**
        - `pub mod binds;` and re-export `BIND_IMAGE_SIZE`, `VVFAT_DISABLED_ENV`, `BindError`, `prepare_binds`, `sync_binds`, `discard_binds`, `use_vvfat`, `vvfat_disabled`.
    - **Verify:** `cargo test -p dosker-runtime --test binds`

- [ ] **Task 7: Supervisor attaches binds and syncs on a clean stop**
    - **Implements:** FR-015 (the capability gate); design §Runtime, §Risks; the binding decisions on the fallback selection and the discard rule.
    - **Modify `crates/dosker-runtime/tests/supervisor.rs` first**
        - Add a `bind_mount(drive, host, read_only)` helper and a `static BIND_ENV_LOCK: Mutex<()>`; all three bind tests hold it so the environment is stable.
        - `supervisor_passes_a_bind_directory_to_the_engine` — a temp host directory with `HOST.TXT`; the recorded spec contains `EngineVolume { drive: "D:", path: <host>, read_only: false, kind: HostDir }`.
        - `supervisor_projects_a_bind_directory_when_vvfat_is_disabled` — a temp host directory with `HOST.TXT`; hold `BIND_ENV_LOCK`, set `DOSKER_NO_VVFAT=1`, start a bind container, assert the recorded volume is `kind: Image` at `<root>/containers/<id>/bind-d.img` and the file exists; stop cleanly; assert the image is gone and the host file is unchanged; remove the variable.
        - `supervisor_discards_the_fallback_image_when_the_guest_crashes` — hold `BIND_ENV_LOCK`, set `DOSKER_NO_VVFAT=1`, kill the engine, join, and assert the fallback image is gone.
    - **Modify `crates/dosker-runtime/src/supervisor.rs`**
        - After the named-volume loop and before `write_config`, call `prepare_binds(record, paths, use_vvfat())`; a failure becomes `failed_start` with `EngineError::Setup(error.to_string())`; extend `engine_volumes` with the result.
        - `failed_start` calls `discard_binds(record, store.paths())`.
        - `mark_exited` calls `discard_binds(record, store.paths())`.
        - `run_stop` on a clean engine exit calls `sync_binds(&state.record, store.paths())`; on success calls `discard_binds`; on failure logs `warning: cannot write back a bind mount: {error}` and keeps the image.
    - **Verify:** `cargo test -p dosker-runtime --test supervisor`

- [ ] **Task 8: `doctor` warning**
    - **Implements:** design §Risks ("Prominent warning"), roadmap "Done when" (the warning appears in `doctor`).
    - **Modify `crates/dosker-cli/tests/cli_engines.rs` first**
        - `doctor_prints_the_development_mode_warning` — with the fake QEMU script from `write_qemu`, `doctor` exits `0` and stdout contains `--bind`, `vvfat`, and `development only`.
    - **Modify `crates/dosker-cli/src/commands/doctor.rs`**
        - Add `const DEVELOPMENT_MODE_WARNING: &str = "development mode: --bind uses QEMU vvfat (beta) and can corrupt host files. Use it for development only, never in CI.";`
        - Print it after the capability line and before the security statement.
    - **Verify:** `cargo test -p dosker-cli --features qemu-engine --test cli_engines`

- [ ] **Task 9: Development mode documentation**
    - **Implements:** design §Runtime ("with the documented beta warning and no CI use"), §Risks.
    - **Modify `docs/volumes-and-network.md`**
        - Add `## Development mode` after the volume-management commands and before `## Publishing ports`.
        - State the form `--bind drive:path[:ro]`, the canonical example `--bind D:/host/path`, and the drives `D:`, `E:`, and `F:` with `C:`, `A:`, and `B:` excluded.
        - State that the source must exist, be a directory, and contain only valid 8.3 names; symlinks and names that collide after uppercase normalization are refused before boot and the error names the host path.
        - State that QEMU attaches a read-write directory with `vvfat`, that QEMU documents `vvfat` as beta, that it can corrupt host files, that development mode is for development only and never for CI, and that `dosker doctor` prints this warning.
        - State that `DOSKER_NO_VVFAT=1` selects the sync fallback: a 64 MiB FAT image at start, a copy back on a clean stop, no write-back and image removal on a failed or killed stop, uppercase 8.3 names on write-back, and a 64 MiB source limit.
        - State that `:ro` always uses the fallback image, that guest writes are discarded through the read-only IDE overlay, and that the host directory is never modified.
        - State that the QEMU integration tests are gated by `DOSKER_INTEGRATION=1` and are not part of CI.
    - **Verify:** `rg -n "DOSKER_NO_VVFAT|vvfat|8\\.3|--bind" docs/volumes-and-network.md`

- [ ] **Task 10: Development mode integration tests on QEMU**
    - **Implements:** FR-015 (the capability gate), FR-017; roadmap "Done when" (host edit visible, invalid name refused before boot and named, read-only write rejected, fallback content).
    - **Create `crates/dosker-cli/tests/qemu_binds.rs`**
        - `#![cfg(unix)]`, `mod common;`, a `static INTEGRATION_LOCK: Mutex<()>`, a `prepared_image()` that returns `None` without `DOSKER_INTEGRATION=1` and panics when the prepared image is missing, a `Container` drop guard copied from `qemu_volumes.rs`, and a `run_with_vvfat_disabled` helper that adds `DOSKER_NO_VVFAT=1` to `root.command_with_qemu()` and uses `run_captured`.
        - `a_host_edit_is_visible_in_the_guest_without_a_rebuild` — a host directory with `HOST.TXT` containing `ONE\r\n`; `run -d --name bind --bind D:<dir> freedos:latest`; `exec bind TYPE D:\HOST.TXT` prints `ONE\r\n`; overwrite the host file with `TWO\r\n`; poll `exec bind TYPE D:\HOST.TXT` for up to 10 seconds until it prints `TWO\r\n`, then stop and remove.
        - `an_invalid_8_3_name_is_refused_before_boot_and_names_the_path` — a host directory with `TOOLONGNAME.TXT`; `run -d --name bad --bind D:<dir> freedos:latest` exits `2`; the combined output contains the host path and `TOOLONGNAME.TXT`; `ps` does not list `bad`.
        - `a_read_only_bind_rejects_a_guest_write` — a host directory with `HOST.TXT`; `run -d --name robind --bind D:<dir>:ro freedos:latest`; `exec robind TYPE D:\HOST.TXT` succeeds; `exec robind COPY C:\DOSKER\TWO.TXT D:\BLOCKED.TXT`; after the stop the host directory has no `BLOCKED.TXT` and `HOST.TXT` is unchanged.
        - `the_fallback_shows_the_same_content_when_vvfat_is_disabled` — a host directory with `HOST.TXT` containing `ONE\r\nTWO\r\n`; `run_with_vvfat_disabled` starts `sync` with `--bind D:<dir>`; parse the container id from the `run -d` stdout; `exec sync TYPE D:\HOST.TXT` prints `ONE\r\nTWO\r\n`; `exec sync COPY C:\DOSKER\TWO.TXT D:\FROMDOS.TXT`; `stop sync`; the host directory contains `FROMDOS.TXT` with `ONE\r\nTWO\r\n` and the file `containers/<id>/bind-d.img` does not exist.
        - Add an `exec_until` helper that repeats an `exec` until the output matches or a 10-second deadline expires.
        - Every test takes `INTEGRATION_LOCK` after `prepared_image()` and uses a unique container name.
    - **Do not modify `.github/workflows/ci.yml` or `.github/workflows/qemu-nightly.yml`.**
    - **Verify:** `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img DOSKER_QEMU=qemu-system-i386 cargo test -p dosker-cli --features qemu-engine --test qemu_binds -- --nocapture`
