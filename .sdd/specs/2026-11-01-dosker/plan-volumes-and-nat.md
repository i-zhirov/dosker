# Volumes and NAT Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-18
- **Status**: Approved
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Roadmap**: `.sdd/specs/2026-11-01-dosker/roadmap.md` — slice: volumes-and-nat
- **Depends on**: Slices 3 and 5.
- **Manual prerequisites**: QEMU. A DOS packet driver and a TCP/IP stack for the integration test.
- **Goal**: Named FAT volume images that mount exclusively into one running container, and QEMU user-mode NAT with host port publishing through a NE2000 guest NIC.

## Constraints

- A volume is a raw disk image at `volumes/<name>/volume.img`, with `volumes/<name>/meta.json` and an exclusive lock at `volumes/<name>/.lock`; `meta.json` has `schema_version` `1`.
- The default volume size is `67108864` bytes (64 MiB). `--size` accepts an optional `K`, `M`, or `G` suffix in base 1024.
- A volume size must be a multiple of `512` bytes.
- Floppy sizes `1474560` (1.44 MB) and `2949120` (2.88 MB) use the `superfloppy` layout.
- A `superfloppy` volume has no MBR. 1.44 MB is FAT12 with `2` heads and `18` sectors per track; 2.88 MB is FAT16 with `2` heads and `36` sectors per track. Both use media `0xF0`, `224` root entries, `2` FATs, `512`-byte clusters, drive number `0x00`, and hidden sectors `0`.
- Every other volume size is `16777216`..`1073741824` bytes and uses `mbr_partitioned` FAT16: `512`-byte sectors, `16` heads, `32` sectors per track, partition start LBA `2048`, media `0xF8`, `2` FATs, `512` root entries, drive number `0x80`, MBR partition type `0x06`, boot flag `0x00`, and BPB hidden sectors `2048`.
- The volume MBR partition entry encodes start and end CHS for 16 heads and 32 sectors per track: `head = (lba / 32) % 16`, `sector = (lba % 32) + 1`, `cylinder = lba / (16 * 32)`, with the high cylinder bits in the sector byte. LBA `2048` is head `0`, sector `1`, cylinder `4`.
- CHS covers LBAs `0..=524287` (1024 cylinders, 16 heads, 32 sectors). An end LBA above `524287` is clamped to the maximum triple: head `15`, sector `32`, cylinder `1023`, encoded `[0x0F, 0xE0, 0xFF]`; a 1 GiB volume's end CHS is therefore capped, while its start CHS and LBA fields stay exact.
- A partitioned volume's cluster size is `(size / 32768)` rounded up to a power of two and clamped to `512..=32768` bytes.
- `fatfs` writes BPB hidden sectors as `0`; after formatting, the partition boot sector at `2048 * 512 + 28` is patched to `2048u32` little-endian.
- The FAT label is `DOSKER`, padded to eleven bytes. The volume id is the first four bytes of `sha256(name)` read little-endian and is exposed as `VolumeImage::volume_id`.
- Volume names match `[a-z0-9][a-z0-9_.-]{0,62}` and are validated at the CLI before the store call and again before every path join; an invalid name is a usage error (exit `2`).
- The `volumes/` parent directory is created with mode `0o700` before `volumes/<name>`.
- Only write paths create `.lock`; read paths open it without creating and treat a missing file as unattached.
- `VolumeLock` owns its file handle (`pub struct VolumeLock { file: File, .. }`), so dropping it unlocks and closes.
- Mountable volume drives are `A:`, `B:`, `D:`, `E:`, and `F:`. `C:` is the IDE boot disk and remains available to the guest as its boot drive; it is never a volume target. A floppy boot image occupies `A:` instead, so this plan does not assume `C:` exists in that case.
- `A:` and `B:` require a `superfloppy` volume and an MBR-partitioned (IDE) boot image; `D:`..`F:` require an `mbr_partitioned` volume.
- A partitioned volume at drive letter `L` is attached as IDE index `L - 'C'`; a superfloppy at `A:` is floppy index `0`, at `B:` floppy index `1`.
- A volume attaches to at most one running container. The lock file holds the container id, the container name, and the supervisor PID.
- A container start warns on stderr when an attached volume's `meta.json` has `dirty = true`.
- An anonymous volume is named `anon-<container-id>-<lowercase drive letter>`; `A:`/`B:` anonymous volumes are `1474560` bytes, the rest are `67108864` bytes.
- A volume created for a container is not removed when a later `start` fails; the container record still references it and `dosker volume rm` is the only remover.
- Volumes survive `dosker rm`; only `dosker volume rm` removes one, and it refuses while the volume is attached.
- Published ports bind to `127.0.0.1` by default; the protocol is `tcp`; the CLI form is `-p [addr:]host:guest` with ports `1..=65535`.
- The canonical volume form is `-v name:D:`; `-v name:D` and a literal `:ro` suffix are also accepted.
- `(host_address, host_port)` must be unique across starting and running containers. The CLI check is best-effort across processes; the engine's NAT bind is the final guard.
- QEMU networking is `-nic user,model=ne2k_isa` plus `,hostfwd=tcp:<addr>:<host>-:<guest>` per mapping in ascending host-port order. `MACHINE net=none` gives `-nic none`.
- A read-only volume passes `readonly=on` to the QEMU `-drive` option.
- NAT is declared only by the QEMU adapter; publishing ports on an engine without it is a usage error, and an empty port list never checks NAT.
- `dosker port` prints `{addr}:{host}->{guest}/tcp` per mapping and nothing when the container publishes none.
- `dosker volume ls` prints the header `NAME SIZE LAYOUT ATTACHED`; the size is in bytes; an unattached volume shows `-`.
- This plan updates `data-model.md` §Volume to allow `fat12` for the 1.44 MB superfloppy (Task 14), resolving the "`fat16` in the MVP" note.
- Out of scope: NAT on engines other than QEMU, a shared L2 network, `--bind` host directories, and wiring the image network into build `RUN` VMs, which keep `-nic none`.

## Interfaces

These signatures are shared across tasks; later tasks consume exactly what
earlier tasks define.

```rust
// dosker-image
pub enum VolumeLayout { MbrPartitioned, Superfloppy }   // serde snake_case
impl VolumeLayout { pub fn as_str(self) -> &'static str }
pub struct VolumeImage {
    pub layout: VolumeLayout,
    pub fat_type: FatType,
    pub disk_size: u64,
    pub geometry: Geometry,
    pub partition_start: Option<u32>,
    pub partition_sectors: Option<u32>,
    pub volume_id: u32,
}
pub fn create_volume(disk: &Path, size: u64, name: &str) -> Result<VolumeImage, ImageError>;
pub const DEFAULT_VOLUME_SIZE: u64;
pub const FLOPPY_1440_SIZE: u64;
pub const FLOPPY_2880_SIZE: u64;
pub const MIN_PARTITIONED_VOLUME_SIZE: u64;
pub const MAX_PARTITIONED_VOLUME_SIZE: u64;
// `Image` gains: net: String, volumes: Vec<VolumeDeclaration>

// dosker-runtime
pub struct VolumeMeta {
    pub schema_version: u32,
    pub name: String,
    pub size: u64,
    pub layout: VolumeLayout,
    pub fat_type: FatType,
    pub created_at: DateTime<Utc>,
    pub dirty: bool,
}
pub struct VolumeHolder {
    pub container_id: ContainerId,
    pub container_name: String,
    pub supervisor_pid: u32,
}
pub struct Volume { pub meta: VolumeMeta, pub path: PathBuf, pub attached: Option<VolumeHolder> }
pub enum VolumeError { NotFound(String), AlreadyExists(String), InvalidName(String),
    InUse { name: String, holder: VolumeHolder }, InvalidMeta { path: PathBuf, message: String },
    Io { path: PathBuf, source: io::Error }, Image(ImageError), Platform(PlatformError) }
impl VolumeStore {
    pub fn new(root: PathBuf) -> Self;
    pub fn create(&self, name: &str, size: u64, now: DateTime<Utc>) -> Result<Volume, VolumeError>;
    pub fn load(&self, name: &str) -> Result<Volume, VolumeError>;
    pub fn list(&self) -> Result<Vec<Volume>, VolumeError>;
    pub fn remove(&self, name: &str) -> Result<(), VolumeError>;
    pub fn acquire(&self, name: &str, holder: VolumeHolder) -> Result<VolumeLock, VolumeError>;
    pub fn set_dirty(&self, name: &str, dirty: bool) -> Result<(), VolumeError>;
}
pub struct VolumeLock { file: File, holder: VolumeHolder }   // Drop unlocks; the file closes with it
impl RuntimePaths {
    pub fn volumes_dir(&self) -> PathBuf;
    pub fn volume_dir(&self, name: &str) -> PathBuf;
    pub fn volume_image(&self, name: &str) -> PathBuf;
    pub fn volume_meta(&self, name: &str) -> PathBuf;
    pub fn volume_lock(&self, name: &str) -> PathBuf;
}
pub enum VolumeKind { Named, Bind }                     // serde snake_case
pub struct VolumeMount {
    pub drive: String, pub kind: VolumeKind, pub name: Option<String>,
    pub host_path: Option<PathBuf>, pub read_only: bool,
}
pub struct PortMapping {
    pub host_address: Ipv4Addr, pub host_port: u16, pub guest_port: u16,
    pub protocol: dosker_image::PortProtocol,
}
impl PortMapping { pub fn endpoint(&self) -> String; }
impl ContainerRecord {
    pub fn with_network(self, net: String, ports: Vec<PortMapping>) -> Self;
    pub fn with_volumes(self, volumes: Vec<VolumeMount>) -> Self;
}
pub enum PortConflict { Taken { host_address: Ipv4Addr, host_port: u16, holder: String } }
pub fn find_conflict(store: &StateStore, id: &ContainerId, ports: &[PortMapping])
    -> Result<Option<PortConflict>, StoreError>;

// dosker-engine
pub struct EnginePort { pub host_address: Ipv4Addr, pub host_port: u16, pub guest_port: u16 }
pub struct EngineVolume { pub drive: String, pub path: PathBuf, pub read_only: bool }
// `EngineSpec` gains: net: String, ports: Vec<EnginePort>, volumes: Vec<EngineVolume>
// dosker-runtime: `supervisor.rs` keeps a private fn engine_ports(&[PortMapping]) -> Vec<EnginePort>

// dosker-test-support
pub fn net_image() -> Result<PathBuf, String>;          // DOSKER_QEMU_NET_IMAGE, workspace-relative

// dosker-cli
pub struct PublishSpec { pub host_address: Ipv4Addr, pub host_port: u16, pub guest_port: u16 }
impl PublishSpec { pub fn parse(value: &str) -> Result<Self, SpecError>; }
pub struct VolumeSpec { pub name: String, pub drive: String, pub read_only: bool }
impl VolumeSpec { pub fn parse(value: &str) -> Result<Self, SpecError>; }
pub struct CreateOptions {
    pub image: String, pub name: Option<String>, pub engine: String,
    pub display: DisplayMode, pub vnc_port: Option<u16>,
    pub publish: Vec<String>, pub volumes: Vec<String>,
}
pub fn create_container(context: &mut Context, options: CreateOptions, now: DateTime<Utc>)
    -> Result<ContainerRecord, CliError>;
pub fn declared_capabilities(engine: &str) -> Option<CapabilitySet>;
pub fn validate_network(capabilities: CapabilitySet, net: &str, ports: &[PortMapping])
    -> Result<(), String>;
fn resolve_volumes(context: &Context, image: &Image, specs: &[VolumeSpec], id: &ContainerId,
    now: DateTime<Utc>) -> Result<(Vec<VolumeMount>, Vec<String>), CliError>;
pub enum CliError { /* existing variants */, Volume(VolumeError) }   // From<VolumeError>
```

## Tasks

- [ ] **Task 1: Volume disk images**
    - **Implements:** FR-014; design §Runtime (`A named volume is a partitioned FAT16 disk image…`), data-model §Volume, validation rule "A volume uses `mbr_partitioned` for hard-disk sizes and `superfloppy` for floppy sizes".
    - **Create `crates/dosker-image/tests/volume.rs` first**
        - `the_default_size_creates_a_partitioned_fat16_volume` — `create_volume` at `DEFAULT_VOLUME_SIZE` reports `MbrPartitioned`, `Fat16`, partition start `2048`, a disk size equal to the file size, and `volume_id` equal to the first four bytes of the name digest.
        - `a_partitioned_volume_has_no_files` — `read_tree` returns an empty tree.
        - `a_floppy_size_creates_a_superfloppy_volume` — `1_474_560` and `2_949_120` report `Superfloppy` with no partition bounds.
        - `the_1440_floppy_is_fat12_and_the_2880_floppy_is_fat16`.
        - `the_partitioned_bpb_has_the_pinned_fields` — read the sector at `2048 * 512` and assert byte `0` is `0xEB`, `le_u16(11) == 512`, `sector[13] == cluster_bytes / 512`, `le_u16(14) == 1`, `sector[16] == 2`, `le_u16(17) == 512`, `le_u32(28) == 2048`, `sector[21] == 0xF8`, `le_u16(24) == 32`, `le_u16(26) == 16`, `sector[36] == 0x80`, `sector[38] == 0x29`, `le_u32(39) == volume_id`, `sector[43..54] == b"DOSKER     "`, `sector[54..62] == b"FAT16   "`, and `sector[510..512] == [0x55, 0xAA]`.
        - `the_mbr_describes_one_bootable_free_partition` — at the default size, `entry[0] == 0x00`, `entry[4] == 0x06`, `le_u32(entry + 8) == 2048`, `le_u32(entry + 12) == (size / 512) - 2048`, signature `0x55AA`, the start CHS bytes at `entry + 1..4` decode to LBA `2048` (head `0`, sector `1`, cylinder `4`), and the end CHS bytes at `entry + 5..8` decode to `2048 + (size / 512) - 2049`.
        - `the_mbr_chs_round_trips_within_the_chs_range` — a shared test helper decodes a CHS triple back to an LBA; assert it round-trips the start and end LBAs for `16 MiB` and `64 MiB`, whose end LBAs are at most `524287`.
        - `the_mbr_chs_caps_an_end_lba_above_the_chs_range` — for `1 GiB` the start CHS still decodes to `2048`, the end CHS bytes are exactly `[0x0F, 0xE0, 0xFF]` (head `15`, sector `32`, cylinder `1023`), and the LBA fields stay `2048` and `(size / 512) - 2048`.
        - `the_superfloppy_bpb_has_the_pinned_fields` — same BPB assertions at offset `0` with the per-layout values, `le_u32(28) == 0`, and `sector[54..62]` of `FAT12   ` or `FAT16   `.
        - `partitioned_cluster_sizes_stay_within_fat16` — `16 MiB`, `64 MiB`, and `300 MiB` each report FAT16 and mount.
        - `a_size_that_is_not_a_multiple_of_512_is_rejected` — `1_474_561`.
        - `a_size_below_the_partitioned_minimum_is_rejected` — `1_048_576`.
        - `a_size_above_the_maximum_is_rejected` — `2_147_483_648`.
        - `the_volume_id_depends_on_the_name` — two names differ; one name reproduces its id.
    - **Create `crates/dosker-image/src/volume.rs`**
        - Constants `DEFAULT_VOLUME_SIZE = 67_108_864`, `FLOPPY_1440_SIZE = 1_474_560`, `FLOPPY_2880_SIZE = 2_949_120`, `MIN_PARTITIONED_VOLUME_SIZE = 16_777_216`, `MAX_PARTITIONED_VOLUME_SIZE = 1_073_741_824`, `PARTITION_START = 2048`, `PARTITION_TYPE_FAT16 = 0x06`, `VOLUME_LABEL_TEXT = "DOSKER"`.
        - `VolumeLayout` — `MbrPartitioned`/`Superfloppy`, serialized `mbr_partitioned`/`superfloppy`, with `as_str`.
        - `VolumeImage` — layout, FAT type, disk size, geometry, partition bounds, and `volume_id`.
        - `create_volume(disk, size, name)` — validate the size (a multiple of `512`; a floppy size; otherwise `16 MiB..=1 GiB`), create a file of exactly `size` bytes, write the MBR for a partitioned volume, format with `fatfs::format_volume`, patch BPB hidden sectors for a partitioned volume, then read the result back with `crate::fat::read_boot_metadata`.
        - `format_options(layout, size, name)` — exact `FormatVolumeOptions`:
            - partitioned: `Fat16`, `bytes_per_sector(512)`, `total_sectors(partition_sectors)`, `bytes_per_cluster(cluster_bytes(size))`, `fats(2)`, `max_root_dir_entries(512)`, `media(0xF8)`, `sectors_per_track(32)`, `heads(16)`, `drive_num(0x80)`, `volume_id`, `volume_label(*b"DOSKER     ")`.
            - 1.44 MB: `Fat12`, `512`, `total_sectors(2880)`, `bytes_per_cluster(512)`, `fats(2)`, `max_root_dir_entries(224)`, `media(0xF0)`, `sectors_per_track(18)`, `heads(2)`, `drive_num(0x00)`, `volume_id`, label.
            - 2.88 MB: `Fat16`, `512`, `total_sectors(5760)`, `bytes_per_cluster(512)`, `fats(2)`, `max_root_dir_entries(224)`, `media(0xF0)`, `sectors_per_track(36)`, `heads(2)`, `drive_num(0x00)`, `volume_id`, label.
        - `patch_hidden_sectors(disk)` — after formatting a partitioned volume, write `2048u32` little-endian at offset `PARTITION_START * 512 + 28` and flush.
        - `chs(lba) -> [u8; 3]` — clamp `lba` to `524287` first, then `head = (lba / 32) % 16`, `sector = (lba % 32) + 1`, `cylinder = lba / (16 * 32)`; returns `[head, sector | ((cylinder >> 2) & 0xC0), cylinder & 0xFF]`. `chs(524288)` and above return `[0x0F, 0xE0, 0xFF]`. The MBR entry writes `chs(2048)` at `entry + 1` and `chs(end_lba)` at `entry + 5`.
        - `write_mbr(disk, size)` — one partition entry: boot flag `0x00`, start CHS `chs(2048)`, type `0x06`, end CHS `chs(2048 + sectors - 1)`, start LBA `2048`, sector count, signature `0x55AA`; the remaining code bytes are zero.
        - `cluster_bytes(size) = (size / 32768).next_power_of_two().clamp(512, 32768)`.
        - `volume_id(name)` — first four bytes of `sha256(name)` read little-endian.
    - **Modify `crates/dosker-image/src/error.rs`**
        - `ImageError::InvalidVolumeSize { size: u64, reason: String }` — not a multiple of 512, neither a floppy size nor in the partitioned range, or not representable as FAT16.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - `pub mod volume;` and re-export `create_volume`, `VolumeImage`, `VolumeLayout`, and the five size constants.
    - **Verify:** `cargo test -p dosker-image --test volume`

- [ ] **Task 2: Volume store, metadata, and exclusive lock**
    - **Implements:** FR-014; design §State Directory, data-model §Volume ("The lock is `volumes/<name>/.lock` and contains the container ID and supervisor PID"), error handling "Volume already attached".
    - **Create `crates/dosker-runtime/tests/volume.rs` first**
        - `create_writes_an_image_and_a_meta_file` — starting from a fresh root, both files exist, the image size is the requested size, and `load` reports the same metadata; the `volumes/` directory mode `0o700` assertion is gated `#[cfg(unix)]` because `ensure_dir_mode` applies modes only on Unix.
        - `create_rejects_a_duplicate_name_without_touching_the_volume` — record the image bytes and meta file contents, call `create` again, assert `AlreadyExists`, and assert both files are byte-identical.
        - `load_reports_a_missing_volume` — `NotFound`.
        - `list_of_a_fresh_root_is_empty` — no `volumes/` directory yet.
        - `list_returns_volumes_sorted_by_name`.
        - `a_second_attach_fails_and_names_the_holder` — after `acquire` for container `other`, a second `acquire` returns `InUse` whose message contains `other`.
        - `dropping_the_lock_releases_the_volume` — the second `acquire` succeeds and `load` reports no holder.
        - `remove_rejects_a_volume_that_is_attached` — `InUse`; the volume is still on disk.
        - `remove_deletes_the_volume_directory` — the image, meta file, and lock file are gone (also exercises the Windows close-before-delete order).
        - `a_stale_holder_without_a_lock_is_not_reported`.
        - `a_meta_file_with_another_schema_version_is_rejected` — `InvalidMeta`.
        - `set_dirty_updates_the_meta_file`.
        - `load_remove_acquire_and_set_dirty_reject_path_like_names` — each of `../x`, `/tmp/x`, and `a/b` returns `InvalidName` and no path outside the data root is touched.
    - **Modify `crates/dosker-runtime/Cargo.toml`**
        - Add `fs2.workspace = true`.
    - **Modify `crates/dosker-runtime/src/paths.rs`**
        - `volumes_dir`, `volume_dir(name)`, `volume_image(name)` (`volume.img`), `volume_meta(name)` (`meta.json`), `volume_lock(name)` (`.lock`), as pinned in Interfaces.
    - **Create `crates/dosker-runtime/src/volume.rs`**
        - `VolumeMeta`, `VolumeHolder`, `Volume`, `VolumeLock`, `VolumeError`, `VolumeStore` as pinned in Interfaces.
        - `check_name(name)` — `ContainerName::parse`, mapped to `VolumeError::InvalidName`; every method that builds a path calls it first.
        - `create(name, size, now)` — validate the name; `ensure_dir_mode(volumes_dir, 0o700)` so a fresh root works; `fs::create_dir(volumes/<name>)` and remember whether this call created it; `AlreadyExists` returns `AlreadyExists` without touching the existing directory; run `create_volume`, write `meta.json` pretty-printed plus a newline through `write_atomic`; on any post-creation failure remove the directory only when this call created it.
        - `load(name)` — read and validate `meta.json` (`schema_version` must be `1`); report `attached` by opening `.lock` read-write **without creating it**: a missing file reports no holder, a failed lock reads the holder JSON, and a successful lock unlocks.
        - `list()` — sorted by name; a missing `volumes/` lists as empty; invalid directory names are skipped; a volume that disappears mid-scan is ignored.
        - `remove(name)` — validate the name; open `.lock` read-write without creating it (a missing file skips the lock); try the exclusive lock; a contended lock returns `InUse` with the recorded holder; on success **drop the file handle first** (Windows cannot delete an open file) and then `remove_dir_all`.
        - `acquire(name, holder)` — validate the name, open/create `.lock`, take the exclusive `fs2` lock, write the holder JSON plus a newline, return `VolumeLock`; a contended lock returns `InUse`. `VolumeLock::drop` truncates the file and unlocks.
        - `set_dirty(name, dirty)` — validate the name, then rewrite `meta.json`.
        - `VolumeError` maps `dosker_platform::PlatformError` through a `From` impl (`Platform` variant), and `ImageError` through `Image`. `InvalidName` displays `invalid volume name: {name}`.
    - **Modify `crates/dosker-runtime/src/lib.rs`**
        - `pub mod volume;` and re-export `VolumeStore`, `VolumeError`, `VolumeLock`, `VolumeMeta`, `VolumeHolder`, `Volume`.
    - **Verify:** `cargo test -p dosker-runtime --test volume`

- [ ] **Task 3: Container records and image records carry network and volume data**
    - **Implements:** FR-014, FR-015; design §Data Model, data-model §Container, §PortMapping, §VolumeMount.
    - **Modify `crates/dosker-image/tests/oci.rs` first**
        - `image_reports_the_network_device_and_volume_declarations` — commit `support::sample_config` with `machine.net = "ne2000"` and one `VolumeDeclaration { drive: "D:" }`, passing the existing tree layer `Descriptor` (the `MEDIA_TYPE_LAYER` blob the other commit tests use), then `load` and assert both fields.
    - **Modify `crates/dosker-runtime/tests/record.rs` first**
        - `a_new_record_has_no_ports_or_volumes_and_uses_the_ne2000_network`.
        - `with_network_records_the_ports_and_the_network_device`.
        - `with_volumes_records_the_mounts`.
        - `an_existing_state_file_loads_with_empty_ports_and_volumes`.
        - `a_port_mapping_renders_its_endpoint` — `127.0.0.1:8080` to guest `80` renders `127.0.0.1:8080->80/tcp`.
    - **Modify `crates/dosker-image/src/oci.rs`**
        - `Image` gains `pub net: String` and `pub volumes: Vec<VolumeDeclaration>`; `load` fills them from `config.machine.net` and `config.volumes`.
    - **Modify `crates/dosker-runtime/src/record.rs`**
        - `PortMapping`, `VolumeMount`, `VolumeKind` as pinned; `PortMapping::endpoint()`.
        - `ContainerRecord` gains `#[serde(default)] ports`, `#[serde(default)] volumes`, `#[serde(default = "default_net")] net: String` (`"ne2000"`).
        - `with_network` and `with_volumes`; `new` initializes the vectors and the network.
    - **Modify `crates/dosker-runtime/src/lib.rs`**
        - Re-export `PortMapping`, `VolumeMount`, `VolumeKind`.
    - **Verify:** `cargo test -p dosker-image --test oci && cargo test -p dosker-runtime --test record`

- [ ] **Task 4: Declare NAT on the QEMU adapter**
    - **Implements:** FR-015; design §Engine Capability Matrix (`nat`: QEMU yes).
    - **Modify `crates/dosker-engine-qemu/tests/command.rs` first**
        - `capabilities_are_the_proven_set` — add `Capability::Nat` to the expected set.
        - Remove `capabilities_never_include_nat` and replace it with `capabilities_include_nat`, which asserts `adapter.capabilities().contains(Capability::Nat)`.
    - **Modify `crates/dosker-cli/tests/cli_engines.rs` first**
        - `CAPABILITIES` becomes `boot,serial,stop,headless,debug_native,gdbstub,screenshot,vnc,nat`; `engines_lists_the_proven_capabilities` asserts `nat` is present instead of absent.
    - **Modify `crates/dosker-engine-qemu/src/adapter.rs`**
        - Add `Capability::Nat` to `capabilities()`.
    - **Note:** `command_disables_networking` in `crates/dosker-engine-qemu/tests/command.rs` still holds here because the command line is unchanged until Task 7; Task 7 replaces it with the NIC assertions.
    - **Verify:** `cargo test -p dosker-engine-qemu --test command && cargo test -p dosker-cli --features qemu-engine --test cli_engines`

- [ ] **Task 5: Parse `-p` and `-v` values**
    - **Implements:** FR-014, FR-015; design §CLI (`run … [-p [addr:]host:guest] [-v name:drive[:ro]]`), data-model §PortMapping, §VolumeMount.
    - **Create `crates/dosker-cli/src/spec.rs` with tests first**
        - `publish_defaults_to_loopback` — `8080:80` → `127.0.0.1`, `8080`, `80`.
        - `publish_accepts_an_explicit_address` — `0.0.0.0:8080:80`.
        - `publish_rejects_a_lone_port` — `80`.
        - `publish_rejects_port_zero` — `0:80` and `8080:0`.
        - `publish_rejects_a_non_ipv4_address` — `not-an-address:8080:80`.
        - `volume_spec_accepts_the_canonical_form` — `data:D:` → name `data`, drive `D:`.
        - `volume_spec_accepts_a_drive_without_a_colon` — `data:D` → `D:`.
        - `volume_spec_normalizes_a_lowercase_drive` — `data:d:` → `D:`.
        - `volume_spec_parses_read_only` — `data:D:ro` and `data:d:ro` → `read_only` true.
        - `volume_spec_rejects_the_boot_drive` — `data:C:`.
        - `volume_spec_rejects_an_unknown_option` — `data:D:rw`.
        - `volume_spec_rejects_a_missing_drive` — `data`.
        - `volume_spec_rejects_a_path_like_name` — `../x:D:`, `/tmp/x:D:`, `a/b:D:`.
    - **Implementation in the same file**
        - `PublishSpec::parse` — split on `:`; two parts are `host:guest`, three are `addr:host:guest`; default `127.0.0.1`; reject other arity, port `0`, non-numeric ports, and non-IPv4 addresses.
        - `VolumeSpec::parse` — strip an optional literal `:ro` suffix to set `read_only`; strip one optional trailing `:`; `rsplit_once(':')` into name and drive; uppercase the drive and append `:`; require `A`..`Z` and not `C`; validate the name with `ContainerName::parse`. This accepts `data:D:`, `data:D`, and their `:ro` forms, and rejects `data` with no separator and any name outside the volume-name rule.
        - `SpecError` — `Publish { value, reason }` and `Volume { value, reason }`, displaying `invalid publish mapping …` / `invalid volume mapping …`.
    - **Modify `crates/dosker-cli/src/lib.rs`**
        - `pub mod spec;`.
    - **Verify:** `cargo test -p dosker-cli --lib spec`

- [ ] **Task 6: `create` mounts volumes and records published ports**
    - **Implements:** FR-014, FR-015; design §Runtime (`-v name:D:` mounts read-write, `:ro` read-only; `-p host:guest`; `VOLUME` creates the anonymous volume at create time), contract §VOLUME.
    - **Create `crates/dosker-cli/tests/cli_volumes.rs` first** (each test imports the base image through `CliRoot::import_image_as("freedos:latest")` before `create`/`run`)
        - `create_mounts_a_named_volume_at_the_requested_drive` — seed `data` with `VolumeStore::create` on the test root (the `volume` commands arrive in Task 10); `create -v data:D: freedos:latest`; `inspect` shows one named mount at `D:` with `read_only` false.
        - `create_accepts_the_canonical_volume_form` — `-v data:D:` and `-v data:D` both record the mount.
        - `create_mounts_a_superfloppy_volume_at_a_floppy_drive` — seed a `1_474_560`-byte volume named `flop`, `create -v flop:A:`, and `inspect` records `A:`.
        - `create_mounts_partitioned_volumes_on_the_ide_drives` — seed `dvol`, `evol`, and `fvol`, mount them at `D:`, `E:`, and `F:` in one `create`, and `inspect` records all three.
        - `create_rejects_an_unknown_volume` — exit `1`, stderr contains `volume not found`.
        - `create_rejects_a_path_like_volume_name` — `-v ../x:D:`, exit `2`, and no `volumes/../x` exists.
        - `create_rejects_a_partitioned_volume_at_a_floppy_drive` — seed a 64 MiB volume named `data`, mount it at `A:`, exit `2`.
        - `create_rejects_a_superfloppy_volume_at_an_ide_drive` — seed a 1.44 MB volume, mount it at `D:`, exit `2`.
        - `create_rejects_a_duplicate_drive` — seed `data`, use two `-v` values on `D:`, exit `2`.
        - `create_creates_an_anonymous_volume_for_an_image_declaration` — `#[cfg(unix)]` (the fake QEMU engine is a shell script, as in `cli_build.rs`); build `FROM freedos:latest` plus `VOLUME D:` and `VOLUME E:`, then `create`; `inspect` shows both anonymous mounts and `<root>/volumes/anon-<id>-d/meta.json` and `…-e/meta.json` exist.
        - `create_removes_anonymous_volumes_when_the_container_is_rejected` — `#[cfg(unix)]`; seed a container named `taken`, build the image with `VOLUME D:`, `create --name taken demo` exits `1`, and no `anon-` volume remains.
        - `create_records_the_published_ports` — with a host port from `free_tcp_port()`, `-p <port>:80` records `127.0.0.1:<port>` to guest `80` in the `inspect` JSON.
        - `create_rejects_a_duplicate_host_port` — two `-p` values on the same port, exit `2`.
    - **Tests inside `crates/dosker-cli/src/commands/create.rs` first**
        - `no_ports_do_not_require_the_nat_capability` — `validate_network(CapabilitySet::EMPTY, "ne2000", &[])` is `Ok`.
        - `a_published_port_requires_the_nat_capability` — `validate_network(CapabilitySet::EMPTY, "ne2000", &[mapping])` names `nat`.
        - `publish_is_rejected_when_the_image_disables_the_network` — `none` with one mapping.
        - `publish_is_accepted_for_the_ne2000_network`.
    - **Modify `crates/dosker-cli/src/error.rs`**
        - `CliError::Volume(#[from] VolumeError)`; `exit_code` maps `Volume(VolumeError::InvalidName(_))` and `Volume(VolumeError::Image(ImageError::InvalidVolumeSize { .. }))` to `EXIT_USAGE`, and every other `Volume` to `EXIT_ERROR`.
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - `RunArgs` and `CreateArgs` gain `-p/--publish` and `-v/--volume` as `Vec<String>` with the value names from the design.
    - **Modify `crates/dosker-cli/src/adapter.rs`**
        - `declared_capabilities(engine) -> Option<CapabilitySet>` — the QEMU set, or the mock set under the feature, without discovering a binary; `None` for an unknown engine.
    - **Modify `crates/dosker-cli/src/context.rs`**
        - `Context` gains `pub volumes: VolumeStore`, built from the same root in `with_console`.
    - **Modify `crates/dosker-cli/src/commands/create.rs`**
        - `CreateOptions` replaces the positional arguments; `execute` builds it from the flags.
        - Order: validate the engine, load the image, parse the values, resolve mounts, resolve ports, build the record, `store.create`, copy the disk, save.
        - `resolve_volumes(context, image, specs, id, now) -> Result<(Vec<VolumeMount>, Vec<String>), CliError>` — named mounts must exist (`VolumeError::NotFound` maps to `CliError::Message("volume not found: {name}")`); duplicate drives are rejected; `A:`/`B:` need a `superfloppy` and an IDE boot image, `D:`..`F:` need `mbr_partitioned`; `C:` and `G:`..`Z:` are rejected naming the supported set; each image `VOLUME` drive not claimed by `-v` gets an anonymous volume created when absent; returns the mounts and the anonymous names this call created. If a step after the first anonymous creation fails, remove the anonymous volumes created so far before returning the error.
        - `resolve_ports(image, engine, values)` — parse; reject a repeated `(host_address, host_port)`; resolve `declared_capabilities(engine).ok_or_else(|| CliError::Usage(format!("unknown engine: {engine}")))?`; then `validate_network(capabilities, &image.net, &ports)?` and map the message to `CliError::Usage`.
        - `validate_network(capabilities: CapabilitySet, net: &str, ports: &[PortMapping]) -> Result<(), String>` — an empty list is always accepted; a non-empty list requires `net == "ne2000"` and `capabilities.contains(Capability::Nat)`. Messages: `the image disables the network device; published ports need MACHINE net=ne2000` and `engine does not provide the nat capability; published ports need the QEMU engine`.
        - Validation failures are `CliError::Usage` (exit `2`); a missing named volume is `CliError::Message` (exit `1`).
        - On any failure after `resolve_volumes`, remove the anonymous volumes this call created (ignore errors), then remove the container record as today.
        - Record: `.with_network(image.net.clone(), ports).with_volumes(mounts)`.
    - **Modify `crates/dosker-cli/src/commands/run.rs`**
        - Build and pass `CreateOptions`.
    - **Modify `crates/dosker-test-support/src/mock_adapter.rs`**
        - Add `Capability::Nat` to `MockAdapter::capabilities`; `declared_capabilities("mock")` returns the same four capabilities.
    - **Modify `crates/dosker-test-support/tests/mock_adapter.rs`**
        - `capabilities_are_boot_serial_and_stop` at line 194 becomes `capabilities_are_boot_serial_stop_and_nat`: the expected set gains `Capability::Nat`, and the negative assertion changes to `!contains(Capability::Headless)`.
    - **Verify:** `cargo test -p dosker-cli --lib commands::create && cargo test -p dosker-cli --features mock-engine --test cli_volumes && cargo test -p dosker-test-support --test mock_adapter`

- [ ] **Task 7: QEMU NE2000 NIC, user-mode NAT, and volume disks**
    - **Implements:** FR-014, FR-015; design §Runtime, engine research ("`-nic user` … `hostfwd=tcp:127.0.0.1:6001-:6000`", "The ISA NE2000 model is `ne2k_isa`").
    - **Modify `crates/dosker-engine-qemu/tests/command.rs` first**
        - `command_uses_the_conformance_machine` — expect `-nic` `user,model=ne2k_isa`; keep the machine, CPU, and memory assertions.
        - Remove `command_disables_networking` and replace it with `command_uses_no_nic_when_the_machine_disables_the_network` — `net` `none` gives `-nic none`.
        - `command_publishes_a_host_port_to_a_guest_port` — the `-nic` value contains `hostfwd=tcp:127.0.0.1:8080-:80`.
        - `command_publishes_ports_in_host_port_order`.
        - `command_publishes_to_an_explicit_address`.
        - `command_rejects_an_unknown_network_device`.
        - `command_attaches_a_partitioned_volume_as_an_ide_disk` — `D:` → `index=1`, `E:` → `index=2`.
        - `command_attaches_a_superfloppy_volume_as_a_floppy` — `A:` → `if=floppy,index=0`, `B:` → `index=1`.
        - `command_marks_a_read_only_volume_read_only`.
        - `command_rejects_a_volume_drive_outside_the_supported_set` — `G:`.
    - **Modify `crates/dosker-engine/src/spec.rs`**
        - `EnginePort`, `EngineVolume`, and `EngineSpec` fields `net: String`, `ports: Vec<EnginePort>`, `volumes: Vec<EngineVolume>`.
    - **Modify `crates/dosker-engine/src/lib.rs`**
        - Re-export `EnginePort`, `EngineVolume`.
    - **Update every `EngineSpec` literal** with `net: "none".to_owned()`, empty `ports`, empty `volumes`; the exception is `sample_spec()` in `crates/dosker-engine-qemu/tests/command.rs`, which takes `net: "ne2000".to_owned()`:
        - `crates/dosker-engine/tests/adapter.rs`, `crates/dosker-engine-qemu/src/session.rs` (test module), `crates/dosker-engine-qemu/tests/qemu_session.rs`, `crates/dosker-engine-qemu/tests/command.rs`, `crates/dosker-engine-qemu/tests/endpoints.rs`, `crates/dosker-engine-qemu/tests/session_endpoints.rs`, `crates/dosker-build/src/run_in_dos/engine.rs`, `crates/dosker-test-support/tests/mock_adapter.rs`, `crates/dosker-runtime/src/supervisor.rs`.
        - `run_in_dos/engine.rs` keeps `net: "none"`: the design's "a `RUN` VM has the same network capability as the image" is not wired in this slice, and build `RUN` networking is listed as out of scope in Constraints.
    - **Modify `crates/dosker-engine-qemu/src/command.rs`**
        - Replace the fixed `-nic none` with `nic_option(spec)?`: `none` for `net == "none"`; `user,model=ne2k_isa` for `net == "ne2000"`; any other value is `EngineError::Setup`; append `,hostfwd=tcp:{host_address}:{host_port}-:{guest_port}` per mapping sorted by `(host_port, guest_port)`.
        - Append one `-drive` per volume after the boot disk, sorted by drive: `file={path},format=raw,if=ide,index={letter - 'C'}` for `D:`..`F:`, `file={path},format=raw,if=floppy,index=0|1` for `A:`/`B:`, plus `,readonly=on` when read-only; any other drive is `EngineError::Setup`.
    - **Verify:** `cargo test -p dosker-engine-qemu --test command && cargo test -p dosker-engine -p dosker-engine-qemu`

- [ ] **Task 8: The supervisor attaches volumes and passes the network spec**
    - **Implements:** FR-014, FR-015; design §Process Model, §Runtime, error handling "Volume already attached: fail the second start with the name of the holder".
    - **Modify `crates/dosker-runtime/tests/supervisor.rs` first**
        - Add `Harness::start_with_data(ports, volumes)` that creates each named volume through `VolumeStore::create`, records `with_network("ne2000", ports)` and `with_volumes(volumes)`, and extends `StubSession` with `stop_error: Option<EngineError>`.
        - Add a spec-recording adapter field (`Arc<Mutex<Option<EngineSpec>>>` set in `build_command`).
        - `supervisor_acquires_the_volume_lock_and_releases_it_on_stop`.
        - `supervisor_fails_when_a_volume_is_already_attached` — the test first calls `VolumeStore::acquire("data", VolumeHolder { container_id: <other id>, container_name: "other", supervisor_pid: <test pid> })` and keeps that lock for the test's duration; the harness then starts a container whose record mounts `data`. The record ends `failed`; `container.log` contains `other`.
        - `supervisor_passes_the_network_ports_and_volumes_to_the_engine`.
        - `supervisor_marks_a_volume_dirty_after_a_crash` — after the supervisor thread joins, `VolumeStore::load` reports `dirty`.
        - `supervisor_marks_a_volume_dirty_when_stop_fails` — `stop_error` is `EngineError::StopTimeout`, the record ends `exited` with `dirty`, and the volume meta reports `dirty` after joining.
    - **Modify `crates/dosker-runtime/src/supervisor.rs`**
        - Before `write_config`: build a `VolumeStore` with `VolumeStore::new(store.paths().root().to_path_buf())`; for each `Named` mount call `acquire` with the record id, record name, and supervisor PID, keeping the locks for the life of `supervise`; `InUse` becomes `failed_start` with `volume {name} is attached to container {container_name}`.
        - Build `Vec<EngineVolume>` from the mounts via `RuntimePaths::volume_image(name)`; a missing file is a setup failure naming the volume.
        - `EngineSpec` uses `record.net`, `engine_ports(&record.ports)`, and the resolved volumes. `fn engine_ports(ports: &[PortMapping]) -> Vec<EnginePort>` is a private helper in `supervisor.rs` (not re-exported from the crate) that copies `host_address`, `host_port`, and `guest_port` for each mapping.
        - `run_stop`: on `Ok` mark each attached volume clean; on the `Err`/kill path mark each attached volume dirty before saving `exited`.
        - `mark_exited(store, record)` builds its own `VolumeStore` from `store.paths().root()` and marks each attached volume dirty; `run_stop`'s `Err` path calls the same helper.
        - Volumes are marked through `VolumeStore::set_dirty(name, dirty)`; a failure only logs a warning and never changes the container outcome.
    - **Verify:** `cargo test -p dosker-runtime --test supervisor`

- [ ] **Task 9: Reject a port conflict before the supervisor starts**
    - **Implements:** FR-015; design §Requirements "Run several containers at once", error handling "Port conflict: fail the start and leave other containers untouched".
    - **Create `crates/dosker-runtime/tests/ports.rs` first**
        - `a_free_host_port_has_no_conflict`.
        - `a_taken_host_port_names_the_holder` — a running record named `first`.
        - `a_stopped_container_does_not_hold_a_port`.
        - `the_same_port_on_another_address_does_not_conflict`.
        - `a_container_does_not_conflict_with_itself`.
    - **Modify `crates/dosker-cli/tests/cli_run.rs` first**
        - `a_second_container_cannot_publish_a_taken_host_port` — use `free_tcp_port()` for the host port; `run -d -p <port>:80` twice; the second exits `1`, stderr contains `already published`, and `inspect` shows the first still `running`; `rm` the failed container.
        - `a_stale_container_does_not_hold_a_port` — seed a `running` record whose supervisor PID is dead, then start a container with the same port; it succeeds (the stale record is recovered first).
    - **Modify `crates/dosker-cli/tests/cli_volumes.rs` first**
        - `volume_warns_when_a_mounted_volume_is_dirty` — seed `data`, `create -v data:D:` a container, mark the volume dirty with `VolumeStore::set_dirty("data", true)` (no clean `stop` in between), then `start` the container; stderr contains `warning: volume data was not shut down cleanly`.
    - **Create `crates/dosker-runtime/src/ports.rs`**
        - `PortConflict` as pinned; message `port {host_address}:{host_port}/tcp is already published by container {holder}`.
        - `find_conflict(store, id, ports)` — compares `(host_address, host_port, protocol)` against other records with `Status::is_active()`; the holder is the record name.
    - **Modify `crates/dosker-runtime/src/lib.rs`**
        - `pub mod ports;` and re-export `find_conflict`, `PortConflict`.
    - **Modify `crates/dosker-cli/src/commands/start.rs`**
        - At the top of `start_container`, run `recover_stale(&context.store, &probe, Utc::now())` and reload the record by id.
        - After `wait_for_previous_supervisor` and before `spawn_supervisor`: when `record.ports` is non-empty, call `find_conflict`; on a conflict append the message to `container.log` and return `CliError::Message`. The record is not transitioned. This check is best-effort across concurrent processes; a lost race surfaces as an engine `NAT bind` failure.
        - For each `Named` mount, `context.volumes.load(name)` and, when `meta.dirty`, print `warning: volume {name} was not shut down cleanly` through `console.error_line`; a load error is ignored here because the supervisor reports the volume attachment failure.
        - A failed start leaves any volume created at `create` time in place, including anonymous volumes, because the container record still references them (see Constraints).
    - **Verify:** `cargo test -p dosker-runtime --test ports && cargo test -p dosker-cli --features mock-engine --test cli_run && cargo test -p dosker-cli --features mock-engine --test cli_volumes`

- [ ] **Task 10: `dosker volume create`, `ls`, and `rm`**
    - **Implements:** FR-014; design §CLI, data-model §Volume.
    - **Modify `crates/dosker-cli/tests/cli_volumes.rs` first** (every test that starts a container imports the base image through `CliRoot::import_image_as("freedos:latest")` first)
        - `volume_create_list_and_remove_round_trip` — create `data`; `ls` shows `NAME SIZE LAYOUT ATTACHED` and `data 67108864 mbr_partitioned -`; `rm` removes it.
        - `volume_create_accepts_a_floppy_size` — `--size 1440K` lists `superfloppy`.
        - `volume_create_rejects_an_unknown_size_suffix` — `--size 64X`, exit `2`.
        - `volume_create_rejects_a_size_outside_the_volume_range` — `--size 1M`, exit `2` (`ImageError::InvalidVolumeSize` nested in `VolumeError`).
        - `volume_create_rejects_a_path_like_name` — `volume create ../x`, exit `2`, and nothing is created outside the data root.
        - `volume_create_rejects_a_duplicate_name` — exit `1`, stderr contains `already exists`, and the existing image is unchanged.
        - `volume_rm_rejects_a_name_outside_the_rule` — `volume rm ../x`, exit `2`.
        - `volume_rm_rejects_an_attached_volume` — start a mock container with `-v data:D:`; `volume rm data` exits `1` and names the container; after `stop` the removal succeeds.
        - `volume_rm_waits_for_a_stopping_container` — hold the lock in a test thread, start `volume rm`, release the lock after 200 ms, and assert the command succeeds within the retry window.
        - `volume_ls_reports_the_holder_container`.
    - **Tests inside `crates/dosker-cli/src/commands/volume.rs` first**
        - `a_size_suffix_is_base_1024` — `64M` → `67_108_864`.
        - `a_bare_size_is_bytes` — `1048576` → `1_048_576` (no suffix means bytes).
        - `a_zero_size_is_rejected`.
        - `a_size_with_an_unknown_suffix_is_rejected` — `64X` and `12KB`.
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - `Command::Volume(VolumeArgs)` with `Create`, `Ls`, `Rm`; `VolumeCreateArgs { #[arg(long = "size", default_value = "64M")] size: String, name: String }`.
    - **Modify `crates/dosker-cli/src/commands/mod.rs`**
        - `pub mod volume;`.
    - **Create `crates/dosker-cli/src/commands/volume.rs`**
        - `execute(args, context, now)`.
        - `parse_size(value)` — optional single `K`/`M`/`G` suffix in base 1024; a suffixless numeric value is bytes; reject empty, zero, non-numeric, and unknown or multi-letter suffixes as `CliError::Usage`.
        - `create` validates the name with `ContainerName::parse` before calling the store (exit `2` on failure) and prints the name on success.
        - `rm` validates the name with `ContainerName::parse` (exit `2` on failure) and calls `remove_retrying(&context.volumes, &name, RM_RETRY_TIMEOUT, RM_RETRY_STEP)`, which retries `VolumeError::InUse` for `5` seconds at `50` ms and then reports the holder.
        - `ls` prints the header and rows (`name`, bytes, `layout.as_str()`, holder name or `-`).
        - `NotFound` → `volume not found: {name}` (exit `1`); `AlreadyExists` → `volume already exists: {name}` (exit `1`); `InvalidName` and a nested `InvalidVolumeSize` → exit `2`; `InUse` keeps its message (exit `1`).
    - **Verify:** `cargo test -p dosker-cli --lib commands::volume && cargo test -p dosker-cli --features mock-engine --test cli_volumes`

- [ ] **Task 11: `dosker port`**
    - **Implements:** FR-015; design §CLI, §Runtime ("`dosker port` reports the effective port").
    - **Create `crates/dosker-cli/tests/cli_port.rs` first** (create the image with `CliRoot::import_image_as` before the containers)
        - `port_prints_the_effective_mappings` — create with `-p <free>:80 -p 127.0.0.1:<free2>:81` using `free_tcp_port()`; exactly two lines in that order.
        - `port_prints_nothing_without_mappings` — exit `0`, empty stdout.
        - `port_reports_an_unknown_container` — exit `1`.
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - `Command::Port(PortArgs { container: String })`.
    - **Modify `crates/dosker-cli/src/commands/mod.rs`**
        - `pub mod port;`.
    - **Create `crates/dosker-cli/src/commands/port.rs`**
        - `execute(args, context, probe, now)` — resolve, recover stale state, reload, print `mapping.endpoint()` in recorded order; no mappings prints nothing and exits `0`.
    - **Verify:** `cargo test -p dosker-cli --features mock-engine --test cli_port`

- [ ] **Task 12: Volume attach test on QEMU**
    - **Implements:** FR-014; roadmap "Done when" (a partitioned FAT16 volume attaches to one running container and survives `rm`; a second attach fails and names the holder; a read-only volume rejects guest writes).
    - **Create `crates/dosker-cli/tests/qemu_volumes.rs` first**
        - Gate every test on `integration_enabled()` and `qemu_image()`, as `qemu_run.rs` does; keep a `static INTEGRATION_LOCK: Mutex<()>` **inside this test binary** to serialize its own tests. The suite is a separate test binary from `qemu_runtime.rs`, so no cross-binary lock exists or is needed; each test uses unique container names.
        - `partitioned_volume_persists_across_rm_and_rejects_a_second_attach` — import the image, `volume create data`, `run -d --name vol -v data:D: freedos:latest`, `exec vol COPY C:\DOSKER\TWO.TXT D:\PERSIST.TXT`, then `run -d --name other -v data:D: freedos:latest`: the second start exits `1` and its output names `vol`; `rm other`. Stop `vol`, `rm vol`, start `vol2` with the same volume, and `exec vol2 TYPE D:\PERSIST.TXT` prints `ONE` and `TWO`.
        - `a_read_only_volume_rejects_a_guest_write` — `volume create rovol`, start with `-v rovol:D:ro`, `exec rovol COPY C:\DOSKER\TWO.TXT D:\BLOCKED.TXT`, then stop. `read_tree` on `<root>/volumes/rovol/volume.img` contains no `BLOCKED.TXT` and the volume still lists as `mbr_partitioned`.
    - **Modify `.github/workflows/ci.yml` and `.github/workflows/qemu-nightly.yml`**
        - Add `cargo test -p dosker-cli --features qemu-engine --test qemu_volumes -- --nocapture` beside the existing `qemu_runtime` and `qemu_run` steps with the same `DOSKER_INTEGRATION`, `DOSKER_QEMU_IMAGE`, and `DOSKER_QEMU` environment.
    - **Verify:** `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img DOSKER_QEMU=qemu-system-i386 cargo test -p dosker-cli --features qemu-engine --test qemu_volumes -- --nocapture`

- [ ] **Task 13: Published-port test on QEMU**
    - **Implements:** FR-015; roadmap "Done when" (a guest TCP service reaches the host at `127.0.0.1:<published>` through the packet driver and the TCP/IP stack).
    - **Modify `crates/dosker-test-support/src/fixtures.rs`**
        - Add `pub const QEMU_NET_IMAGE_ENV: &str = "DOSKER_QEMU_NET_IMAGE";` and `pub fn net_image() -> Result<PathBuf, String>` copying `qemu_image()`: an empty or unset variable returns an error naming `DOSKER_QEMU_NET_IMAGE`; a relative value resolves against the workspace root (`env!("CARGO_MANIFEST_DIR")` plus `../..`); a non-file returns an error with the resolved path.
    - **Create `crates/dosker-cli/tests/qemu_ports.rs` first**
        - `prepared_net_image()` follows `qemu_run.rs::prepared_image`: return `None` when `integration_enabled()` is false; when integration is enabled, call `dosker_test_support::fixtures::net_image()` and panic with its message when it fails. There is no silent skip.
        - The image is imported with `image import --booter`, so no agent is needed and `stop` uses the engine channel.
        - `published_port_reaches_a_guest_tcp_service` — `image import --booter <net image> net-booter:latest`; take a host port from `dosker_engine_qemu::command::free_tcp_port()`; `run -d --name pub -p <port>:80 net-booter:latest`; poll `TcpStream::connect(("127.0.0.1", port))`, send `GET / HTTP/1.0\r\n\r\n`, and assert a response arrives; `dosker port pub` prints the mapping; then `stop pub`. The packet driver and HTTP server start from the image's `AUTOEXEC.BAT`, so no `exec` blocks the supervisor.
        - `a_duplicate_host_port_leaves_the_first_running` — the second `run -d -p <port>:80` exits `1` with `already published`, and `inspect` on the first still reports `running`.
    - **Modify `.github/scripts/check-qemu-variables.sh`**
        - Add `DOSKER_QEMU_NET_IMAGE_URL|raw .img URL of a booter image whose AUTOEXEC.BAT loads an NE2000 packet driver and starts a TCP server on port 80`.
    - **Modify `.github/workflows/ci.yml`**
        - Add `DOSKER_QEMU_NET_IMAGE_URL: ${{ vars.DOSKER_QEMU_NET_IMAGE_URL }}` to the `preflight` step.
        - In the `agent-qemu` job (not `agent-build`), add a download step with `DOSKER_QEMU_NET_IMAGE_URL: ${{ vars.DOSKER_QEMU_NET_IMAGE_URL }}` that writes `agent/fixtures/dosker-net.img`, exports `DOSKER_QEMU_NET_IMAGE` to `$GITHUB_ENV`, and add `cargo test -p dosker-cli --features qemu-engine --test qemu_ports -- --nocapture` with `DOSKER_INTEGRATION=1` and `DOSKER_QEMU=qemu-system-i386`.
    - **Modify `.github/workflows/qemu-nightly.yml`**
        - Add `DOSKER_QEMU_NET_IMAGE_URL` to the nightly `preflight` selection, then add the same download and test step to the `runtime-macos` job.
    - **Verify:** `DOSKER_INTEGRATION=1 DOSKER_QEMU_NET_IMAGE=agent/fixtures/dosker-net.img DOSKER_QEMU=qemu-system-i386 cargo test -p dosker-cli --features qemu-engine --test qemu_ports -- --nocapture`

- [ ] **Task 14: Document volumes and networking, and resolve the FAT12 note**
    - **Implements:** FR-014, FR-015; design §Goals, §Security ("Published ports bind to `127.0.0.1` by default"), NFR `Usability`.
    - **Modify `.sdd/specs/2026-11-01-dosker/data-model.md` first**
        - §Volume `fat_type` row: `fat16` for `mbr_partitioned`, `fat12` for the 1.44 MB superfloppy.
        - The validation rule "A volume uses `mbr_partitioned` for hard-disk sizes and `superfloppy` for floppy sizes" gains the sentence `A superfloppy volume of 1.44 MB is fat12; mbr_partitioned and 2.88 MB volumes are fat16.`
    - **Create `docs/volumes-and-network.md`**
        - `## Volumes` — layout and size rules (`64M` default, the two floppy sizes, the 16 MiB..1 GiB range), `volumes/<name>/volume.img`, `meta.json`, the exclusive `.lock`, `-v name:D[:ro]`, the mountable drive set and layout rule, anonymous volumes from `VOLUME`, survival across `rm`, and `dosker volume create|ls|rm`.
        - `## Publishing ports` — `-p [addr:]host:guest`, the `127.0.0.1` default, the one-port-per-address rule, the conflict error, and `dosker port` output.
        - `## Guest networking` — QEMU user-mode NAT through `ne2k_isa`, the guest packet-driver requirement, and the `CONFIG.SYS` `LASTDRIVE` requirement.
        - `## Integration test fixture` — the booter image required by `DOSKER_QEMU_NET_IMAGE`, its `C:\NET` contents, and that the suite fails loudly when integration is enabled without it.
    - **Modify `README.md`**
        - Insert a `### Volumes and networking` section immediately after the existing `### Images` section and before `### Windows runtime`, containing one paragraph that links `[Volumes and network](docs/volumes-and-network.md)` and names `-v`, `-p`, `dosker volume`, and `dosker port`.
        - In the CI variable lists, document `DOSKER_QEMU_NET_IMAGE_URL`.
    - **Modify `docs/windows.md`**
        - Replace the exact line `NAT is not yet supported.` under `## Engine networking` with a paragraph stating that QEMU user-mode NAT and `-p` work on Windows, that published ports bind to `127.0.0.1` by default, and that volumes live under `%LOCALAPPDATA%\dosker\volumes`.
    - **Verify:** `cargo test --workspace --features dosker-cli/mock-engine && test -f docs/volumes-and-network.md && rg -q 'docs/volumes-and-network.md' README.md && ! rg -q 'NAT is not yet supported' docs/windows.md && rg -q 'superfloppy.*fat12' .sdd/specs/2026-11-01-dosker/data-model.md`
