# FreeDOS Base Builder Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-15
- **Status**: Implemented
- **Design**: [`.sdd/specs/2026-11-01-dosker/design.md`](design.md)
- **Roadmap**: [`.sdd/specs/2026-11-01-dosker/roadmap.md`](roadmap.md) — slice: `freedos-base-builder`
- **Depends on**: Slice 5 (`image-store-and-tree`) and Slice 6 (`doskerfile-and-host-build`), both Implemented
- **Manual prerequisites**: A FreeDOS 1.3 Floppy Edition boot floppy (`144m/x86BOOT.img` from `FD13-FloppyEdition.zip`) supplied through `DOSKER_FREEDOS_SOURCE`. The built agent at `agent/build/DOSKER.EXE`. QEMU for the integration tests. The completed FreeDOS builder spike (`agent/tools/build-base-image.sh`, `agent/fixtures/freedos-base.img`, `agent/tools/base-image-test.sh`), which supplies the MBR and boot-sector templates.
- **Goal**: Build the deterministic FreeDOS base and its bootable raw disk, store it as `freedos:1.3`, resolve it from `FROM`, inject the agent into imported DOS images, and pin the base and agent in the lock file.

## Constraints

- Base disk: 32 MiB (`33 554 432` bytes), MBR partition at LBA `2048`, `63 488` sectors, type `0x06`, bootable.
- FAT16: 512-byte sectors, 512-byte clusters (1 sector), 2 FATs, 512 root entries, media descriptor `0xF8`, hidden sectors `2048`.
- Geometry: 16 heads, 32 sectors per track, 124 cylinders, sector size 512.
- Volume label `DOSKER`; volume id `0x0045105C`; OEM name `FRDOS5.1` from the pinned boot-sector template.
- `CONFIG.SYS` is exactly `FILES=20\r\nBUFFERS=10\r\nLASTDRIVE=Z\r\n`.
- Base `AUTOEXEC.BAT` is exactly `@ECHO OFF\r\nSET COMSPEC=C:\COMMAND.COM\r\nC:\DOSKER\DOSKER.EXE\r\n`.
- All generated image timestamps are `1980-01-01T00:00:00Z`; generated entries carry no read-only, hidden, system, or archive bits.
- `C:\DOSKER` is reserved for the agent, `CONFIG.INI`, and the wrapped `AUTOEXEC`.
- Agent version `1.0.0` from `dosker-agent-proto::AGENT_VERSION`; `DOSKER.EXE` is `43 314` bytes.
- FreeDOS source pins: `KERNEL.SYS` `sha256:f34a7483c575fcf2709d9a7d0bc3db81c6211c279530f9e1bf78576b9233924d`; `COMMAND.COM` `sha256:93ed257d3aa2454821aa305b731544550dcccff898865b590deb972c47b48610`.
- Boot input pins: MBR `sha256:d2cc0119c6d3caec354658eb822d3b0dfb69884a521d7c8aa28d59cacdc638ff`; boot sector `sha256:b27479134bbd40f1aa45a12e8540a1d1e9cd1fd73cea8bec9d942372132256b7`; agent `sha256:590fcc601e8ad9e85ebd8b3b4575c67460b51f46e0bcf7d89f6bff57f9f05591`.
- Every pin is the exact `sha256:<64 lowercase hex>` string returned by `sha256_digest`; comparisons are string equality, never bare hex.
- FreeDOS version tag is `1.3`; the only supported `FROM freedos:<version>` value in this slice.
- The base config sets `dos_flavour=freedos`, `agent_version=1.0.0`, `booter=false`, `machine.disk_interface=ide`.
- The lock pin for the base is `name="freedos"`, `tag="1.3"`, and the base manifest digest; the agent pin is `version="1.0.0"`.
- Imported DOS images get `dos_flavour=other` and `agent_version=1.0.0`; a booter import stays opaque and gets no agent.
- `--update` refreshes lock pins; it does not rebuild a stored base. A stored base is reused only when its tag exists, its full config equals the expected config for `freedos:1.3` and agent `1.0.0`, and its boot metadata validates against the pinned MBR and boot-sector templates and the layout constants; otherwise it is rebuilt.
- OCI media types stay `application/vnd.dosker.layer.v1.tar`, `application/vnd.dosker.image.config.v1+json`, and `application/vnd.dosker.booter.v1.raw`; the raw-disk annotation stays `org.dosker.rawdisk.digest`.
- `COPY` and `RUN --host` timestamps stay `1980-01-01T00:00:00Z`; `C:\DOSKER` changes are discarded by later build steps.
- The base builder writes a fixed file list; raw-disk injection writes only the injection delta. Neither is a general tree-to-FAT projection, which stays out of this slice.
- Two-host digest equality is a CI/manual acceptance check, not a local unit test.

## Shared Interfaces

These names and signatures are used by more than one task and must not drift.

- `dosker_image::freedos::FreedosVersion`, `FreedosPins`, `BaseInputs`, `FreedosBinaries`, `BaseImage`.
- `dosker_image::freedos::load_freedos_binaries(&BaseInputs) -> Result<FreedosBinaries, ImageError>`.
- `dosker_image::freedos::build_base_image(&FreedosBinaries, &FreedosVersion) -> Result<BaseImage, ImageError>`.
- `dosker_image::freedos::ensure_freedos_base(&OciStore, Option<&BaseInputs>, &FreedosVersion) -> Result<Image, ImageError>`.
- `dosker_image::inject::plan_agent_injection(&Tree, &[u8]) -> Result<AgentInjection, ImageError>`.
- `dosker_image::inject::apply_agent_injection(&Path, &AgentInjection) -> Result<(), ImageError>`.
- `dosker_image::OciStore::import(&Path, &ImageReference, ImageMedia, Option<&[u8]>, DateTime<Utc>) -> Result<Image, ImageError>`.
- `dosker_build::Builder` local-import tag: `dosker-build:` plus the hex of `sha256_digest(source_digest || "\n" || agent_digest)`.
- `dosker_image::ANNOTATION_SOURCE_DISK_DIGEST`.
- `dosker_build::Builder::with_freedos_inputs(BaseInputs)` and `Builder::with_agent_executable(PathBuf)`.

## Tasks

- [x] **Task 1: Pinned FreeDOS inputs and agent executable**

    - **Implements:** FR-004; design §Build And Runtime Semantics (build), §Security And Privacy (FreeDOS built locally), §Testing Strategy (unit tests).
    - **Create `agent/fixtures/base/mbr.bin`** and **Create `agent/fixtures/base/boot-sector.bin`**
        - Extract once from the completed spike image, then treat as immutable pinned inputs:
            - `dd if=agent/fixtures/freedos-base.img of=agent/fixtures/base/mbr.bin bs=512 count=1`
            - `dd if=agent/fixtures/freedos-base.img of=agent/fixtures/base/boot-sector.bin bs=512 skip=2048 count=1`
        - Each file is 512 bytes. Their SHA-256 values must equal the two boot input pins in Constraints.
    - **Create `crates/dosker-image/src/freedos/inputs.rs`**
        - `pub const FREEDOS_VERSION: &str = "1.3"`
        - `pub const FREEDOS_KERNEL_SHA256`, `FREEDOS_COMMAND_SHA256`, `FREEDOS_AGENT_SHA256`, `FREEDOS_MBR_SHA256`, `FREEDOS_BOOT_SECTOR_SHA256` — the exact `sha256:<hex>` constants in Constraints.
        - `pub const MBR_TEMPLATE: &[u8] = include_bytes!("../../../../agent/fixtures/base/mbr.bin")` and `pub const BOOT_SECTOR_TEMPLATE: &[u8] = include_bytes!("../../../../agent/fixtures/base/boot-sector.bin")`.
        - `pub struct FreedosVersion(String)` deriving `Clone, Debug, PartialEq, Eq`, with `SUPPORTED: &[&str] = &["1.3"]`, `parse(&str) -> Result<Self, ImageError>` (exact match, otherwise `ImageError::UnsupportedFreedosVersion`), and `as_str(&self) -> &str`.
        - `pub struct FreedosPins { pub kernel_sha256: String, pub command_sha256: String, pub agent_sha256: String }` deriving `Clone, Debug`; `FreedosPins::freedos_1_3() -> Self` and `impl Default for FreedosPins`.
        - `pub struct BaseInputs { pub source: PathBuf, pub agent_executable: PathBuf, pub pins: FreedosPins }` deriving `Clone, Debug`; `BaseInputs::from_environment(&FreedosVersion) -> Result<Self, ImageError>` reads `DOSKER_FREEDOS_SOURCE` (missing is `ImageError::FreedosSourceNotSet`) and `DOSKER_AGENT_EXE`, defaults the agent path to `default_agent_executable()`, and sets `pins = FreedosPins::default()`.
        - `pub fn default_agent_executable() -> PathBuf` — `DOSKER_AGENT_EXE` when set and non-empty, otherwise `Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agent/build/DOSKER.EXE")`.
        - `pub fn load_agent_executable() -> Result<Vec<u8>, ImageError>` — reads `default_agent_executable()`; a missing file is `ImageError::FreedosFileMissing { path, file: "DOSKER.EXE" }`. It does not verify a pin because imported images may be tested with a substitute agent; the base loader verifies the pin.
        - `pub struct FreedosBinaries { pub kernel: Vec<u8>, pub command: Vec<u8>, pub agent: Vec<u8>, pub mbr: Vec<u8>, pub boot_sector: Vec<u8> }` deriving `Clone, Debug`.
        - `pub fn load_freedos_binaries(inputs: &BaseInputs) -> Result<FreedosBinaries, ImageError>` — checks the source is a file, reads `KERNEL.SYS` at the root, tries `COMMAND.COM` at `FREEDOS/BIN/COMMAND.COM` first and then at the root, reads the agent path, verifies each `sha256:<hex>` pin against `inputs.pins` by exact string equality, and copies `MBR_TEMPLATE` and `BOOT_SECTOR_TEMPLATE` into `Vec<u8>`. It uses `fat::read_file_if_present`: an absent named path becomes `FreedosFileMissing { path, file }`; a corrupt volume, `NotBootable`, and host I/O errors keep their own variants.
    - **Modify `crates/dosker-image/src/fat.rs`** — add `pub(crate) fn read_file_if_present(image: &Path, dos_path: &str) -> Result<Option<Vec<u8>>, ImageError>`. It mounts the volume and returns `Ok(None)` only when `open_file` reports `io::ErrorKind::NotFound`; every other mount or read error stays `ImageError::Corrupt`. `read_file` keeps its existing behavior.
    - **Create `crates/dosker-image/src/freedos/mod.rs`** — declares `pub mod inputs;`, re-exports the input symbols above, and re-exports `AGENT_VERSION` from `dosker_agent_proto` so the build engine uses one source for the agent version.
    - **Modify `crates/dosker-image/Cargo.toml`** — add `dosker-agent-proto.workspace = true`.
    - **Modify `crates/dosker-image/src/error.rs`** — add `FreedosSourceNotSet` (`DOSKER_FREEDOS_SOURCE is not set; set it to a local FreeDOS 1.3 boot floppy image`), `FreedosSourceMissing { path }` (the message names the path and says to set `DOSKER_FREEDOS_SOURCE` to a local boot floppy image), `FreedosFileMissing { path, file }`, `FreedosPinMismatch { file, expected, actual }` where both digests are `sha256:<hex>`, `UnsupportedFreedosVersion { version, supported }`, and `FreedosTemplate { reason }`, each with a message naming the file, path, or value.
    - **Modify `crates/dosker-image/src/lib.rs`** — `pub mod freedos;` and re-export `BaseInputs`, `FreedosBinaries`, `FreedosPins`, `FreedosVersion`, `FREEDOS_VERSION`, `load_freedos_binaries`, `default_agent_executable`, `load_agent_executable`.
    - **Create `crates/dosker-image/tests/freedos_inputs.rs`**
        - `embedded_templates_match_their_pins` — both committed template files hash to their constants and are 512 bytes; the MBR ends `55 AA` and its partition entry is bootable type 6 at LBA 2048 with 63 488 sectors.
        - `load_reads_the_kernel_and_command_from_a_source_image` — a support FAT image holding synthetic `KERNEL.SYS` and `COMMAND.COM`; the loader returns those bytes, the agent bytes, and the templates. Pins are computed from the synthetic bytes.
        - `load_prefers_the_freedos_bin_directory` — the source holds distinct `COMMAND.COM` and `FREEDOS/BIN/COMMAND.COM`; the loader returns the nested file.
        - `load_reports_a_missing_source_file` — a path that does not exist → `FreedosSourceMissing`.
        - `load_reports_a_missing_kernel` — a source image without `KERNEL.SYS` → `FreedosFileMissing { file: "KERNEL.SYS" }`.
        - `load_reports_a_missing_command` — a source without either command location → `FreedosFileMissing { file: "COMMAND.COM" }`.
        - `load_rejects_a_tampered_kernel` — the source kernel differs from the pinned `sha256:<hex>` → `FreedosPinMismatch { file: "KERNEL.SYS" }` with prefixed expected and actual values.
        - `load_rejects_a_tampered_agent` — the agent file differs from the pinned digest → `FreedosPinMismatch { file: "DOSKER.EXE" }`.
        - `load_keeps_a_corrupt_source_error` — an MBR image with a valid partition but a corrupted FAT returns `ImageError::Corrupt`, not `FreedosFileMissing`.
        - `base_inputs_report_a_missing_source_variable` — with a module `Mutex` guard, remove `DOSKER_FREEDOS_SOURCE` in the required `unsafe` env call, call `BaseInputs::from_environment(&FreedosVersion::parse("1.3").unwrap())`, assert `Err(ImageError::FreedosSourceNotSet)`, and restore the variable.
        - `base_inputs_read_the_source_and_agent_paths` — under the same guard, set both variables to temporary paths and assert the returned fields.
        - `version_accepts_only_the_pinned_release` — `"1.3"` parses; `"1.2"` and `"latest"` fail and name `1.3`.
        - `default_agent_path_is_the_workspace_build_output` — with `DOSKER_AGENT_EXE` removed under the module guard, the path ends `agent/build/DOSKER.EXE`.
        - Add `mod support;` and reuse `support::make_fat16_image` and `support::write_fat_file`.
    - **Verify:** `cargo test -p dosker-image --test freedos_inputs`

- [x] **Task 2: Deterministic base image builder**

    - **Implements:** FR-004; design §Component Interfaces (Image builder, FreeDOS builder), §Build And Runtime Semantics (build), §Risks (deterministic FreeDOS builder).
    - **Modify `crates/dosker-image/src/fat.rs`**
        - Make `PartitionFile` generic over `R: Read + Write + Seek` and `pub(crate)`, with `PartitionFile::new(inner: R, base: u64, length: u64)`.
        - Add `pub(crate) struct FatFixedTime` and `pub(crate) static FAT_FIXED_TIME`, implementing `fatfs::TimeProvider::get_current_date_time` as `1980-01-01 00:00:00.000`; expose `pub(crate) fn fixed_fs_options() -> FsOptions` that installs it.
        - Add `pub(crate) fn read_tree_from<R: Read + Write + Seek>(source: R) -> Result<Tree, ImageError>` and `pub(crate) fn read_boot_metadata_from<R: Read + Write + Seek>(source: R) -> Result<BootMetadata, ImageError>`, plus a private `fat_region_from` that detects the region from the first sector and the reader's end offset. The existing path-based `read_tree` and `read_boot_metadata` delegate to them.
        - Keep the existing read behavior for callers unchanged.
    - **Create `crates/dosker-image/src/freedos/build.rs`**
        - `pub const DISK_SIZE: u64 = 33_554_432;`, `PARTITION_START: u32 = 2048;`, `PARTITION_SECTORS: u32 = 63_488;`, `HEADS: u16 = 16;`, `SECTORS_PER_TRACK: u16 = 32;`, `BYTES_PER_SECTOR: u16 = 512;`, `BYTES_PER_CLUSTER: u32 = 512;`, `ROOT_DIR_ENTRIES: u16 = 512;`, `MEDIA_DESCRIPTOR: u8 = 0xF8;`, `VOLUME_ID: u32 = 0x0045_105C;`, `VOLUME_LABEL: [u8; 11] = *b"DOSKER     ";`.
        - `pub const CONFIG_SYS_BYTES: &[u8] = b"FILES=20\r\nBUFFERS=10\r\nLASTDRIVE=Z\r\n";`
        - `pub const BASE_AUTOEXEC_BYTES: &[u8] = b"@ECHO OFF\r\nSET COMSPEC=C:\\COMMAND.COM\r\nC:\\DOSKER\\DOSKER.EXE\r\n";`
        - `pub struct BaseImage { pub version: FreedosVersion, pub agent_version: String, pub raw: Vec<u8>, pub tree: Tree, pub boot: BootMetadata }` deriving `Clone, Debug`.
        - `pub fn build_base_image(binaries: &FreedosBinaries, version: &FreedosVersion) -> Result<BaseImage, ImageError>`:
            - Start from a zeroed `Vec<u8>` of `DISK_SIZE`; validate that the MBR template is 512 bytes, ends `55 AA`, and its first partition entry is `0x80`, type `0x06`, start LBA `2048`, sector count `63 488`; otherwise `ImageError::FreedosTemplate`.
            - Write the MBR template at offset 0.
            - Format the partition with `fatfs::format_volume` over a `PartitionFile` window at `PARTITION_START * 512` and `PARTITION_SECTORS * 512`, using `FatType::Fat16`, `bytes_per_sector(512)`, `total_sectors(63_488)`, `bytes_per_cluster(512)`, `fats(2)`, `max_root_dir_entries(512)`, `media(0xF8)`, `sectors_per_track(32)`, `heads(16)`, `drive_num(0x80)`, `volume_id(0x0045_105C)`, `volume_label(*b"DOSKER     ")`.
            - Mount the same window with `fixed_fs_options()` and write, in this order, `KERNEL.SYS`, `COMMAND.COM`, `CONFIG.SYS` (`CONFIG_SYS_BYTES`), `AUTOEXEC.BAT` (`BASE_AUTOEXEC_BYTES`), then `DOSKER` (directory) and `DOSKER/DOSKER.EXE` (agent bytes). Truncate before writing.
            - Validate that the boot-sector template is 512 bytes and ends `55 AA`; else `ImageError::FreedosTemplate`. Copy the freshly formatted boot sector's bytes `0x0B..0x3E` (the BPB and extended BPB) into the template, then write little-endian `2048` at offsets `28..32` (hidden sectors). Leave template bytes `0..0x0B` (jump and OEM name) and `0x3E..512` (FreeDOS loader and signature) untouched. Write the patched sector at the partition offset.
            - Read back `tree` with `read_tree_from` and `boot` with `read_boot_metadata_from` over the finished bytes; return them with `agent_version = dosker_agent_proto::AGENT_VERSION.to_owned()`.
    - **Modify `crates/dosker-image/src/freedos/mod.rs`** — add `pub mod build;` and re-export `BaseImage`, `build_base_image`, and the layout constants. **Modify `crates/dosker-image/src/lib.rs`** — re-export the new symbols.
    - **Create `crates/dosker-image/tests/freedos_build.rs`**
        - Add `mod support;` and build `FreedosBinaries` from synthetic kernel/command/agent bytes plus the committed templates.
        - `build_is_repeatable` — two builds of the same inputs produce equal `raw`, equal `Tree::digest()`, and equal boot metadata.
        - `build_has_the_pinned_geometry` — file length 33 554 432; partition entry bootable type 6 at LBA 2048 with 63 488 sectors; `read_boot_metadata` reports `MbrPartitioned`, `Fat16`, label `DOSKER`, media `0xF8`, geometry 124/16/32/512; raw BPB offsets `28..32` are 2048 and `19..21` are 63 488.
        - `build_copies_the_pinned_mbr` — raw bytes `0..512` equal `MBR_TEMPLATE`.
        - `build_keeps_the_freedos_boot_loader` — the boot sector's `0..0x0B` and `0x3E..512` equal the template's, and `510..512` are `55 AA`.
        - `build_writes_the_expected_tree` — exactly `AUTOEXEC.BAT`, `COMMAND.COM`, `CONFIG.SYS`, `DOSKER`, `DOSKER/DOSKER.EXE`, `KERNEL.SYS`; the file bytes are the inputs and constants; every entry's timestamp is `1980-01-01T00:00:00Z`; no read-only, hidden, system, or archive bit is set.
        - `build_writes_the_config_sys_settings` — the file bytes equal `CONFIG_SYS_BYTES`, including `LASTDRIVE=Z`, `FILES=20`, and `BUFFERS=10`.
        - `build_writes_an_autoexec_that_starts_the_agent` — the file bytes equal `BASE_AUTOEXEC_BYTES`.
        - `build_reports_a_mismatched_mbr` — a `FreedosBinaries` with a corrupted partition type returns `FreedosTemplate` naming the partition entry.
        - `build_reports_a_short_boot_template` — a 100-byte boot sector returns `FreedosTemplate`.
    - **Verify:** `cargo test -p dosker-image --test freedos_build`

- [x] **Task 3: Store the base and reuse it by tag**

    - **Implements:** FR-004; design §Build And Runtime Semantics (FROM resolves a built-in FreeDOS base), §Data Model (image config), §API Contracts (OCI media types); data model §Image Config, §BootMetadata, §Layer.
    - **Create `crates/dosker-image/src/freedos/store.rs`**
        - `pub(crate) fn expected_base_config(boot: &BootMetadata, agent_version: &str) -> ImageConfig`; `impl BaseImage { pub fn base_config(&self) -> ImageConfig; pub fn store(&self, store: &OciStore, reference: &ImageReference) -> Result<CommittedImage, ImageError> }`. `BaseImage::base_config` delegates to `expected_base_config(&self.boot, &self.agent_version)`.
        - `base_config` returns schema version 1, `dos_flavour = DosFlavour::Freedos`, `booter = false`, `boot = self.boot.clone()`, the default machine profile (its `disk_interface` is already `ide`), empty `env`, `workdir = "C:\\"`, no `cmd`, no ports or volumes, no engine pin or config, no requirements, `agent_version = Some(self.agent_version.clone())`, `created_by = "freedos-base"`, and one history entry with `created_at = fixed_timestamp()` and `empty_layer = false`.
        - `store` commits one tree layer built from `self.tree.to_delta()` with `created_by = "freedos-base"`, no parent, and no boot override; puts `self.raw` with `put_blob`; annotates `org.dosker.rawdisk.digest` with the raw blob digest; and calls `commit_image(Some(reference), &config, vec![descriptor], annotations)`.
        - `pub fn ensure_freedos_base(store: &OciStore, inputs: Option<&BaseInputs>, version: &FreedosVersion) -> Result<Image, ImageError>`:
            - Tag `freedos:<version>`. If the tag exists, load its config and manifest. Reuse it only when: the stored config equals `expected_base_config(&stored.boot, AGENT_VERSION)` in full; `validate_base_boot(&stored.boot)` succeeds; the manifest has a raw-disk digest; and that blob exists. Return the loaded image.
            - Otherwise take the supplied inputs, or call `BaseInputs::from_environment(version)?` only now, then load them with `load_freedos_binaries`, build with `build_base_image`, store with `BaseImage::store`, and return the loaded image. A valid stored base never touches `inputs` or the environment; a rebuild requires the source and agent.
            - An unknown version is rejected by `FreedosVersion::parse` before this function is called.
    - **Modify `crates/dosker-image/src/freedos/build.rs`** — add `pub(crate) fn validate_base_boot(boot: &BootMetadata) -> Result<(), ImageError>`: requires the MBR-partitioned layout, disk size 33 554 432, geometry 124/16/32/512, partition start 2048 and 63 488 sectors, `Fat16`, label `DOSKER`, media `0xF8`; the MBR inline bytes equal `MBR_TEMPLATE` and verify against their digest; the boot-sector inline bytes are 512 bytes with the template's `0..0x0B` and `0x3E..512` bytes, signature `55 AA`, hidden sectors 2048, total sectors 63 488, 1 sector per cluster, 1 reserved sector, 2 FATs, 512 root entries, nonzero sectors per FAT, media `0xF8`, 16 heads, 32 sectors per track, drive `0x80`, extended boot signature `0x29`, volume id `0x0045105C`, label `DOSKER     `, and file-system type `FAT16   `. Decode `bytes_inline` with `base64::engine::general_purpose::STANDARD`; a missing or undecodable value is `ImageError::FreedosTemplate`.
    - **Modify `crates/dosker-image/src/freedos/mod.rs`** — add `pub mod store;` and export `ensure_freedos_base` and `BaseImage::store`. **Modify `crates/dosker-image/src/lib.rs`** — re-export them.
    - **Create `crates/dosker-image/tests/freedos_store.rs`**
        - Use a synthetic `BaseImage` from `build_base_image` with synthetic pins.
        - `store_commits_a_freedos_config` — loaded config: `DosFlavour::Freedos`, `booter == false`, `agent_version.as_deref() == Some("1.0.0")`, `boot == image.boot`, `machine.disk_interface == "ide"`, `workdir == "C:\\"`, `created_by == "freedos-base"`, one history entry at the fixed timestamp.
        - `store_round_trips_the_tree_and_raw_disk` — `store.load_tree` equals `image.tree`; `store.copy_disk_to` bytes equal `image.raw`; `manifest.raw_disk_digest()` equals `sha256_digest(&image.raw)`; the layer parent is `None`.
        - `store_is_repeatable` — two stores from the same inputs produce the same manifest digest, config digest, layer digest, and raw blob digest.
        - `ensure_builds_and_tags_a_missing_base` — an empty store and `Some(&synthetic_inputs)` create `freedos:1.3`; the returned image has the agent version.
        - `ensure_reuses_a_valid_base` — a second call with `None` inputs still succeeds because the full config and boot validate; the manifest digest is unchanged. The no-input tests in this file share a module `Mutex` and remove/restore `DOSKER_FREEDOS_SOURCE` with the required `unsafe` env call.
        - `ensure_reports_missing_inputs_when_no_base` — an empty store with `None` inputs and no `DOSKER_FREEDOS_SOURCE` returns `ImageError::FreedosSourceNotSet`.
        - `ensure_rebuilds_a_stale_base` — commit a `freedos:1.3` config with `DosFlavour::Freedos` and `agent_version = Some("0.0.0")`; `ensure` with `Some(&synthetic_inputs)` replaces it with a base whose agent version is `1.0.0`.
        - `ensure_stale_base_requires_inputs` — the same stale config with `None` inputs and no `DOSKER_FREEDOS_SOURCE` returns `FreedosSourceNotSet` instead of reusing it.
        - `ensure_rebuilds_a_base_with_a_changed_config` — commit a valid base with `workdir = "C:\\APP"`; `ensure` with `Some(&synthetic_inputs)` replaces it with the expected config (`workdir = "C:\\"`).
        - `ensure_rebuilds_a_base_with_a_changed_boot_sector` — commit a valid base whose inline boot-sector loader byte is flipped; `validate_base_boot` fails and `ensure` with `Some(&synthetic_inputs)` rebuilds.
    - **Verify:** `cargo test -p dosker-image --test freedos_store`

- [x] **Task 4: Resolve `FROM freedos:<version>` in the build engine**

    - **Implements:** FR-004; design §Build And Runtime Semantics (`FROM` resolves a built-in FreeDOS base), §Error Handling And Edge Cases (version mismatch, `--update`), §Data Model (lock file).
    - **Modify `crates/dosker-build/src/build.rs`**
        - Add `freedos_inputs: Option<BaseInputs>` to `Builder`; initialize it to `None` in `new` and `with_resolver`.
        - Add `pub fn with_freedos_inputs(mut self, inputs: BaseInputs) -> Self`.
        - In `resolve_base`, replace the `FromSource::Freedos` error with: parse the version with `FreedosVersion::parse`, call `ensure_freedos_base(&self.store, self.freedos_inputs.as_ref(), &version)` **without** calling `BaseInputs::from_environment` first, and load the result with `self.load_stored_base(&format!("freedos:{}", version.as_str()))` so the returned base pin is `freedos`, `1.3`, and the manifest digest. `ensure_freedos_base` checks the stored tag before it reads any input or environment value.
    - **Modify `crates/dosker-build/src/lib.rs`** — re-export nothing new unless the tests need `BaseInputs`; tests may import it from `dosker_image`.
    - **Modify `crates/dosker-build/tests/support/mod.rs`**
        - Add `pub fn write_freedos_source(path: &Path)` — a 4 MiB FAT16 superfloppy holding `KERNEL.SYS` and `COMMAND.COM` with fixed contents; return those contents so tests can compute pins.
        - Add `pub fn write_agent(path: &Path) -> Vec<u8>` — a small fake agent.
        - Add `pub fn synthetic_inputs(source: &Path, agent: &Path) -> BaseInputs` — pins computed with `sha256_digest`.
        - Move `FakeResolver` and `TestReporter` from `tests/build.rs` into this module as public items; `build.rs` and `freedos_base.rs` import them.
    - **Modify `crates/dosker-build/tests/build.rs`**
        - Remove `build_rejects_a_freedos_base`; the success path moves to `freedos_base.rs`.
    - **Create `crates/dosker-build/tests/freedos_base.rs`**
        - `from_freedos_builds_and_stores_the_base` — a context with `FROM freedos:1.3\n`; the build succeeds; the store holds `freedos:1.3`; the config is `DosFlavour::Freedos` with agent `1.0.0`; the build outcome digest equals the base manifest digest.
        - `from_freedos_writes_the_base_and_agent_pins` — the lock TOML has `name = "freedos"`, `tag = "1.3"`, a `sha256:` digest, and `[agent] version = "1.0.0"`.
        - `from_freedos_reuses_a_stored_base_without_source` — build once with `with_freedos_inputs`; remove `DOSKER_FREEDOS_SOURCE` under a `Mutex` guard and the required `unsafe` env call; build again with a `Builder` that has no `with_freedos_inputs`; the build succeeds, reuses the stored tag, and reports the same digest. This is the no-source path, not a nonexistent explicit path.
        - `from_freedos_rejects_an_unknown_version` — `FROM freedos:1.2` fails and the message contains `supported: 1.3`.
        - `from_freedos_requires_the_source_when_no_base_exists` — an empty store, a `Builder` without `with_freedos_inputs`, and `DOSKER_FREEDOS_SOURCE` removed fail with `FreedosSourceNotSet`; no lock file is written. An explicit `BaseInputs` whose source path is missing still fails with `FreedosSourceMissing` naming the path. This test, the reuse test above, and the stale-rebuild test below share one module `Mutex` and restore the variable.
        - `from_freedos_requires_inputs_to_rebuild_a_stale_base` — a stale `freedos:1.3`, a `Builder` without `with_freedos_inputs`, and `DOSKER_FREEDOS_SOURCE` removed fail with `FreedosSourceNotSet`; with `with_freedos_inputs` the same build rebuilds and succeeds. Use the shared env guard.
        - `from_freedos_refreshes_a_moved_pin_with_update` — pre-write a lock pin for `freedos:1.3` with a wrong digest; `update: false` returns `LockMismatch`; `update: true` succeeds and writes the actual digest.
    - **Verify:** `cargo test -p dosker-build --test freedos_base` and `cargo test -p dosker-build --test build`

- [x] **Task 5: Agent injection plan for imported DOS images**

    - **Implements:** FR-005; design §Build And Runtime Semantics (`FROM` imported DOS image gets the agent injection layer), contract §FROM / §Agent Injection For Imported DOS Images.
    - **Create `crates/dosker-image/src/inject.rs`**
        - `pub const AGENT_DIR: &str = "DOSKER";`, `pub const AGENT_FILE: &str = "DOSKER.EXE";`, `pub const AGENT_PATH: &str = r"C:\DOSKER\DOSKER.EXE";`, `pub const AUTOEXEC_PATH: &str = r"C:\AUTOEXEC.BAT";`, `pub const AUTOEXEC_OLD_PATH: &str = r"C:\DOSKER\AUTOEXEC.OLD";`.
        - `pub const WRAPPED_AUTOEXEC_BYTES: &[u8] = b"@ECHO OFF\r\nCALL C:\\DOSKER\\AUTOEXEC.OLD\r\nC:\\DOSKER\\DOSKER.EXE\r\n";`
        - `pub const NEW_AUTOEXEC_BYTES: &[u8] = b"@ECHO OFF\r\nC:\\DOSKER\\DOSKER.EXE\r\n";`
        - `pub struct AgentInjection { pub applied: bool, pub delta: LayerDelta }` deriving `Clone, Debug`.
        - `pub fn plan_agent_injection(tree: &Tree, agent: &[u8]) -> Result<AgentInjection, ImageError>`:
            - `C:\DOSKER` as a file, `C:\DOSKER\DOSKER.EXE` as a directory, `C:\AUTOEXEC.BAT` as a directory, or `C:\DOSKER\AUTOEXEC.OLD` as a directory is `ImageError::AgentInjection { reason }` naming the path.
            - Add `C:\DOSKER` as a directory only when the tree has no such path.
            - Add `C:\DOSKER\DOSKER.EXE` only when the existing file is absent or its bytes differ from `agent`. Always use the fixed timestamp. A new file uses default attributes; a replacement inherits the existing file's attributes, matching the FAT writer's open-in-place behavior.
            - If `C:\AUTOEXEC.BAT` exists and `starts_agent` is true for its bytes, leave `AUTOEXEC.BAT` untouched.
            - Else if `C:\AUTOEXEC.BAT` exists, add `C:\DOSKER\AUTOEXEC.OLD` with the original bytes, fixed timestamp, and default attributes; replace `AUTOEXEC.BAT` with `WRAPPED_AUTOEXEC_BYTES`, keeping the original file's attributes and using the fixed timestamp. An existing `C:\DOSKER\AUTOEXEC.OLD` is `ImageError::AgentInjection` naming the path.
            - Else add `AUTOEXEC.BAT` with `NEW_AUTOEXEC_BYTES`, default attributes, and the fixed timestamp.
            - `applied` is true when the delta has any file or directory entry.
        - `pub fn starts_agent(bytes: &[u8]) -> bool` — splits on `\r\n` and `\n`, trims each line, uppercases, splits on ASCII whitespace into tokens, drops one optional leading `CALL`, and returns true when the next token equals `C:\DOSKER\DOSKER.EXE`. Trailing tokens are arguments and are ignored. A line such as `REM C:\DOSKER\DOSKER.EXE` does not match, and neither does a token such as `C:\DOSKER\DOSKER.EXE.BAK` or `C:\DOSKER\AUTOEXEC.OLD`. Public for tests.
    - **Modify `crates/dosker-image/src/error.rs`** — add `AgentInjection { reason: String }` with message `agent injection: {reason}`.
    - **Modify `crates/dosker-image/src/lib.rs`** — `pub mod inject;` and re-export `AGENT_PATH`, `AUTOEXEC_OLD_PATH`, `AgentInjection`, `plan_agent_injection`, `starts_agent`.
    - **Create `crates/dosker-image/tests/inject.rs`**
        - `injection_creates_the_agent_directory_and_file` — an empty tree yields `DOSKER`, `DOSKER/DOSKER.EXE`, and `AUTOEXEC.BAT` (`NEW_AUTOEXEC_BYTES`), all at the fixed timestamp and with default attributes.
        - `injection_wraps_an_existing_autoexec` — `AUTOEXEC.BAT` = `@ECHO OFF\r\n` with `read_only` and `archive` set; the delta holds `AUTOEXEC.OLD` with the original bytes, default attributes, and the fixed timestamp, and `AUTOEXEC.BAT` = `WRAPPED_AUTOEXEC_BYTES` with the original `read_only`/`archive` attributes and the fixed timestamp; `applied` is true.
        - `injection_keeps_the_wrap_out_of_an_absent_autoexec` — no `AUTOEXEC.OLD` is added when there is no original.
        - `injection_detects_an_existing_agent_line` — `AUTOEXEC.BAT` = `@ECHO OFF\r\nc:\\dosker\\dosker.exe\r\n`; the delta has no `AUTOEXEC.BAT` change and no `AUTOEXEC.OLD`; `applied` is false when the agent file already matches.
        - `injection_replaces_a_different_agent_executable` — the tree has `DOSKER/DOSKER.EXE` with other bytes and `hidden`/`archive` set; the delta replaces it with the supplied bytes, keeps those attributes, and uses the fixed timestamp.
        - `injection_is_idempotent` — plan and apply the delta to the tree; a second plan returns an empty delta and `applied == false`.
        - `injection_rejects_a_dosker_file` and `injection_rejects_an_existing_old_batch` — `ImageError::AgentInjection` naming the conflicting path.
        - `starts_agent_matches_the_agent_command` — the bare line with case and surrounding spaces matches; `C:\DOSKER\DOSKER.EXE /TEST` matches; `CALL C:\DOSKER\DOSKER.EXE` matches; `CALL C:\DOSKER\DOSKER.EXE /TEST` matches; `C:\DOSKER\AUTOEXEC.OLD`, `C:\DOSKER\DOSKER.EXE.BAK`, and `REM C:\DOSKER\DOSKER.EXE` do not.
        - `injection_detects_the_self_test_agent_command` — `AUTOEXEC.BAT` = `@ECHO OFF\r\nC:\DOSKER\DOSKER.EXE /TEST\r\n`; no wrap, no `AUTOEXEC.OLD`, and `applied` is false when the agent file already matches. This is the `dosker-agent-test.img` startup shape.
        - `injection_detects_a_call_with_arguments` — `AUTOEXEC.BAT` = `CALL C:\DOSKER\DOSKER.EXE /TEST\r\n`; no wrap and no `AUTOEXEC.OLD`.
    - **Verify:** `cargo test -p dosker-image --test inject`

- [x] **Task 6: Raw-disk injection and `dosker image import` integration**

    - **Implements:** FR-005; design §Build And Runtime Semantics (import gets the agent injection layer; the canonical image is a tree plus disk metadata), §Data Model (raw disk annotation), contract §FROM (agent injection, imported image requirements).
    - **Modify `crates/dosker-image/src/inject.rs`**
        - `pub fn apply_agent_injection(image: &Path, injection: &AgentInjection) -> Result<(), ImageError>`:
            - Return immediately when `injection.applied` is false.
            - Locate the FAT region with `fat::fat_region`, open the image read/write, wrap it in `fat::PartitionFile`, and mount with `fat::fixed_fs_options()`.
            - Create the delta directories first, then for each file open or create the path, truncate, and write the bytes. Opening an existing file preserves its attributes; the delta carries the same attributes, so the raw disk and the tree agree. Use `DosPath::as_str()` as the FAT path. Unmount and propagate mount/write errors as `ImageError::Corrupt`.
            - The MBR, partition table, boot sector, and all files outside the delta must remain byte-identical.
    - **Modify `crates/dosker-image/src/manifest.rs`**
        - Add `pub const ANNOTATION_SOURCE_DISK_DIGEST: &str = "org.dosker.source.digest";` and `Manifest::source_disk_digest(&self) -> Option<&str>` reading that annotation.
    - **Modify `crates/dosker-image/src/oci.rs`**
        - Change `import` to `import(&self, source: &Path, reference: &ImageReference, media: ImageMedia, agent: Option<&[u8]>, now: DateTime<Utc>) -> Result<Image, ImageError>`.
        - Booter path is unchanged: store the opaque blob, set `booter=true`, `DosFlavour::None`, no agent, one `MEDIA_TYPE_BOOTER` layer, no raw-disk or source annotation.
        - Disk path:
            - `ImageMedia::Disk` with `None` agent returns `ImageError::AgentMissing` (`a disk import requires the agent executable`).
            - Read the original tree with `read_tree(source)` and plan with `plan_agent_injection(&original_tree, agent)`.
            - When the plan applies, copy the source to a temporary file beside the blobs named `.import.` plus the process id and a counter, apply the injection, store that file with `put_blob_from_file`, and remove the temporary file on success and on error. Otherwise store the source with `put_blob_from_file`.
            - Resolve `let blob_path = store.blob_path(raw_digest)?;`; read the injected tree with `read_tree(&blob_path)` and the boot metadata with `read_boot_metadata(&blob_path)`; the boot metadata must equal the source's.
            - Build the source layer from `original_tree.to_delta()` with `created_by = "import"` and no parent. When the plan applies, build a second layer from `injection.delta` with `created_by = "agent-inject"` and the first layer digest as parent.
            - Set `config.agent_version = Some(dosker_agent_proto::AGENT_VERSION.to_owned())`, `config.boot = boot`, `config.dos_flavour = DosFlavour::Other`, and the disk interface from the boot layout.
            - Annotate `org.dosker.rawdisk.digest` with `raw_digest` and `org.dosker.source.digest` with the SHA-256 of the original source file (add a private buffered `sha256_file` helper).
            - Keep the existing `created` blob cleanup on error.
    - **Modify `crates/dosker-image/src/error.rs`** — add `AgentMissing` (`a disk import requires the agent executable`).
    - **Modify `crates/dosker-image/src/lib.rs`** — re-export `ANNOTATION_SOURCE_DISK_DIGEST` and `apply_agent_injection`.
    - **Modify `crates/dosker-image/tests/support/mod.rs`**
        - Add `pub fn set_fat_attribute(image: &Path, partition_offset: u64, short_name: &str, attribute: u8)` — read the BPB from the partition boot sector, compute the root-directory sector, scan 32-byte entries for the 11-byte short name, and write the attribute byte. Use it to mark a source fixture `AUTOEXEC.BAT` with `0x21` (`READ_ONLY | ARCHIVE`).
    - **Modify `crates/dosker-image/tests/oci.rs`**
        - Add `fn agent() -> Vec<u8> { b"MZ synthetic agent".to_vec() }` and pass `Some(agent().as_slice())` to every `ImageMedia::Disk` import and `None` to every booter import.
        - `import_disk_reads_the_fat_tree_and_boot_metadata` now expects two layers; `import_writes_the_raw_disk_annotation` also asserts the source annotation and that neither annotation is a layer digest.
        - `copy_disk_to_copies_the_annotated_raw_disk` — import with `Some(agent())`; copy with `copy_disk_to`; compare the copied bytes to `fs::read(store.blob_path(image.raw_disk_digest.as_deref().unwrap())?)`, not to the source; assert the copied bytes differ from the source; `read_tree(&destination)` has `DOSKER/DOSKER.EXE` with the agent bytes and `AUTOEXEC.BAT` equal to `NEW_AUTOEXEC_BYTES` (the fixture has no original `AUTOEXEC.BAT`). Add `read_tree` to the test imports.
        - Add `import_rejects_a_disk_without_an_agent` — `ImageMedia::Disk` with `None` returns `AgentMissing`.
        - Keep `import_is_deterministic_for_the_same_source_and_time` and `import_leaves_the_source_unchanged`.
    - **Create `crates/dosker-image/tests/import_agent.rs`**
        - Use `support::make_fat16_image`, `support::write_fat_file`, and `support::set_fat_attribute` to build a 16 MiB MBR-partitioned FAT16 image with `AUTOEXEC.BAT = b"@ECHO OFF\r\n"` marked `0x21`, and `APP/FOO.TXT` with default attributes. Read the source tree once before injection; the fixture's `AUTOEXEC.BAT` carries `read_only` and `archive`.
        - `import_injects_the_agent_into_a_dos_image` — the tree has `DOSKER`, `DOSKER/DOSKER.EXE` with the supplied bytes and default attributes, `DOSKER/AUTOEXEC.OLD` with the original bytes, default attributes, and the fixed timestamp, and `AUTOEXEC.BAT` equals `WRAPPED_AUTOEXEC_BYTES` with the original `read_only`/`archive` attributes and the fixed timestamp; the config is `DosFlavour::Other` with agent `1.0.0`; two layers; the raw and source annotations differ.
        - `import_tree_matches_the_injected_raw_disk` — `store.load_tree` equals `read_tree(&store.blob_path(image.raw_disk_digest.as_deref().unwrap())?)`, including the preserved `AUTOEXEC.BAT` attributes and the fixed timestamps. Use the same stored raw blob for the read-back and the attribute fixture, so the comparison is deterministic.
        - `import_preserves_the_mbr_partition_and_boot_sector` — the injected raw has the same length; bytes before the partition and the boot sector region equal the source; `read_boot_metadata(injected)` equals `read_boot_metadata(source)`.
        - `import_does_not_inject_twice` — use `store.copy_disk_to` to write the injected raw to a file and import it under a second tag with the same agent; the tree and raw digest are unchanged, `AUTOEXEC.BAT` contains one agent line, and no second `AUTOEXEC.OLD` is created.
        - `import_writes_a_new_autoexec_when_none_exists` — a source without `AUTOEXEC.BAT` gets `NEW_AUTOEXEC_BYTES` and no `AUTOEXEC.OLD`.
        - `import_booter_stays_opaque` — `ImageMedia::Booter` returns a booter config, no tree, no agent, one booter layer, and the source digest as the layer digest.
        - `import_rejects_an_unreadable_disk` — a blank file returns `NotBootable`.
    - **Modify `crates/dosker-build/src/build.rs`**
        - Add `agent_executable: Option<PathBuf>` to `Builder`; initialize it to `None` in `new` and `with_resolver`; add `pub fn with_agent_executable(mut self, path: PathBuf) -> Self`.
        - In `import_base`, for `ImageMedia::Disk` load the agent with `load_agent_executable()` when `self.agent_executable` is `None`, or `fs::read` the configured path (mapping a failure to `BuildError::Io`); pass `Some(agent.as_slice())` to `store.import`. For `ImageMedia::Booter` pass `None`.
        - Compute the local-import tag from both inputs: `source_digest = hash_file(&path)`, `agent_digest = sha256_digest(&agent)`, `combined = sha256_digest(format!("{source_digest}\n{agent_digest}").as_bytes())`, and `ImageReference::parse(&format!("dosker-build:{}", combined.strip_prefix("sha256:").unwrap()))`. A booter keeps the source-only key. A changed agent therefore produces a different tag and a new import.
        - Delete the existing post-load check that compares `manifest.raw_disk_digest()` with `source_digest`; an injected raw disk legitimately differs from the source. Reuse a found combined-tag image only when `manifest.source_disk_digest() == Some(source_digest.as_str())`, `config.agent_version.as_deref() == Some(dosker_image::freedos::AGENT_VERSION)`, and the raw-disk blob exists; otherwise untag it with `store.remove(&ImageRef::Tag(reference))` and import again so the source annotation and agent are present. The `source_disk_digest().or_else(|| raw_disk_digest())` fallback is not a reuse path in this slice: a stored image without `org.dosker.source.digest` is a pre-slice image that may lack the agent, so it is re-imported.
        - A pre-slice image under the old source-only tag is not found by the combined key and is imported again; it stays in the store and can be removed with `dosker rmi`. This same re-import rule covers a raw-only image found under the combined tag without the source annotation.
    - **Modify `crates/dosker-build/tests/build.rs`**
        - `Fixture::new` writes a fake agent at the fixture context's `agent/DOSKER.EXE`; add `Fixture::agent_path()`; `Fixture::build` uses `Builder::with_resolver(self.root_path().to_path_buf(), Box::new(FakeResolver)).with_agent_executable(self.agent_path().to_path_buf())`.
        - `build_imports_a_local_disk_image` now expects the injected `C:\DOSKER\DOSKER.EXE`, the wrapped `AUTOEXEC.BAT`, and `C:\DOSKER\AUTOEXEC.OLD`; the raw-disk annotation stays present.
        - Add `build_from_a_local_import_pins_the_agent` — a `FROM ./base.img` build with the fake agent writes a lock whose `[agent]` version is `1.0.0`.
        - Add `build_reimports_a_local_image_when_the_agent_changes` — build once with agent A, then build again with agent B; the second build imports under a different tag, the imports count is 2, and the built tree's `C:\DOSKER\DOSKER.EXE` equals B.
        - Add `build_reimports_a_pre_slice_local_import` — store a raw-only image under `dosker-build:<source hex>` with no source annotation, then build `FROM ./base.img`; the new combined tag is imported, the tree has `C:\DOSKER\DOSKER.EXE`, and the old tag remains.
        - Add `build_reimports_a_combined_tag_without_the_source_annotation` — store a raw-only image with `agent_version = Some("1.0.0")` but no source annotation under the combined tag computed with the same formula; the build re-imports, the new tree has `C:\DOSKER\DOSKER.EXE`, and the new manifest has the source annotation.
        - `build_second_run_from_a_local_import_reuses_the_image` still imports once: the first import writes the source annotation, and the second build finds the combined tag with matching source and agent and reuses it without hitting a raw-vs-source check.
    - **Modify `crates/dosker-cli/src/commands/image.rs`**
        - For `ImageMedia::Disk`, load the agent with `dosker_image::freedos::load_agent_executable()`; for `ImageMedia::Booter`, pass `None`. Keep the existing output line and exit codes.
    - **Modify `crates/dosker-test-support/src/fixtures.rs`**
        - Add `pub fn agent_executable() -> Result<PathBuf, String>` resolving `agent/build/DOSKER.EXE` from the workspace root and reporting a clear error when it is absent.
    - **Modify `crates/dosker-cli/tests/common/mod.rs`**
        - `CliRoot::new` writes a small fake agent at the CLI root's `agent/DOSKER.EXE` and `base_command` sets `DOSKER_AGENT_EXE` to it.
        - `command_with_qemu` overrides `DOSKER_AGENT_EXE` with `dosker_test_support::fixtures::agent_executable()`, so the QEMU tests inject the real agent.
    - **Modify `crates/dosker-cli/tests/cli_build.rs`**
        - `build_writes_a_version_one_lock_file` now expects `assert_eq!(lock.agent, Some(AgentPin { version: "1.0.0".to_owned() }));`; add `AgentPin` to the `dosker_core` import.
    - **Modify `crates/dosker-cli/tests/cli_image.rs`**
        - `image_import_accepts_the_mbr_cli_fixture` also asserts the imported config has agent `1.0.0` and the store tree has `C:\DOSKER\DOSKER.EXE`.
        - `image_import_booter_accepts_an_opaque_blob` asserts the config has no agent.
        - Keep the printed-size assertion: injection preserves the file length.
    - **Verify:** `cargo test -p dosker-image --test oci` and `cargo test -p dosker-image --test import_agent` and `cargo test -p dosker-build --test build` and `cargo test -p dosker-cli --test cli_image` and `cargo test -p dosker-cli --test cli_build`

- [x] **Task 7: Boot and exec integration coverage**

    - **Implements:** FR-004, FR-005; roadmap slice `freedos-base-builder` (`Done when`); design §Testing Strategy (real-engine integration).
    - **Modify `crates/dosker-test-support/src/fixtures.rs`**
        - Add `pub const FREEDOS_SOURCE_ENV: &str = "DOSKER_FREEDOS_SOURCE";` and `pub const FREEDOS_BASE_IMAGE_ENV: &str = "DOSKER_FREEDOS_BASE_IMAGE";`.
        - Add `pub fn freedos_base_image() -> Result<PathBuf, String>`: read `DOSKER_FREEDOS_BASE_IMAGE`, default to `agent/fixtures/freedos-base.img`, resolve a relative value against the workspace root like `qemu_image`, and error when the file is absent.
    - **Create `crates/dosker-cli/tests/qemu_freedos.rs`** (`#![cfg(unix)]`, `mod common;`)
        - Reuse the integration gate pattern from `qemu_build.rs`: skip when `DOSKER_INTEGRATION != 1`; panic when it is enabled but the inputs named by the environment are absent.
        - `built_freedos_base_boots_and_answers_exec` — context with `FROM freedos:1.3\n`; run `build -t freedos-built:latest` through `root.command_with_qemu()` with `.env("DOSKER_FREEDOS_SOURCE", source)`; then `run -d --name freedos-check freedos-built:latest`, `exec freedos-check ECHO HELLO`, assert the output contains `HELLO` and status 0, then `stop`. This is the base-to-agent handshake.
        - `imported_dos_image_with_injected_agent_boots_and_answers_exec` — resolve the source with `fixtures::freedos_base_image()` and import it as `injected-image:latest`; `run -d --name injected-check injected-image:latest`; `exec injected-check ECHO HELLO`; assert `HELLO` and status 0; `stop`.
        - Use the shared integration lock so only one guest boots at a time.
    - **Verify:** `DOSKER_INTEGRATION=1 DOSKER_QEMU=qemu-system-i386 DOSKER_FREEDOS_SOURCE="$DOSKER_FREEDOS_SOURCE" DOSKER_FREEDOS_BASE_IMAGE=agent/fixtures/freedos-base.img cargo test -p dosker-cli --features qemu-engine --test qemu_freedos -- --nocapture`

- [x] **Task 8: Documentation and two-host acceptance wiring**

    - **Implements:** FR-004, FR-005; design §Security And Privacy (FreeDOS built locally), §Testing Strategy (CI, integration), §Goals (reproducible builds).
    - **Create `docs/base-images.md`**
        - Inputs: `DOSKER_FREEDOS_SOURCE` (`144m/x86BOOT.img` from the FreeDOS 1.3 Floppy Edition ZIP, with the ZIP URL and the pinned SHA-256 `75a4e11a7fce6f124e20927b3022b4b715a2a3f7324c5f5bfea42d90d80eb072`), `DOSKER_AGENT_EXE` (default `agent/build/DOSKER.EXE`), and the committed MBR and boot-sector templates.
        - Pin table: the five `sha256:<hex>` values and byte sizes; the FreeDOS version `1.3` and the agent version `1.0.0`.
        - Pin format and update procedure: every pin is the exact `sha256:<hex>` string returned by `sha256_digest`; to update a pin, replace the source binary or template, recompute with `shasum -a 256` or `sha256sum`, update the constant, rerun `cargo test -p dosker-image --test freedos_inputs`, and rerun the two-host determinism job. Re-extract `agent/fixtures/base/{mbr,boot-sector}.bin` only when `agent/fixtures/freedos-base.img` changes.
        - Agent-artifact pin check: `load_freedos_binaries` compares the agent file with `FREEDOS_AGENT_SHA256`; the CI jobs download one `dosker-agent-binaries` artifact so both hosts use the same bytes; a mismatch is a build error naming the expected and actual digests.
        - Source input: the CLI requires `DOSKER_FREEDOS_SOURCE` to be a local boot floppy image only when the base must be built. A stored `freedos:1.3` base is reused without the source or agent inputs. A URL or ZIP is a CI repository-variable value only; `.github/scripts/fetch-freedos-source.sh` resolves it to a local path before the build. An old source-only `dosker-build:` tag from before this slice is not reused and can be removed with `dosker rmi`.
        - Layout: 32 MiB, partition LBA 2048, 63 488 sectors, type 6, FAT16, hidden sectors 2048, label `DOSKER`, volume id, geometry.
        - Files: the exact `CONFIG.SYS` and `AUTOEXEC.BAT` content and the fixed timestamp.
        - `FROM freedos:1.3`: reuse by tag without source or agent inputs, the lock pin shape, `--update` refreshing a moved pin, and rebuilding by removing the tag.
        - Determinism: a repeat build in two roots is identical; CI compares the digest from two hosts.
    - **Modify `docs/images.md`**
        - Under Import, describe the agent injection: `C:\DOSKER\DOSKER.EXE`, the wrapped `AUTOEXEC.BAT` with `C:\DOSKER\AUTOEXEC.OLD`, the no-duplicate rule, the fixed timestamp, the `dos_flavour=other` and agent version, and the booter exception.
        - Document `org.dosker.source.digest` beside `org.dosker.rawdisk.digest`.
    - **Modify `README.md`**
        - Extend the FreeDOS source section: the repository variable may be a URL or ZIP for CI, but the CLI loader requires a local boot floppy image when a base must be built (CI resolves it first). A stored `freedos:1.3` base is reused without the source or agent inputs. The same local path feeds a first build or a rebuild, and the agent binary is required at `agent/build/DOSKER.EXE` or through `DOSKER_AGENT_EXE`. Link `docs/base-images.md`.
    - **Create `.github/scripts/fetch-freedos-source.sh`**
        - Read `DOSKER_FREEDOS_SOURCE`; accept a ZIP URL (verify the pinned SHA-256 with `sha256sum` or `shasum -a 256`, extract `144m/x86BOOT.img`), a raw image URL, or a local path; print the image path on stdout; fail with a message naming the variable and input on every error.
    - **Modify `.github/workflows/ci.yml`**
        - In `agent-qemu`, replace only the inline source download block with `source_path="$(bash .github/scripts/fetch-freedos-source.sh)"`, then `export DOSKER_FREEDOS_SOURCE="$source_path"` before `agent/tools/build-base-image.sh` runs, followed by the existing `printf 'DOSKER_FREEDOS_SOURCE=%s\n' "$source_path" >> "$GITHUB_ENV"`. Keep the existing `Build the FreeDOS base image` step (`agent/tools/build-base-image.sh`) and the `base-image-test.sh` and `fixture-test.sh` steps; they still run after the fetch and still produce `agent/fixtures/freedos-base.img` for the import/base-image tests. Add a step `Rust FreeDOS base and injection suite` that runs `cargo test -p dosker-cli --features qemu-engine --test qemu_freedos -- --nocapture` with `DOSKER_INTEGRATION=1`, `DOSKER_QEMU=qemu-system-i386`, and `DOSKER_FREEDOS_BASE_IMAGE=agent/fixtures/freedos-base.img`.
        - Add `freedos-base-determinism` with `needs: [preflight, agent-build]` and a matrix of `ubuntu-latest` and `macos-15`: install QEMU (`qemu-system-x86` on Ubuntu, `brew install qemu` on macOS), download the `dosker-agent-binaries` artifact into `agent/build`, fetch the source with the script and export the printed path to `$GITHUB_ENV`, create `"$RUNNER_TEMP/freedos-context/Doskerfile"` containing `FROM freedos:1.3`, and run two CLI builds with `cargo run -q -p dosker-cli -- build -t check:latest "$RUNNER_TEMP/freedos-context"` under `DOSKER_ROOT="$RUNNER_TEMP/root-a"` and `"$RUNNER_TEMP/root-b"` with `DOSKER_AGENT_EXE=agent/build/DOSKER.EXE`. Capture each `built check:latest sha256:` digest into `"$RUNNER_TEMP/freedos-base-digest-a.txt"` and `"$RUNNER_TEMP/freedos-base-digest-b.txt"`, require them to match, copy the value to `"$RUNNER_TEMP/freedos-base-digest.txt"`, and upload that file as `freedos-base-digest-${{ matrix.os }}`.
        - Add `compare-freedos-base` with `needs: freedos-base-determinism`: download the `freedos-base-digest-ubuntu-latest` and `freedos-base-digest-macos-15` artifacts and fail when they differ.
        - Give `freedos-base-determinism` explicit `actions/checkout@v4` and `dtolnay/rust-toolchain@1.98.1` steps, set `DOSKER_FREEDOS_SOURCE: ${{ vars.DOSKER_FREEDOS_SOURCE }}` on the fetch step, and upload each host's digest as `freedos-base-digest-${{ matrix.os }}`.
        - In `windows-qemu`, add `agent-build` to `needs` and a `actions/download-artifact@v4` step for `dosker-agent-binaries` into `agent/build` before the suite, so `command_with_qemu` can set `DOSKER_AGENT_EXE`.
        - In `qemu-nightly.yml`, add an `agent-build` job on `ubuntu-latest` with the same OpenWatcom steps as `ci.yml`, upload `dosker-agent-binaries`, make `runtime-macos` depend on `[preflight, agent-build]`, and download the artifact into `agent/build` before the runtime suite.
        - The orchestrator sets roadmap Slice 7 to `In Progress` when execution starts and to `Implemented` after the final review. Keep the plan link current.
    - **Verify:** `ruby -ryaml -e 'YAML.load_file(".github/workflows/ci.yml"); YAML.load_file(".github/workflows/qemu-nightly.yml")' && rg -q 'sha256:f34a7483c575fcf2709d9a7d0bc3db81c6211c279530f9e1bf78576b9233924d' docs/base-images.md && rg -q '75a4e11a7fce6f124e20927b3022b4b715a2a3f7324c5f5bfea42d90d80eb072' docs/base-images.md && rg -q 'org.dosker.source.digest' docs/images.md && git diff --check`

## Final Verification

Run the full workspace checks after Task 8:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --features dosker-cli/mock-engine,dosker-cli/qemu-engine -- -D warnings`
- `cargo test --workspace --features dosker-cli/mock-engine`
- `cargo test -p dosker-engine-qemu`
- `cargo test -p dosker-test-support --test transport_contract`
- `cargo test -p dosker-cli --features mock-engine --test detached_lifecycle`
- `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img DOSKER_QEMU=qemu-system-i386 cargo test -p dosker-cli --features qemu-engine --test qemu_runtime`
- `DOSKER_INTEGRATION=1 DOSKER_QEMU=qemu-system-i386 DOSKER_FREEDOS_SOURCE="$DOSKER_FREEDOS_SOURCE" DOSKER_FREEDOS_BASE_IMAGE=agent/fixtures/freedos-base.img cargo test -p dosker-cli --features qemu-engine --test qemu_freedos -- --nocapture`

## Deviations

- The FreeDOS base is built from the pinned kernel, command interpreter, agent, MBR template, and boot-sector template with fixed FAT timestamps and layout, rather than invoking `SYS.COM` under QEMU.
- `FROM freedos:1.3` reuses a valid stored base without source inputs; a stale or invalid base rebuilds and requires the source and agent.
- Imported DOS images receive a canonical tree injection layer and a narrow raw-disk injection so they boot before the general tree-to-FAT projection.
- Local imported images use a combined source-plus-agent tag and re-import pre-slice or annotation-less images.
- Agent detection matches `DOSKER.EXE` as a command token with optional arguments and `CALL`.
- Read-only source images are copied to a writable scratch file before injection.
