# In-DOS RUN Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-15
- **Status**: In Progress
- **Design**: [`.sdd/specs/2026-11-01-dosker/design.md`](design.md)
- **Roadmap**: [`.sdd/specs/2026-11-01-dosker/roadmap.md`](roadmap.md) — slice: `in-dos-run`
- **Depends on**: Slices 2 (`dos-agent-serial`), 5 (`image-store-and-tree`), 6 (`doskerfile-and-host-build`), and 7 (`freedos-base-builder`), all Implemented
- **Manual prerequisites**: QEMU (`qemu-system-i386`) on `PATH` or through `DOSKER_QEMU`. The built agent at `agent/build/DOSKER.EXE` and the test helpers at `agent/build/SETBOOT.EXE` and `agent/build/REBOOT.EXE`, all built by `agent/build.sh`. A FreeDOS base with the agent: either a stored `freedos:1.3` or `DOSKER_FREEDOS_SOURCE` plus the pinned FreeDOS inputs from the base builder. For the integration suite: `DOSKER_INTEGRATION=1`, `DOSKER_QEMU_IMAGE` pointing at `agent/fixtures/dosker-agent.img`, that image containing `C:\DOSKER\EXITCODE.EXE`, and `DOSKER_FREEDOS_SOURCE` for the boot-metadata and guest-reboot cases.
- **Goal**: Execute `RUN` inside DOS against a projected bootable disk, read the tree and boot metadata back, commit the delta as a layer, cache the result, and fail the build without a layer on any command, boot, timeout, or guest-reboot failure.

## Constraints

- One transient VM per `RUN` instruction; consecutive-`RUN` batching is not implemented.
- The command timeout is `--timeout` on `dosker build`; its default is `300` seconds and it applies to the agent `EXEC`.
- The boot wait is `RUN_BOOT_TIMEOUT: Duration = Duration::from_secs(60)`; the agent shutdown grace is `RUN_STOP_GRACE: Duration = Duration::from_secs(10)`.
- The build VM runs headless with the engine selected at that step; only `qemu` is implemented. `dosbox-x` and `bochs` fail with `BuildError::Engine`.
- `RUN` requires a non-booter image with `agent_version` set and a raw-disk blob. A tree-only image with no raw disk is an error before the cache lookup.
- `apply_boot_metadata` writes the effective MBR first and locates the FAT region from that MBR; the projection then formats only that FAT volume and writes the tree. The MBR, partition table, boot sector, geometry, volume label, media descriptor, and disk size are preserved for an unchanged boot.
- The projection supports FAT12 and FAT16 with a 512-byte sector. FAT32, a sector size other than 512, a non-1 reserved-sector count, or a formatted BPB that cannot reproduce the source layout is `ImageError::Projection`.
- `PROJECTION_SECTOR_BYTES: usize = 512`.
- `C:\DOSKER\CONFIG.INI` is written before boot with exactly `CONTROL_PORT={control_port}\r\nBAUD={baud}\r\n`.
- `C:\DOSKER` and every path below it are excluded from the read-back delta; no layer entry may name that directory.
- Every `RUN` layer entry uses `1980-01-01T00:00:00Z`; files get the archive attribute and directories get no attribute, matching `RUN --host`.
- A layer's `boot` override is set only when `BootMetadata::boot_artifacts_equal` reports a difference.
- The `RUN` cache key is `sha256(parent_tree_digest || "\n" || build_config_digest || "\n" || normalized_instruction || "\n" || sorted(input_hashes))` with input hashes `staged:<parent tree digest>`, `engine:<engine version at that step>`, `agent:<agent version>`, and `disk:<raw disk digest>`.
- A cache hit boots no VM. A failed `RUN` commits no layer and writes no cache record.
- `RUN` output is reported after the agent `EXEC` returns. Empty output reports `RUN produced no captured output`; a truncated result reports `RUN output was truncated`.
- A non-zero exit, command timeout, boot failure, missing agent, or guest reboot fails the build. A guest reboot surfaces as `BuildError::GuestRebooted`.
- The build VM uses QEMU `-no-reboot`; a clean QEMU exit while the command runs is mapped to `GuestRebooted`. The runtime keeps its own reboot policy.
- The build VM passes the image disk interface and serial layout, and writes the effective control port and baud into `CONFIG.INI`. The QEMU adapter maps `serial_ports` serial ports and places the agent on `control_port`; the default is two ports with the agent on COM2. QEMU CPU, memory, video, sound, and network defaults stay as the adapter defines them.
- The build VM creates `<root>/containers/<id>` before `start` and, on Unix, when any of the four engine socket paths falls back for a long root, creates `dosker_platform::short_socket_dir()` with mode `0700` before `start`.
- The build VM has no NIC in this slice; a `RUN` must not depend on network access.
- No commit steps; the user commits.

## Rulings

- **Projection scope.** `project_tree` formats the existing FAT volume using the source BPB and restores the original boot sector byte-for-byte. It never touches the MBR, the partition gap, or the bytes after the volume. `apply_boot_metadata` separately writes the effective MBR and boot sector over the private disk before projection. A source whose BPB would be changed by formatting is rejected rather than silently rewritten.
- **Partition overrides.** A `RUN` may change the partition bounds. The layer records the read-back MBR and boot sector. The next `RUN` writes the effective MBR first, locates the FAT region from the new partition entry, validates `partition_start` and `partition_sectors` against that region, writes the boot sector at the new region, and projects the tree there. A new region that cannot be found or mounted fails with a clear projection error.
- **VM abstraction.** `dosker-build` owns an `InDosRunner` seam. `EngineInDosRunner` drives the existing `dosker-engine` adapter/session contract through an `EngineOpener`; tests inject a fake opener, adapter, and session. The runtime supervisor is not reused.
- **Socket directory.** `EngineInDosRunner` creates the normal container socket directory before starting QEMU. On Unix, it checks the `agent`, `qmp`, `monitor`, and `gdb` socket paths for the generated id and, when any path falls back to `short_socket_dir()` because the temp root exceeds 100 bytes, creates that fallback directory with mode `0700` before QEMU binds the sockets. The `monitor` path is the longest and can fall back alone.
- **Boot comparison.** `BootMetadata::boot_artifacts_equal` compares the layout, disk size, geometry, partition bounds, and the `sha256` fields of the MBR and boot-sector `BlobRef`s. Inline bytes are not compared, so a metadata value with or without `bytes_inline` compares equal.
- **`C:\DOSKER` restoration.** The staged tree is never rewritten from read-back. The builder filters `C:\DOSKER` out of the read-back tree and out of the delta, so agent files, `CONFIG.INI`, `RUN.BAT`, `OUT.TMP`, and `ERR.TMP` cannot enter a layer.
- **Output timing.** The agent client collects output for one `EXEC`; the builder reports it after the command returns, including on a non-zero exit, before returning the typed error.
- **Effective boot.** `OciStore::load_effective_boot` returns the last non-null layer boot override or the config boot. `BuildState` tracks the same value so consecutive `RUN` instructions project the most recent boot sector, MBR, and partition bounds.
- **Disk template.** The private disk starts as a reflink or full copy of the image's `org.dosker.rawdisk.digest` blob. The effective MBR and boot artifacts are written over that copy before projection; this makes a stored image's layer boot override visible to the next `RUN`.
- **Engine selection per step.** The initial build engine is the base config's `engine_pin` or QEMU. Each `ENGINE` instruction changes the engine for later steps. The final engine is the last `ENGINE` in the file (or the initial engine). Every step resolves its effective engine, checks and records its pin, and a `RUN` cache key uses the engine version at that step. `resolve_lock` retains every engine pin already in the on-disk lock, so a non-final RUN-step engine cannot be dropped from the check when a later `ENGINE` sets a different final engine.
- **Cache identity.** The `RUN` projection depends on the raw disk layout as well as the tree, so the raw disk digest is an input hash. A tree-only base is rejected before the cache lookup and cannot reuse a stale `RUN` cache entry.
- **Attributes and timestamps.** The FAT writer cannot set DOS attributes, so the projection writes default attributes and fixed timestamps. The read-back delta compares kind and content only; an attribute-only or timestamp-only difference never produces a layer.
- **Error mapping.** Engine opener and discovery failures map to `Engine`; probe, start, boot timeouts, early exits, and stop failures map to `RunBootFailed`; `ExecTimeout` maps to `RunTimeout`; `GuestRebooted` maps to `GuestRebooted`; a non-zero exit maps to `RunExited`; projection and read-back failures map to `RunInDos` with the instruction line.

## Shared Interfaces

These names and signatures are used by more than one task and must not drift.

- `dosker_image::BootMetadata::boot_artifacts_equal(&self, other: &Self) -> bool`.
- `dosker_image::OciStore::load_effective_boot(&self, reference: &ImageRef) -> Result<BootMetadata, ImageError>`.
- `dosker_image::project::apply_boot_metadata(disk: &Path, boot: &BootMetadata) -> Result<(), ImageError>`.
- `dosker_image::project::project_tree(disk: &Path, tree: &Tree) -> Result<BootMetadata, ImageError>`.
- `dosker_image::ImageError::Projection { reason: String }`.
- `dosker_build::run_in_dos::{RUN_BOOT_TIMEOUT, RUN_STOP_GRACE, RUN_EMPTY_OUTPUT_NOTE, RUN_TRUNCATED_NOTE}`.
- `dosker_build::run_in_dos::{InDosRunRequest, InDosRunOutcome, InDosRunner, RunDelta, build_delta}`.
- `dosker_build::run_in_dos::{EngineOpener, QemuEngineOpener, EngineInDosRunner}`.
- `dosker_build::Builder::with_in_dos_runner(Box<dyn InDosRunner>) -> Self`.
- `dosker_build::BuildState::with_boot(ImageConfig, Option<Tree>, &[Descriptor], EngineName, BootMetadata, Option<String>) -> Self`.
- `dosker_build::BuildError::{RunInDos, RunExited, RunTimeout, RunBootFailed, GuestRebooted}`.
- `dosker_build::lock::{check_engine_pin, record_engine_pin}`.
- `dosker_engine::EngineSpec::allow_reboot: bool`, `EngineSpec::serial_ports: u8`, and `EngineSpec::control_port: u8`.
- `dosker_platform::fs::ensure_dir_mode(&Path, u32) -> Result<(), PlatformError>` and `dosker_platform::short_socket_dir() -> PathBuf`.

## Tasks

- [x] **Task 1: Boot artifact comparison**

    - **Implements:** FR-002; design §Data Model (effective boot metadata), §Build And Runtime Semantics (RUN boot metadata), data model §BootMetadata.
    - **Modify `crates/dosker-image/tests/boot.rs`**
        - Add `fn mbr_boot(sector_byte: u8) -> BootMetadata` — valid MBR-partitioned metadata with a 512-byte MBR and boot sector built from a repeated byte.
        - `boot_artifacts_ignore_inline_bytes` — the same `sha256` with `bytes_inline: Some(base64_value)` and `None` compares equal.
        - `boot_artifacts_detect_a_changed_boot_sector` — a changed boot-sector byte makes the values unequal.
        - `boot_artifacts_detect_a_changed_mbr` — a changed MBR byte makes the values unequal.
        - `boot_artifacts_detect_changed_partition_bounds` — changing `partition_start`, `partition_sectors`, or `disk_size` makes the values unequal.
        - `boot_artifacts_detect_a_changed_geometry` — changing `heads` or `sectors` makes the values unequal.
    - **Modify `crates/dosker-image/src/boot.rs`**
        - `impl BootMetadata { pub fn boot_artifacts_equal(&self, other: &Self) -> bool; }` — compares `layout`, `disk_size`, `geometry`, `partition_start`, `partition_sectors`, the `mbr` `sha256`, and the `boot_sector` `sha256`. Ignore `fat_type`, `volume_label`, and `media_descriptor` because they are covered by the boot-sector digest.
        - Add private `fn blob_digest(blob: &Option<BlobRef>) -> Option<&str>`.
    - **Verify:** `cargo test -p dosker-image --test boot`

- [x] **Task 2: Effective boot metadata from the store**

    - **Implements:** FR-002; design §Data Model (effective boot metadata), data model §Layer.
    - **Modify `crates/dosker-image/tests/oci.rs`**
        - Add a first layer from an empty or one-file delta with the existing `layer` helper and a second layer built with `Layer::from_delta(&LayerDelta::new(), "step", Some(first_digest), Some(changed_boot))`.
        - `load_effective_boot_uses_the_last_layer_override` — commit a base config with boot A and layers with `None` then boot B; `load_effective_boot` returns B.
        - `load_effective_boot_falls_back_to_the_config` — a single layer with no override returns the config boot.
        - `load_effective_boot_returns_the_config_for_a_booter` — commit a booter image; the returned value is the opaque config boot.
    - **Modify `crates/dosker-image/src/oci.rs`**
        - `pub fn load_effective_boot(&self, reference: &ImageRef) -> Result<BootMetadata, ImageError>` — resolve the reference, return `config.boot` for a booter, otherwise load every manifest layer with `load_layer` and return `effective_boot(&config, &layers)`.
    - **Verify:** `cargo test -p dosker-image --test oci`

- [x] **Task 3: Bootable FAT projection and boot application**

    - **Implements:** FR-002, FR-007; design §Assumptions And Constraints (canonical tree plus boot metadata), §Build And Runtime Semantics (RUN materializes a bootable image, reads back the tree), §Risks (one boot per RUN), data model §BootMetadata.
    - **Create `crates/dosker-image/tests/project.rs`**
        - Add `mod support;` and use `support::make_fat16_image`, `support::make_fat16_image_with_label`, `support::write_fat_file`, `support::write_fat_dir`.
        - Constants: `PARTITION_OFFSET: u64 = 1_048_576`, `PARTITION_LENGTH: u64 = 8 * 1024 * 1024`, `DISK_SIZE: u64 = 16 * 1024 * 1024`.
        - Add `fn mbr_disk(path: &Path)` and `fn superfloppy_disk(path: &Path)` fixtures; add `fn tree(files: &[(&str, &[u8])], dirs: &[&str]) -> Tree` using `fixed_timestamp()` and default attributes.
        - `projection_round_trips_the_tree` — a tree with `APP/HELLO.TXT`, `EMPTY` directory, and `AUTOEXEC.BAT`; `read_tree` after projection equals the input tree.
        - `projection_writes_an_empty_volume` — an empty tree over a disk holding `OLD.TXT` and `OLD/INNER.TXT`; `read_tree` is empty.
        - `projection_replaces_an_existing_file`.
        - `projection_removes_a_file_absent_from_the_tree`.
        - `projection_removes_a_directory_absent_from_the_tree`.
        - `projection_preserves_the_mbr_and_boot_sector` — bytes `0..512` and `PARTITION_OFFSET..PARTITION_OFFSET+512` are unchanged; `read_boot_metadata` is unchanged.
        - `projection_preserves_a_superfloppy_boot_sector`.
        - `projection_preserves_disk_size_label_media_and_geometry`.
        - `projection_rejects_a_non_bootable_image` — a zeroed 1 MiB file returns `ImageError::NotBootable`.
        - `projection_rejects_a_fat32_volume` — a hand-built FAT32 boot sector in a 1 MiB superfloppy returns `ImageError::Projection`.
        - `projection_rejects_a_bpb_it_cannot_preserve` — a boot sector with `reserved_sectors = 2` and otherwise valid FAT16 fields returns `ImageError::Projection`.
        - `projection_rejects_a_non_512_byte_sector` — a boot sector with `bytes_per_sector = 1024` and otherwise valid FAT16 fields returns `ImageError::Projection`, because the projection restores exactly 512 boot-sector bytes.
        - `apply_boot_metadata_writes_the_mbr_and_boot_sector`.
        - `apply_boot_metadata_uses_the_override_mbr_to_locate_the_partition` — a disk whose current MBR points at 1 MiB, an effective MBR that points at 2 MiB, and a valid boot sector at 2 MiB; after `apply_boot_metadata`, `fat_region` reports 2 MiB and `read_boot_metadata` reports the 2 MiB bounds. This is the partition-override round-trip at the projection layer.
        - `projection_round_trips_a_moved_partition` — after the moved-partition `apply_boot_metadata`, `project_tree` with a tree writes it at the new region and `read_tree` returns it; the hole at the old region is not part of the volume.
        - `apply_boot_metadata_rejects_missing_inline_bytes` — `bytes_inline: None` returns `Projection`.
        - `apply_boot_metadata_rejects_an_opaque_layout`.
        - `apply_boot_metadata_rejects_a_mismatched_partition` — the effective MBR points at one region while `partition_start` names another; `Projection` names the partition bounds.
    - **Create `crates/dosker-image/src/project.rs`**
        - `pub const PROJECTION_SECTOR_BYTES: usize = 512;`
        - `pub fn apply_boot_metadata(disk: &Path, boot: &BootMetadata) -> Result<(), ImageError>`:
            - Reject `BootLayout::Opaque` with `ImageError::Projection { reason: "an opaque image has no bootable FAT volume" }`.
            - Require the disk length to equal `boot.disk_size`.
            - For `MbrPartitioned`, decode and verify `boot.mbr`, require 512 bytes, and write the MBR at offset 0 **before** locating the region.
            - For `Superfloppy`, require no MBR and locate the region at offset 0.
            - Call `fat_region(disk)` **after** writing the MBR, so the region comes from the effective partition table. Map a `NotBootable` or `UnsupportedLayout` result to `ImageError::Projection { reason: "the effective MBR has no readable FAT partition" }`.
            - For `MbrPartitioned`, require `boot.partition_start == region.offset / 512` and `boot.partition_sectors == region.length / 512`; a disagreement is `ImageError::Projection { reason: "the effective MBR and partition bounds disagree" }`.
            - Decode and verify `boot.boot_sector`, require 512 bytes, and write it at `region.offset`.
            - A missing or undecodable `BlobRef`, a digest mismatch, or a size mismatch is `ImageError::Projection` naming the artifact.
        - `pub fn project_tree(disk: &Path, tree: &Tree) -> Result<BootMetadata, ImageError>`:
            - `fat_region(disk)` locates the volume from the effective MBR written by `apply_boot_metadata`.
            - `Bpb::parse` requires a valid FAT12/FAT16 BPB: `bytes_per_sector == 512`, `sectors_per_cluster` nonzero and a power of two, `reserved_sectors == 1`, `fats` 1 or 2, nonzero root entries for FAT12/FAT16, nonzero total sectors, and nonzero sectors per FAT. A FAT32 BPB or any value formatting would replace is `ImageError::Projection`.
            - `Bpb::fat_type()` returns `fatfs::FatType::Fat12` below 4085 clusters and `fatfs::FatType::Fat16` below 65525; a larger cluster count is `ImageError::Projection`.
            - `Bpb::format_options()` passes the parsed FAT type, sector size, total sectors, `bytes_per_cluster = sectors_per_cluster * bytes_per_sector`, FAT count, root entries, media byte, sectors per track, heads, drive number, volume id, and the raw 11-byte label. The volume id and label are passed only when the extended boot signature at offset 38 is `0x29`; a label of all zeros is not passed.
            - `format_volume` runs over `PartitionFile::new(file, region.offset, region.length)`; a formatting error is `ImageError::Projection { reason: "cannot format the volume: {source}" }`.
            - Read the freshly formatted boot sector and compare its data-layout fields with the source through `Bpb::layout_matches`: bytes per sector, sectors per cluster, reserved sectors, FAT count, root entries, total sectors, media, and sectors per FAT. A difference is `ImageError::Projection { reason: "the volume BPB cannot be preserved" }`.
            - Restore the original 512-byte boot sector at `region.offset`.
            - Mount with `fixed_fs_options()`, create every `tree.dirs()` entry in path order, then create and truncate every `tree.files()` entry in path order and write its bytes. Use the crate-private `fat::fat_path`. Unmount. Mount, create, write, and unmount errors are `ImageError::Corrupt` naming the disk path and the failing DOS path.
            - Return `read_boot_metadata(disk)`.
        - Private `struct Bpb`; private `fn read_boot_sector(disk, offset) -> Result<[u8; PROJECTION_SECTOR_BYTES], ImageError>`; private `fn decode_inline(blob: Option<&BlobRef>, name: &str) -> Result<Vec<u8>, ImageError>`; private `fn write_tree(disk, region, tree) -> Result<(), ImageError>`.
    - **Modify `crates/dosker-image/src/fat.rs`** — change `fn fat_path` to `pub(crate) fn fat_path`.
    - **Modify `crates/dosker-image/src/error.rs`** — add `Projection { reason: String }` with message `cannot project the tree onto the disk: {reason}`.
    - **Modify `crates/dosker-image/src/lib.rs`** — add `pub mod project;` and re-export `apply_boot_metadata`, `project_tree`.
    - **Modify `crates/dosker-image/tests/support/mod.rs`** — no new helper; `make_fat16_image_with_label` already exists.
    - **Verify:** `cargo test -p dosker-image --test project`

- [x] **Task 4: Build state boot and disk template tracking**

    - **Implements:** FR-002; design §Build And Runtime Semantics (RUN boot metadata and image config), data model §Layer (effective boot).
    - **Modify `crates/dosker-build/tests/config_instructions.rs`**
        - `apply_layer_records_the_effective_boot` — a tree layer with `boot: Some(value)` updates `state.boot`.
        - `apply_layer_without_an_override_keeps_the_effective_boot` — a tree layer with `boot: None` leaves `state.boot` unchanged.
        - `new_state_starts_from_the_config_boot` — `BuildState::new` uses `config.boot` and `disk_digest == None`.
        - `with_boot_keeps_the_supplied_boot_and_digest`.
    - **Modify `crates/dosker-build/src/state.rs`**
        - Add `pub boot: BootMetadata` and `pub disk_digest: Option<String>` to `BuildState`.
        - Add `pub fn with_boot(config: ImageConfig, tree: Option<Tree>, base_layers: &[Descriptor], selected_engine: EngineName, boot: BootMetadata, disk_digest: Option<String>) -> Self`.
        - Keep `pub fn new(config: ImageConfig, tree: Option<Tree>, base_layers: &[Descriptor], selected_engine: EngineName) -> Self` as a wrapper using `config.boot.clone()` and `None`.
        - In `apply_layer`, set `self.boot` to `layer.meta.boot.clone()` when it is `Some`.
    - **Modify `crates/dosker-build/src/build.rs`**
        - Add `boot: BootMetadata` and `disk_digest: Option<String>` to `ResolvedBase`.
        - In `load_stored_base`, call `self.store.load_effective_boot(&reference)?` and take `manifest.raw_disk_digest().map(str::to_owned)`.
        - In `import_base`, the same for `image_ref`.
        - Destructure the new fields and build state with `BuildState::with_boot(config, tree, &base_layers, selected_engine, boot, disk_digest)`.
    - **Verify:** `cargo test -p dosker-build --test config_instructions` and `cargo test -p dosker-build --test build`

- [x] **Task 5: In-DOS run request and read-back delta**

    - **Implements:** FR-002, FR-007; design §Build And Runtime Semantics (RUN tree delta, `C:\DOSKER` exclusion, boot metadata), contract §RUN.
    - **Create `crates/dosker-build/tests/run_in_dos_delta.rs`**
        - Add `fn file(path: &str, bytes: &[u8]) -> TreeFile` and `fn boot(sector_byte: u8) -> BootMetadata`.
        - `delta_adds_a_new_file` — fixed timestamp and archive attribute.
        - `delta_replaces_a_changed_file`.
        - `delta_whiteouts_a_deleted_file_once`.
        - `delta_whiteouts_a_deleted_directory_once`.
        - `delta_handles_a_file_replaced_by_a_directory` — whiteout plus directory entry.
        - `delta_ignores_the_agent_directory`.
        - `delta_ignores_timestamp_and_attribute_only_changes`.
        - `delta_records_a_boot_override` — a changed boot sector digest produces `Some(boot_after)`.
        - `delta_records_a_partition_override` — a changed partition start or sector count produces `Some(boot_after)`.
        - `delta_keeps_boot_null_when_the_artifacts_match`.
    - **Create `crates/dosker-build/src/run_in_dos.rs`**
        - `pub const RUN_BOOT_TIMEOUT: Duration = Duration::from_secs(60);`
        - `pub const RUN_STOP_GRACE: Duration = Duration::from_secs(10);`
        - `pub const RUN_EMPTY_OUTPUT_NOTE: &str = "RUN produced no captured output";`
        - `pub const RUN_TRUNCATED_NOTE: &str = "RUN output was truncated";`
        - `pub struct InDosRunRequest<'a> { pub line: usize, pub command: &'a str, pub workdir: &'a DosPath, pub env: &'a BTreeMap<String, String>, pub timeout: Duration, pub boot_timeout: Duration, pub disk_path: &'a Path, pub disk_interface: &'a str, pub serial_ports: u8, pub control_port: u8, pub log_dir: &'a Path, pub root: &'a Path, pub engine: EngineName }`.
        - `pub struct InDosRunOutcome { pub exit_code: i32, pub output: String, pub truncated: bool }`.
        - `pub trait InDosRunner { fn run(&mut self, request: &InDosRunRequest<'_>) -> Result<InDosRunOutcome, BuildError>; }`
        - `pub struct RunDelta { pub delta: LayerDelta, pub boot: Option<BootMetadata> }`.
        - `pub fn build_delta(staged: &Tree, read_back: &Tree, boot_before: &BootMetadata, boot_after: &BootMetadata) -> RunDelta`:
            - Ignore reserved paths.
            - A staged path whose read-back kind differs, whose file bytes differ, or which is absent produces a change; a changed path absent from read-back or with a different kind becomes a whiteout when topmost.
            - A read-back-only or changed path becomes a `TreeFile` with archive or a `TreeDir` with default attributes, using `fixed_timestamp()`.
            - Ignore timestamps and attributes for change detection.
            - `boot` is `Some(boot_after.clone())` only when `!boot_before.boot_artifacts_equal(boot_after)`.
        - Private `fn topmost_change(path: &DosPath, changed: &BTreeSet<DosPath>) -> bool`.
    - **Modify `crates/dosker-build/src/lib.rs`** — re-export the module symbols.
    - **Verify:** `cargo test -p dosker-build --test run_in_dos_delta`

- [x] **Task 6: QEMU build VM runner**

    - **Implements:** FR-007, FR-009; design §Process Model (transient build VM), §Component Interfaces (Engine Adapter), §Error Handling And Edge Cases (boot timeout, guest reboot), contract §RUN.
    - **TDD:** write the failing `crates/dosker-build/tests/run_in_dos_engine.rs` cases, including the long-path socket case, and the `command_allows_a_reboot_for_a_transient_build` case first.
    - **Modify `crates/dosker-engine/src/spec.rs`**
        - Add `pub allow_reboot: bool`, `pub serial_ports: u8`, and `pub control_port: u8`.
    - **Modify `crates/dosker-engine-qemu/src/command.rs`**
        - Push `-no-reboot` only when `!spec.allow_reboot`.
        - Replace the two fixed `-serial` arguments with a loop over `1..=spec.serial_ports`: the agent socket on `spec.control_port`, `none` on every other port. Return `EngineError::Setup` when `control_port` is zero or greater than `serial_ports`.
    - **Modify every `EngineSpec` literal to set `allow_reboot: false`, `serial_ports: 2`, and `control_port: 2` unless it is the build runner** — `crates/dosker-runtime/src/supervisor.rs`, `crates/dosker-test-support/tests/mock_adapter.rs`, `crates/dosker-engine/tests/adapter.rs`, `crates/dosker-engine-qemu/src/session.rs` (test `session_spec`), `crates/dosker-engine-qemu/tests/command.rs`, `crates/dosker-engine-qemu/tests/qemu_session.rs`, `crates/dosker-engine-qemu/tests/endpoints.rs`, `crates/dosker-engine-qemu/tests/session_endpoints.rs`.
    - **Create `crates/dosker-build/src/run_in_dos/engine.rs`**
        - `pub trait EngineOpener { fn open(&mut self, engine: EngineName) -> Result<Box<dyn EngineAdapter>, BuildError>; }`
        - `pub struct QemuEngineOpener;` — for `EngineName::Qemu` discover `QemuAdapter` with `Arc::new(dosker_platform::process::system())`, mapping a discovery error to `BuildError::Engine { engine: "qemu".to_owned(), reason: error.to_string() }`; for every other engine return `BuildError::Engine { engine: engine.as_str().to_owned(), reason: "in-DOS RUN supports only the qemu engine" }`.
        - `pub struct EngineInDosRunner { opener: Box<dyn EngineOpener> }`; `new`, `with_opener`, and `Default`.
        - `impl InDosRunner for EngineInDosRunner`:
            - Open and probe the adapter; map probe failures to `RunBootFailed`.
            - Generate a `ContainerId`; call `ensure_socket_dirs(request.root, &id)`:
                - `const SOCKET_DIR_MODE: u32 = 0o700;`
                - `ensure_dir_mode(&request.root.join("containers").join(id.as_str()), SOCKET_DIR_MODE)`.
                - On Unix, iterate `["agent", "qmp", "monitor", "gdb"]`; if **any** `dosker_platform::engine_socket_path(root, &id, name)` is under `dosker_platform::short_socket_dir()`, `ensure_dir_mode` that directory with `SOCKET_DIR_MODE`. Checking `monitor` alone is not enough because it is the longest path and can fall back while the others fit.
                - Map `PlatformError` to `RunBootFailed`.
            - Build `EngineSpec { root: request.root.to_path_buf(), container_id: id, workdir: request.workdir.to_dos_string(), env: request.env.clone(), cmd: None, expects_agent: true, log_dir: request.log_dir.to_path_buf(), disk_path: request.disk_path.to_path_buf(), disk_interface: request.disk_interface.to_owned(), serial_ports: request.serial_ports, control_port: request.control_port, booter: false, display: DisplayMode::None, vnc_port: None, allow_reboot: false }`.
            - `start`, then `ready(request.boot_timeout)`; map start/ready errors to `RunBootFailed`, with specific messages for `ReadyTimeout` and `Exited(code)`; kill on a ready error.
            - `exec` with the command, env, workdir, and timeout; map `ExecTimeout` to `RunTimeout`, `GuestRebooted` to `GuestRebooted`, `NoAgent` to `RunBootFailed` naming the missing agent, and other errors to `RunBootFailed`; kill on error.
            - `stop(RUN_STOP_GRACE)`; a stop error kills and returns `RunBootFailed` with "the guest did not shut down cleanly".
            - Return the outcome.
    - **Modify `crates/dosker-build/src/run_in_dos.rs`** — `pub mod engine;` and re-export.
    - **Modify `crates/dosker-build/tests/run_in_dos_engine.rs`**
        - `FakeOpener`/`FakeAdapter`/`FakeSession` recording the spec and scripted results in `Arc<Mutex<FakeEngineState>>`.
        - `runner_boots_headless_with_reboot_disabled`.
        - `runner_passes_the_serial_layout`.
        - `runner_passes_the_workdir_environment_and_disk`.
        - `runner_uses_the_command_timeout`.
        - `runner_returns_the_captured_output`.
        - `runner_stops_the_guest_after_the_command`.
        - `runner_maps_a_command_timeout`.
        - `runner_maps_a_guest_reboot`.
        - `runner_maps_a_boot_timeout_and_kills_the_vm`.
        - `runner_maps_an_engine_exit_before_ready`.
        - `runner_maps_a_stop_failure`.
        - `runner_rejects_a_non_qemu_engine`.
        - `#[cfg(unix)] runner_creates_the_short_socket_directory` — a request root longer than 100 bytes; after the run, the recorded spec's container directory exists, `dosker_platform::short_socket_dir()` exists with mode `0700`, and `engine_socket_path` for all four names resolves into the short directory.
        - `#[cfg(unix)] runner_creates_the_short_socket_directory_for_the_longest_endpoint` — a root sized so the `agent` path fits but the `monitor` path exceeds 100 bytes; the short directory exists with mode `0700` and `engine_socket_path(&root, &spec.container_id, "monitor")` is under it.
    - **Modify `crates/dosker-engine-qemu/tests/command.rs`**
        - `command_allows_a_reboot_for_a_transient_build`.
        - `command_maps_the_agent_to_the_effective_control_port` — 3/3 produces three pairs with the agent at the third; 2/2 produces two with the agent at the second.
    - **Verify:** `cargo test -p dosker-engine-qemu --test command` and `cargo test -p dosker-build --test run_in_dos_engine` and `cargo test -p dosker-engine --test adapter`

- [x] **Task 7: Build engine integration for `RUN`**

    - **Implements:** FR-002, FR-003, FR-007; design §Build And Runtime Semantics (RUN, cache key, failed instruction), §Error Handling And Edge Cases, contract §RUN, §Normalization And Cache Keys.
    - **TDD:** write the failing `crates/dosker-build/tests/run_in_dos.rs` cases and support helpers first; they do not compile until the action and error variants land.
    - **Modify `crates/dosker-build/src/lock.rs`**
        - Add `pub fn check_engine_pin(existing: &LockFile, engine: &ResolvedEngine, update: bool) -> Result<(), BuildError>` and `pub fn record_engine_pin(lock: &mut LockFile, engine: &ResolvedEngine)`.
        - `resolve_lock` seeds the returned lock's `engines` map with every pin from the on-disk lock, then checks and records the supplied final engine. It no longer starts from `LockFile::new()` with an empty engine map.
        - Re-export `check_engine_pin` and `record_engine_pin` from `crates/dosker-build/src/lib.rs`.
    - **Modify `crates/dosker-build/src/build.rs`**
        - Add `in_dos_runner: RefCell<Box<dyn InDosRunner>>`; `with_in_dos_runner`.
        - Split the current `select_engine` into `initial_engine(config: &ImageConfig) -> EngineName` (config `engine_pin` or QEMU) and `final_engine(instructions: &[Instruction], config: &ImageConfig) -> EngineName` (last `ENGINE` or initial).
        - Resolve `final_engine` for `resolve_lock`; initialize `BuildState` with `initial_engine`; keep the lock mutable.
        - In the instruction loop, before `prepare_step`, `let step_engine = self.resolver.resolve(state.selected_engine.as_str())?; check_engine_pin(&lock, &step_engine, options.update_lock)?; record_engine_pin(&mut lock, &step_engine);`.
        - Change `prepare_step` to take `step_engine: &ResolvedEngine`. For `InstructionKind::Run { host: false, .. }`:
            - A booter image returns the `Instruction` error `a booter image does not support RUN`.
            - `config.agent_version.is_none()` returns `RunBootFailed { line, reason: "the image has no agent" }`.
            - `state.disk_digest.as_deref()` must be `Some`; otherwise return `RunInDos { line, reason: "the base image has no raw disk; RUN requires a bootable disk image" }` **before the cache lookup**.
            - A selected engine other than QEMU returns `BuildError::Engine { engine: state.selected_engine.as_str().to_owned(), reason: "in-DOS RUN supports only the qemu engine" }`.
            - Input hashes: `staged:<parent_tree_digest>`, `engine:<step_engine.version>`, `agent:<agent_version>`, `disk:<raw_disk_digest>`.
            - `tree_step` is true; the normalized instruction is `RUN {raw}`; on success return `PreparedStep { normalized, input_hashes, tree_step: true, action: StepAction::RunInDos { command: raw.clone(), workdir } }`.
        - In `apply_miss`, add the `StepAction::RunInDos { command: String, workdir: DosPath }` branch with these locals and steps:
            - `let tree = state.tree.as_ref().ok_or_else(|| BuildError::RunInDos { line, reason: "the image has no tree".to_owned() })?;`
            - `let disk_digest = state.disk_digest.as_deref().ok_or_else(|| BuildError::RunInDos { line, reason: "the base image has no raw disk; RUN requires a bootable disk image".to_owned() })?;`
            - `let tmp_root = self.root.join("tmp"); fs::create_dir_all(&tmp_root).map_err(|source| BuildError::Io { path: tmp_root.clone(), source })?;`
            - `let tempdir = tempfile::Builder::new().prefix("dosker-run-").tempdir_in(&tmp_root).map_err(|source| BuildError::Io { path: tmp_root.clone(), source })?;`
            - `let disk_path = tempdir.path().join("disk.img");` and `let log_dir = tempdir.path();`
            - `self.store.copy_blob_to(disk_digest, &disk_path)`, `dosker_image::apply_boot_metadata(&disk_path, &state.boot)`, `let projected = dosker_image::project_tree(&disk_path, tree)`, and `dosker_image::write_config(&disk_path, state.config.machine.control_port, state.config.machine.baud)`; map every image or I/O error to `RunInDos { line, reason: error.to_string() }`.
            - Require `state.boot.boot_artifacts_equal(&projected)`; otherwise return `RunInDos { line, reason: "the projection changed the boot metadata" }`.
            - `let outcome = self.in_dos_runner.borrow_mut().run(&InDosRunRequest { line: instruction.line, command: command.as_str(), workdir: &workdir, env: &state.config.env, timeout: options.timeout, boot_timeout: RUN_BOOT_TIMEOUT, disk_path: disk_path.as_path(), disk_interface: state.config.machine.disk_interface.as_str(), serial_ports: state.config.machine.serial_ports, control_port: state.config.machine.control_port, log_dir: log_dir, root: tempdir.path(), engine: state.selected_engine })?;`
            - Report `outcome.output`; report `RUN_EMPTY_OUTPUT_NOTE` when it is empty and `RUN_TRUNCATED_NOTE` when truncated; return `RunExited { line, code }` for a non-zero exit.
            - `let read_back = dosker_image::read_tree(&disk_path).map_err(|error| BuildError::RunInDos { line, reason: error.to_string() })?;`
            - `let boot_after = dosker_image::read_boot_metadata(&disk_path).map_err(|error| BuildError::RunInDos { line, reason: error.to_string() })?;`
            - `let run = build_delta(tree, &read_back, &state.boot, &boot_after);`
            - `let layer = Layer::from_delta(&run.delta, &prepared.normalized, previous.map(str::to_owned), run.boot.clone())?;`, then `put_layer`, `state.apply_layer`, and `cache.store`.
        - Update `BuildOptions.timeout` doc to `RUN` and `RUN --host`.
    - **Modify `crates/dosker-build/src/error.rs`** — the five typed variants with the line-prefixed messages; import `Duration`.
    - **Modify `crates/dosker-build/tests/build.rs`**
        - Rename `build_rejects_an_in_image_run` to `build_rejects_an_in_image_run_without_an_agent`; keep a tree-only base with no agent by calling `store_tree_base(fixture.root_path(), "base:latest", &empty_tree(), None, None)`, and assert `BuildError::RunBootFailed { reason }` contains `no agent`; remove the `RUN --host` assertion.
        - Add `build_with_runner` and `build_with_runner_and_resolver` to `Fixture`.
    - **Modify `crates/dosker-build/tests/support/mod.rs`**
        - Add the disk constants, `make_agent_disk`, `store_agent_base`, `store_agent_base_from_disk`, `store_agent_base_with_gap_byte`, `disk_file`, `write_disk_file`, `remove_disk_file`, `disk_byte`, `flip_disk_byte`, and `move_disk_partition`.
        - `make_agent_disk(path) -> BootMetadata` creates the FAT16 MBR disk with `AUTOEXEC.BAT`, `HELLO.TXT`, `OLD.TXT`, `OLD/INNER.TXT`, and `DOSKER/DOSKER.EXE`, and returns `read_boot_metadata`.
        - `store_agent_base` delegates to `store_agent_base_from_disk(root, reference, disk_path)`; `store_agent_base_from_disk` reads the disk tree and boot metadata, stores the raw blob, commits one tree layer with the raw-disk annotation and `agent_version = Some("1.0.0")`, and returns the manifest digest.
        - `store_agent_base_with_gap_byte(root, reference, byte)` creates a disk with `make_agent_disk`, writes `byte` at `AGENT_DISK_PARTITION_OFFSET + AGENT_DISK_PARTITION_LENGTH + 4096` (an unused gap after the partition, so the tree and boot metadata do not change), then calls `store_agent_base_from_disk`.
        - `move_disk_partition(disk, new_offset, new_length) -> BootMetadata`: read the current tree, build a new MBR pointing at the new partition, format a FAT16 volume at the new offset with the source BPB geometry and label, write the tree files, and return `read_boot_metadata`.
        - Add `VersionedResolver { versions: BTreeMap<String, String> }` implementing `EngineResolver`.
        - Add `engine: EngineName` to `RecordedRun` and record `request.engine` in `ScriptedRunner`; add the `ScriptedRunner` type.
        - Add `runner_fixture`.
    - **Create `crates/dosker-build/tests/run_in_dos.rs`**
        - `run_in_dos_writes_a_file_into_the_layer`.
        - `run_in_dos_replaces_a_file`.
        - `run_in_dos_deletes_a_file`.
        - `run_in_dos_ignores_agent_files`.
        - `run_in_dos_writes_config_ini_before_boot` — `MACHINE serial=3 control_port=3 baud=9600`; the runner sees `CONTROL_PORT=3\r\nBAUD=9600\r\n`, `serial_ports == 3`, `control_port == 3`.
        - `run_in_dos_reports_the_command_output`.
        - `run_in_dos_reports_empty_output`.
        - `run_in_dos_uses_the_command_and_boot_timeouts`.
        - `run_in_dos_records_a_boot_override`.
        - `run_in_dos_keeps_boot_null_when_unchanged`.
        - `run_in_dos_projects_the_previous_boot_override`.
        - `run_in_dos_reapplies_a_partition_override` — two `RUN` steps; the first runner calls `move_disk_partition` to move the partition by 1 MiB, the first layer's boot is `Some` with the new bounds, the second runner observes the new partition start in the projected disk and returns no changes, the second layer's boot is `None`, and two VM calls are recorded.
        - `run_in_dos_uses_the_engine_before_a_later_engine` — `RUN COPY HELLO.TXT MADE.TXT` then `ENGINE dosbox-x`; the runner records `engine == Qemu`; the lock has `qemu` and `dosbox-x` pins.
        - `run_in_dos_cache_key_uses_the_engine_at_the_run_step` — build with `RUN` then `ENGINE dosbox-x` under QEMU version A, then rebuild with QEMU version B and `update_lock: true`; the RUN misses and the runner is called because its step engine version changed even though the final engine version is unchanged.
        - `run_in_dos_cache_key_includes_the_raw_disk` — two bases with identical tree, config, and boot metadata but different raw blobs, created with `store_agent_base` and `store_agent_base_with_gap_byte`; assert the tree digests, config digests, and effective boots are equal and the RUN cache key differs; the second build misses and calls the runner.
        - `run_in_dos_requires_a_raw_disk` — call `store_tree_base(fixture.root_path(), "base:latest", &empty_tree(), None, Some("1.0.0"))`; `RunInDos` names the missing raw disk; no runner call.
        - `run_in_dos_requires_a_raw_disk_before_the_cache` — write a `CacheRecord` under the old key without `disk:`, then build the same tree-only base; `RunInDos` is returned before the lookup and the runner is not called.
        - `run_in_dos_rejects_a_booter_image`.
        - `run_in_dos_requires_an_agent` — call `store_tree_base(fixture.root_path(), "base:latest", &empty_tree(), Some(b"DISK"), None)`; `RunBootFailed` names the missing agent; no runner call.
        - `run_in_dos_rejects_a_non_qemu_engine` — `ENGINE dosbox-x` before `RUN`; `BuildError::Engine`; no runner call.
        - `run_in_dos_rejects_a_pinned_non_final_engine_mismatch` — pre-write `Doskerfile.lock` with the matching base pin, `qemu` version A, and `dosbox-x`; the Doskerfile is `FROM base:latest`, `RUN COPY HELLO.TXT MADE.TXT`, `ENGINE dosbox-x`; the resolver reports `qemu` version B and the same `dosbox-x`; with `update_lock: false` the build returns `LockMismatch` naming `qemu` and the runner is not called.
        - `run_in_dos_cache_hit_boots_no_vm`.
        - `run_in_dos_failure_writes_no_cache_record` — a single-RUN Doskerfile; a failing build followed by a succeeding build runs the RUN again (`cached_steps == 0`).
        - `run_in_dos_removes_the_temporary_disk`.
        - `run_in_dos_reports_a_non_zero_exit`, `run_in_dos_reports_a_guest_reboot`, `run_in_dos_reports_a_timeout`, `run_in_dos_reports_a_boot_failure`.
    - **Modify `crates/dosker-build/tests/lock_resolution.rs`**
        - `record_engine_pin_adds_a_second_engine`.
        - `check_engine_pin_rejects_a_mismatched_version`.
        - `resolve_lock_keeps_existing_engine_pins` — a lock with `qemu` version A, `resolve_lock` for final `dosbox-x`, the returned lock still contains the `qemu` pin.
    - **Verify:** `cargo test -p dosker-build --test run_in_dos` and `cargo test -p dosker-build --test build` and `cargo test -p dosker-build --test freedos_base` and `cargo test -p dosker-build --test lock_resolution`

- [ ] **Task 8: Real QEMU integration, boot metadata, and guest reboot**

    - **Implements:** FR-007; design §Testing Strategy (real-engine integration), §Success Criteria SC-002; roadmap slice `in-dos-run` (`Done when`).
    - **Create `agent/test/setboot.c`** — a 16-bit DOS helper that reads the FreeDOS base partition boot sector at CHS 4/0/1 (LBA 2048) from drive `0x80` with INT 13h AH=02 into a 512-byte buffer, flips the low bit of the volume-label byte at offset 43 (the boot sector stays bootable), writes the sector back with AH=03, and returns 0 on success or 1 when the BIOS sets carry.
    - **Create `agent/test/reboot.c`** — a 16-bit DOS helper that resets through an empty IDT and a fault, so QEMU sees a system reset and exits under `-no-reboot`.
    - **Modify `agent/build.sh`** — add `SETBOOT.EXE` and `REBOOT.EXE` artifacts compiled from `agent/test/setboot.c` and `agent/test/reboot.c` with the same `wcc`/`wlink` pattern as `EXITCODE.EXE`.
    - **Modify `agent/tools/build-fixture.sh`** — add `DOSKER_SETBOOT_EXE` and `DOSKER_REBOOT_EXE` defaults, copy `SETBOOT.EXE` and `REBOOT.EXE` to `::/DOSKER/`, and update `agent/tools/fixture-test.sh` expected-file list.
    - **Modify `agent/tools/fixture-test.sh`** — create dummy `SETBOOT.EXE` and `REBOOT.EXE` in `$TMP`, export `DOSKER_SETBOOT_EXE` and `DOSKER_REBOOT_EXE` for every `build-fixture.sh` call, and add both names to the expected-file loop so the standalone fixture test still runs without a compiled helper.
    - **Modify `.github/workflows/ci.yml` and `.github/workflows/qemu-nightly.yml`** — upload `SETBOOT.EXE` and `REBOOT.EXE` with the agent artifact and download them where the QEMU suites run.
        - In `ci.yml`, add a `Rust RUN integration suite` step after the existing QEMU suites with `DOSKER_INTEGRATION=1`, `DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img`, `DOSKER_QEMU=qemu-system-i386`, the `DOSKER_FREEDOS_SOURCE` already exported by the FreeDOS base step, and `cargo test -p dosker-cli --features qemu-engine --test qemu_run -- --nocapture`.
        - In `qemu-nightly.yml`, add `env: DOSKER_FREEDOS_SOURCE: ${{ vars.DOSKER_FREEDOS_SOURCE }}` to the `preflight` job's `Check required QEMU repository variables` step and pass `DOSKER_FREEDOS_SOURCE` after `DOSKER_QEMU_IMAGE_URL` to `.github/scripts/check-qemu-variables.sh`, so the required variable is validated before the runtime job starts.
        - In the `runtime-macos` job, keep the existing prepared-image export: the download step writes `DOSKER_QEMU_IMAGE=$RUNNER_TEMP/dosker-agent.img` to `GITHUB_ENV`. Add a `Fetch the FreeDOS source` step before the build/RUN suites with `env: DOSKER_FREEDOS_SOURCE: ${{ vars.DOSKER_FREEDOS_SOURCE }}`; the step runs `source_path="$(bash .github/scripts/fetch-freedos-source.sh)"`, so the script receives the repository variable and resolves it to a local image, then appends `DOSKER_FREEDOS_SOURCE=$source_path` to `GITHUB_ENV`, so every later build and test step inherits the resolved local path.
        - Add the `Rust RUN integration suite` step to `runtime-macos` with `DOSKER_INTEGRATION=1`, `DOSKER_QEMU_IMAGE="$DOSKER_QEMU_IMAGE"`, `DOSKER_QEMU=qemu-system-i386`, `DOSKER_FREEDOS_SOURCE="$DOSKER_FREEDOS_SOURCE"`, and the same `qemu_run` command. Do not reference `agent/fixtures/dosker-agent.img` in this job; that path is gitignored and absent.
    - **Modify `crates/dosker-test-support/src/fixtures.rs`**
        - `setboot_executable() -> Result<PathBuf, String>` and `reboot_executable()` resolving `agent/build/SETBOOT.EXE` and `agent/build/REBOOT.EXE`.
        - `freedos_source() -> Result<PathBuf, String>` reading `DOSKER_FREEDOS_SOURCE` and resolving a relative path against the workspace root.
    - **Create `crates/dosker-cli/tests/qemu_run.rs`** (`#![cfg(unix)]`, `mod common;`)
        - Reuse the `qemu_build.rs` gate and a module `Mutex`.
        - Prepared-image tests: `run_in_dos_writes_and_deletes_files`, `run_in_dos_non_zero_exit_fails_the_build`, `build_runs_and_rebuilds_from_cache`.
        - `run_in_dos_records_a_boot_metadata_change` — a context with `SETBOOT.EXE` copied from `fixtures::setboot_executable()`, and a Doskerfile `FROM freedos:1.3`, `COPY SETBOOT.EXE C:/`, `RUN SETBOOT.EXE`, `RUN ECHO AFTER`, `CMD ECHO BUILT`; build through `root.command_with_qemu()` plus `.env("DOSKER_FREEDOS_SOURCE", fixtures::freedos_source()?)` and `common::run_captured`; the build succeeds; the `RUN SETBOOT.EXE` layer's `boot` is `Some` and differs from the base boot; the `RUN ECHO AFTER` layer boots with the changed boot sector and has `boot: None`; the built tree contains `SETBOOT.EXE`.
        - `run_in_dos_reports_a_guest_reboot` — a context with `REBOOT.EXE` copied from `fixtures::reboot_executable()`, and a Doskerfile `FROM freedos:1.3`, `COPY REBOOT.EXE C:/`, `RUN REBOOT.EXE`; build through `root.command_with_qemu()` plus `.env("DOSKER_FREEDOS_SOURCE", fixtures::freedos_source()?)` and `common::run_captured`; the build exits non-zero, `stderr` contains `the guest rebooted during the command`, and `built:latest` is absent.
        - Leave `agent/build/EXITCODE.EXE` inside the prepared image; do not generate a new exit-code helper.
    - **Verify:** `ruby -ryaml -e 'YAML.load_file(".github/workflows/ci.yml"); YAML.load_file(".github/workflows/qemu-nightly.yml")' && DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE="$DOSKER_QEMU_IMAGE" DOSKER_QEMU=qemu-system-i386 DOSKER_FREEDOS_SOURCE="$DOSKER_FREEDOS_SOURCE" cargo test -p dosker-cli --features qemu-engine --test qemu_run -- --nocapture`

- [ ] **Task 9: Documentation and roadmap status**

    - **Implements:** FR-007; design §Goals (headless, reproducible builds), §Security And Privacy (untrusted images), §Testing Strategy (integration).
    - **Create `docs/builds.md`**
        - `RUN` semantics: one transient VM per instruction, no batching, the command runs through `COMMAND.COM`, output is captured, and output redirection is not needed.
        - Timeouts: `--timeout` defaults to 300 seconds and applies to the agent command; the boot wait is 60 seconds; the shutdown grace is 10 seconds.
        - The disk projection: the tree is written over a private copy of the image's raw disk; the effective MBR is written first; the FAT region is located from it; only FAT12/FAT16 volumes are supported; geometry, label, media descriptor, and disk size are preserved for an unchanged boot.
        - Partition overrides: a changed MBR, partition table, or boot sector is recorded in the layer's boot override; the next `RUN` reapplies the effective MBR, locates the partition from it, and writes the boot sector at the new region.
        - `C:\DOSKER`: `CONFIG.INI` is written before boot; `C:\DOSKER` and agent temporary files are excluded from the layer; the agent files are never modified by a layer.
        - Serial layout: the build VM maps the effective `serial_ports` count and places the agent on `control_port`; `CONFIG.INI` carries the effective control port and baud.
        - Output: the captured agent output is reported after `EXEC` returns, including on a non-zero exit; empty output reports exactly `RUN produced no captured output`; truncated output reports `RUN output was truncated`.
        - Failures: non-zero exit, timeout, boot failure, missing agent, and guest reboot fail the build and commit nothing; a guest reboot reports `the guest rebooted during the command`.
        - Cache: the key includes the parent tree digest, the build config digest, the normalized instruction, the engine version at that step, the agent version, and the raw disk digest; a cache hit boots no VM.
        - Engine selection: each `ENGINE` applies to later steps; the lock records every engine used; `RUN` uses the engine at its own step.
        - Engine support: `qemu` is the only implemented engine; `dosbox-x` and `bochs` fail with a message naming `qemu`. The RUN VM has no NIC, so a command must not depend on network access.
        - Prerequisites: QEMU, `agent/build/DOSKER.EXE`, and a FreeDOS base with the agent; a tree-only image without a raw disk fails before the cache.
        - An example Doskerfile: `FROM freedos:1.3`, `COPY HELLO.TXT C:\`, `RUN COPY HELLO.TXT MADE.TXT`, and `CMD ECHO BUILT`, with a one-line explanation that `RUN` boots once and `CMD` runs at container start.
    - **Modify `docs/images.md`** — add a "Boot metadata" section: a layer may carry a boot override; the effective boot is the last non-null `boot` in layer order or the image config; a `RUN` that changes the boot artifacts writes the override; the next projection uses it.
    - **Modify `README.md`** — add `- [Builds](docs/builds.md)` to the Documentation list.
    - **Modify `.sdd/specs/2026-11-01-dosker/roadmap.md`** — set the `in-dos-run` slice to `**Status:** Planned` and `**Plan:** [\`plan-in-dos-run.md\`](plan-in-dos-run.md)`. Do not make this change until the plan is approved and execution starts.
    - **Verify:** `rg -q 'RUN produced no captured output' docs/builds.md && rg -q '60' docs/builds.md && rg -q 'qemu' docs/builds.md && rg -q 'Boot metadata' docs/images.md && rg -q 'docs/builds.md' README.md && rg -q 'Status:\*\* Planned' .sdd/specs/2026-11-01-dosker/roadmap.md && rg -q 'plan-in-dos-run.md' .sdd/specs/2026-11-01-dosker/roadmap.md && git diff --check`

## Final Verification

Run the full workspace checks after Task 9:

- `agent/build.sh`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --features dosker-cli/mock-engine,dosker-cli/qemu-engine -- -D warnings`
- `cargo test --workspace --features dosker-cli/mock-engine`
- `cargo test -p dosker-image --test boot --test oci --test project`
- `cargo test -p dosker-build --test run_in_dos_delta --test run_in_dos_engine --test run_in_dos --test config_instructions --test build --test freedos_base --test lock_resolution`
- `cargo test -p dosker-engine-qemu`
- `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img DOSKER_QEMU=qemu-system-i386 DOSKER_FREEDOS_SOURCE="$DOSKER_FREEDOS_SOURCE" cargo test -p dosker-cli --features qemu-engine --test qemu_run -- --nocapture`
- `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img DOSKER_QEMU=qemu-system-i386 cargo test -p dosker-cli --features qemu-engine --test qemu_build -- --nocapture`

## Deviations

- The projection supports FAT12 and FAT16 with 512-byte sectors and rejects FAT32 and other sector sizes; the agent and the supported bases are real-mode DOS images, and a FAT32 image cannot host the 16-bit agent.
- The RUN VM passes the disk interface and the machine serial layout, and writes the effective control port and baud into `CONFIG.INI`.
- The RUN VM uses the QEMU adapter's CPU, memory, video, and sound defaults; applying the full machine profile is outside this slice.
- The RUN VM has no NIC because the QEMU adapter emits `-nic none`; user-mode NAT arrives in a later release.
- The projection writes default attributes and fixed timestamps; the RUN delta compares kind and content, so attribute-only and timestamp-only differences do not create a layer.
- The build VM uses `-no-reboot` and maps a clean QEMU exit during EXEC to `GuestRebooted`; the runtime keeps its own reboot policy.
- `SETBOOT.EXE` and `REBOOT.EXE` are test-only DOS helpers built by `agent/build.sh` and copied into the integration context; they are not shipped product files.
