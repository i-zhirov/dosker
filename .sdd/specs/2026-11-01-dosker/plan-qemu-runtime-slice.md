# QEMU Runtime Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-02
- **Status**: Implemented
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Roadmap**: `.sdd/specs/2026-11-01-dosker/roadmap.md` — slice: `qemu-runtime-slice`
- **Depends on**: `runtime-foundation`, `dos-agent-serial`
- **Manual prerequisites**: QEMU builds for Linux x86_64 and macOS arm64
  (`qemu-system-i386`). A prepared FreeDOS image from the M0 spike that
  already contains `C:\DOSKER\DOSKER.EXE`, a wrapped `AUTOEXEC.BAT`, and
  `C:\DOSKER\CONFIG.INI`. For the integration suite, the human supplies
  the image through `DOSKER_QEMU_IMAGE`. No image builder, no 16-bit
  toolchain, and no OpenWatcom install are needed for this slice.
- **Goal**: Deliver the first usable runtime — a QEMU adapter, prepared
  image import, a private writable disk, the full container lifecycle and
  inspection commands, agent-backed `exec`/`attach`/`cp`, readiness and
  shutdown, `screenshot`, gdbstub pass-through, `engines`, and `doctor` —
  and prove the design's M1 definition of done on Linux x86_64 and macOS
  arm64.

**Approval handoff.** This plan stays `Draft` until the user approves
it. The roadmap slice `qemu-runtime-slice` remains `Status: Unplanned`
with `Plan: —` until then. On approval, set this plan's `Status` to
`Approved`, set the roadmap slice `Status` to `Planned`, and set its
`Plan:` field to
`.sdd/specs/2026-11-01-dosker/plan-qemu-runtime-slice.md`. Do not change
the roadmap before approval.

## Constraints

- Slice 1 (`runtime-foundation`) and Slice 2 (`dos-agent-serial`) are
  `Implemented`. Preserve their interfaces and tests. Extend the engine,
  record, protocol, supervisor, and CLI surfaces; do not replace them.
- The imported DOS image already contains the agent. This slice has no
  image builder, no general agent injection, no OCI layout, no layers, no
  volumes, no NAT, and no Windows host runtime.
- The agent is the long-lived guest process. A container stays running
  after `CMD` exits. `stop` or `rm` ends it.
- `stop` sends `SHUTDOWN`, waits for `SHUTDOWN_ACK`, then stops the engine.
  A kill sets `dirty=true` and keeps the disk.
- A booter image has no agent. `exec`, `attach`, and `cp` return
  `no_agent`. `run`, `screenshot`, `debug`, `stop`, `start`, `restart`,
  `ps`, `inspect`, and `logs` work.
- `create` makes a private writable disk from the image. Use a reflink
  when the host filesystem supports it and a full copy otherwise. The
  image stays read-only. `stop` and `start` keep the disk; `rm` discards
  it.
- Readiness waits up to 60 s for `HELLO` on a normal image. A booter is
  ready when the engine process is alive.
- `--display none|window|vnc` defaults to `none`. `--vnc-port` defaults to
  a free local port. VNC binds to `127.0.0.1` only.
- `cp` on a running container uses the agent and is limited to 16 MiB per
  file. Larger files are deferred to volumes or a stopped-container path.
- Use the existing control protocol, state store, supervisor, transport,
  and agent client. There is no central daemon.
- The control endpoint is a Unix socket on Linux and macOS. Engine
  endpoints are Unix sockets on Linux and macOS. Windows engine endpoints,
  the Job Object, and the Windows runtime are Slice 4.
- Supported hosts in this slice: Linux x86_64 and macOS arm64. The QEMU
  crate must still compile on Windows x86_64 for the workspace matrix; on
  Windows, `QemuAdapter::start` returns `EngineError::Unsupported`.
- Unix socket paths stay at or below 100 bytes. Reuse the Slice 1
  `/tmp/dosker-<euid>/` fallback for engine sockets.
- QEMU guest machine: `-machine pc -cpu 486 -m 16`, VGA, two serial
  ports, one IDE disk. COM2 is the agent link; COM1 stays free.
- The image record is a local directory record with schema version 1. It
  is not an OCI layout; Slice 5 replaces it.
- No transient references in code, identifiers, test names, or strings.
  Do not name slices, tasks, requirement IDs, milestones, or plan paths in
  product code.

## Technical Context

**Language and toolchain.** Rust 1.98.1, edition 2024, one virtual
workspace, synchronous I/O. QEMU is an external process driven through
its command line, QMP, the monitor socket, a serial socket, and a gdbstub
socket. The host reads and writes the guest disk only through the FAT
image boundary described below.

**Crate graph additions.**

| Crate | Depends on | Responsibility |
| --- | --- | --- |
| `dosker-image` | dosker-core, dosker-platform, serde, serde_json, thiserror, chrono, sha2, fatfs | Image reference, minimal image record, local image store, `FatRegion` lookup, `C:\DOSKER\CONFIG.INI` read/write |
| `dosker-engine-qemu` | dosker-core, dosker-engine, dosker-platform, dosker-agent-proto, serde_json, thiserror | QEMU discovery, version, capabilities, command line, session, QMP, serial log |

`dosker-runtime` adds `dosker-image`. `dosker-cli` adds `dosker-image`
and a normal (not optional) `dosker-engine-qemu` dependency, so
`engines` and `doctor` can report QEMU in every build. The
`qemu-engine` Cargo feature is empty and only selects QEMU as the
default engine; it does not gate the dependency. The workspace gains the
members `crates/dosker-image` and `crates/dosker-engine-qemu`, the path
dependencies with the same names, and `fatfs = "0.3"` and
`sha2 = "0.10"`.

`dosker-engine` stays independent of QEMU. The QEMU crate owns every
QEMU option and every QMP command. The supervisor and CLI never format a
QEMU argument.

**Image directory layout.**

```text
<root>/
  images/
    <name>/
      <tag>/
        image.json          minimal image record, schema version 1
        disk.img            read-only raw disk or opaque booter blob
  containers/
    <id>/
      state.json
      disk.img              private writable copy
      engine.log
      serial.log
      container.log
      control.sock
      agent.sock            COM2 serial to the agent
      qmp.sock
      monitor.sock
      gdb.sock
```

Engine socket names are `agent`, `qmp`, `monitor`, and `gdb`. Their
state keys are `serial_agent`, `qmp`, `monitor`, and `gdb`. COM1 stays
free and has no host endpoint in this slice; `attach` is the agent line
shell, not a COM1 terminal. When the default path exceeds 100 bytes, the
path is `/tmp/dosker-<euid>/<id>-<name>.sock`.

**Minimal image record.** `image.json` holds
`schema_version`, `name`, `tag`, `media` (`disk` or `booter`), `digest`
(`sha256:<hex>` of the stored blob), `size`, `source` (the imported path
as text), `agent_version` (nullable), `cmd` (nullable), and `imported_at`.
A `disk` image must have a readable FAT boot sector. A `booter` image is
an opaque blob with no filesystem assumptions.

**QEMU command for a normal image.**

```text
qemu-system-i386
  -machine pc
  -cpu 486
  -m 16
  -no-reboot
  -display none
  -drive file=<container>/disk.img,format=raw,if=ide,index=0
  -serial none
  -serial unix:<container>/agent.sock,server=on,wait=off
  -qmp unix:<container>/qmp.sock,server=on,wait=off
  -monitor unix:<container>/monitor.sock,server=on,wait=off
  -chardev socket,id=gdb0,path=<container>/gdb.sock,server=on,wait=off
  -gdb chardev:gdb0
  -nic none
```

A booter image replaces both serial options with `-serial none` and has
no agent socket. `--display window` uses `-display cocoa` on macOS and
`-display gtk` on Linux. `--display vnc` uses `-display none` plus
`-vnc 127.0.0.1:<display>`, where `<display> = port - 5900. QEMU is started without `-S`, so the gdbstub is
reachable while the guest runs.

**QEMU version and capabilities.** Discovery reads `DOSKER_QEMU`. When
its value has no path separator it is a bare program name and is
resolved through `PATH`; when it contains a path separator and is
relative it is resolved against the working directory. With no
override, discovery searches `PATH` for `qemu-system-i386`. It then runs
`--version` and parses the first `QEMU emulator version X.Y.Z` token.
The adapter declares `boot`, `serial`, `stop`, `headless`,
`debug_native`, `gdbstub`, `screenshot`, and `vnc`. `host_dir_live`,
`nat`, and `audio` are not declared in this slice.

**Engine socket ownership.** QEMU creates and listens on the serial,
QMP, monitor, and gdbstub Unix sockets through `server=on`; the
supervisor connects as the client. The supervisor therefore records
sockets after QEMU has created them rather than binding them first.
This is the engine-specific realization of the socket contract for this
slice and is recorded under Deviations.

**Control protocol additions.** One UTF-8 JSON object per line, as in
Slice 1. New requests and replies:

| Request | Body | Response `result` |
| --- | --- | --- |
| attach | `{"type":"attach"}` | `{"type":"attach"}` |
| cp | `{"type":"cp","direction":"in","source":"/host/f","destination":"C:\\APP\\F"}` | `{"type":"cp","bytes":123}` |
| debug | `{"type":"debug","kind":"gdb"}` | `{"type":"debug","kind":"gdb","endpoint":"/path/gdb.sock"}` |
| screenshot | `{"type":"screenshot","path":"/host/shot.ppm"}` | `{"type":"screenshot","path":"/host/shot.ppm"}` |

New error codes: `no_agent`, `unsupported`, `too_large`,
`guest_rebooted`, and `busy`. `cp` direction is `in` (host to guest) or
`out` (guest to host).

**Pinned values.**

| Value | Setting |
| --- | --- |
| Readiness timeout | 60 s |
| `run -d` and `start` wait | 65 s |
| Default exec timeout | 30 s |
| Default stop grace | 10 s |
| QEMU probe timeout | 5 s |
| QMP connect timeout | 5 s |
| Screenshot wait | 5 s |
| Booter liveness probe | 250 ms |
| Engine stop poll | 20 ms |
| Engine socket retry | 50 ms |
| Image record schema | 1 |
| Image digest | SHA-256, `sha256:<hex>` |
| Control port | COM2 |
| Baud | 115200 |
| `CONFIG.INI` contents | `CONTROL_PORT=2\r\nBAUD=115200\r\n` |
| Maximum `cp` file | 16 MiB (`16777216` bytes) |
| Screenshot format | PPM |
| VNC address | `127.0.0.1` |
| Default display | `none` |
| CLI exit: success | 0 |
| CLI exit: runtime or store error | 1 |
| CLI exit: usage or invalid reference | 2 |
| CLI exit: exec timed out | 124 |
| CLI exit: not running, unreachable, or `no_agent` | 125 |

**Preserved interfaces.** `ContainerRecord` gains fields with
`#[serde(default)]`; existing `state.json` files still load.
`ContainerRecord::new` keeps its signature. `EngineSpec` gains fields;
every constructor in the workspace is updated in Task 4. `EngineSession`
gains defaulted methods so the mock adapter and existing tests compile.
`StateStore`, `recover_stale`, `SystemProbe`, `EndpointRecord`,
`ControlConnection`, `AgentClient`, and the existing control request
handling are extended, not rewritten.

## Tasks

- [x] **Task 1: Image crate, image reference, and FAT image access**
    - **Implements:** FR-005 (a prepared DOS image is accepted as a
      bootable disk), FR-006 (booter media), FR-018 (the image is never
      written at run time); design §Runtime (`CONFIG.INI` before boot),
      §Data Model §Validation Rules.
    - **Modify `Cargo.toml`**
        - Add `"crates/dosker-image"` to `workspace.members`.
        - Add `dosker-image = { path = "crates/dosker-image" }` and
          `fatfs = "0.3"` and `sha2 = "0.10"` to
          `[workspace.dependencies]`.
    - **Create `crates/dosker-image/Cargo.toml`**
        - Inherit `version.workspace`, `edition.workspace`, and
          `license.workspace`.
        - Dependencies: `dosker-core.workspace`, `dosker-platform.workspace`,
          `serde.workspace`, `serde_json.workspace`, `thiserror.workspace`,
          `chrono.workspace`, `sha2.workspace`, `fatfs.workspace`.
        - Dev-dependencies: `tempfile.workspace`, `serde_json.workspace`.
    - **Create `crates/dosker-image/src/lib.rs`**
        - `pub mod error; pub mod fat; pub mod reference;`.
        - Re-export `ImageError`, `ImageReference`, `ImageName`, `ImageTag`,
          `FatLayout`, `FatRegion`, `fat_region`, `write_config`,
          `read_file`, `CONFIG_PATH`, `CONFIG_DIR`, `CONFIG_FILE`,
          `DEFAULT_CONTROL_PORT`, and `DEFAULT_BAUD`.
    - **Create `crates/dosker-image/src/error.rs`**
        - `pub enum ImageError` with
          `InvalidReference(String)`, `NotFound(ImageReference)`,
          `AlreadyExists(ImageReference)`, `NotBootable(PathBuf)`,
          `UnsupportedLayout(PathBuf)`,
          `Corrupt { path: PathBuf, message: String }`,
          `Io { path: PathBuf, source: io::Error }`, and
          `Platform(#[from] PlatformError)`.
        - Messages name the path or the reference.
    - **Create `crates/dosker-image/src/reference.rs`**
        - `ImageName(String)` — `[a-z0-9][a-z0-9_.-]{0,62}`, serde as a
          bare string, `parse`, `as_str`, `Display`.
        - `ImageTag(String)` — `[A-Za-z0-9][A-Za-z0-9_.-]{0,127}`, same
          API.
        - `ImageReference { name: ImageName, tag: ImageTag }` with
          `parse(value: &str) -> Result<Self, ImageError>`: a value with
          no `:` uses the default tag `latest`, so `freedos-test` parses
          as `freedos-test:latest`; a value with a colon splits at the
          first `:` and rejects an empty tag or a second colon. Also
          `name()`, `tag()`, `Display` as `name:tag`, and serde.
    - **Create `crates/dosker-image/src/fat.rs`**
        - `pub const CONFIG_DIR: &str = "DOSKER";`,
          `pub const CONFIG_FILE: &str = "CONFIG.INI";`,
          `pub const CONFIG_PATH: &str = r"C:\DOSKER\CONFIG.INI";`,
          `pub const DEFAULT_CONTROL_PORT: u8 = 2;`,
          `pub const DEFAULT_BAUD: u32 = 115200;`.
        - `pub enum FatLayout { MbrPartitioned, Superfloppy }` with
          `Clone, Copy, Debug, PartialEq, Eq`.
        - `pub struct FatRegion { pub offset: u64, pub length: u64, pub
          layout: FatLayout }` with `Clone, Copy, Debug, PartialEq, Eq`.
        - `pub fn fat_region(image: &Path) -> Result<FatRegion, ImageError>`
          — read the first 512 bytes; when the `0x55AA` signature is
          present and the first partition entry at offset 446 has type
          `0x06` or `0x0E`, return `FatRegion { offset: start_lba * 512,
          length: partition_sectors * 512, layout: MbrPartitioned }`
          after checking the range is inside the file; a superfloppy
          FAT12/FAT16/FAT32 BPB returns `FatRegion { offset: 0, length:
          file_size, layout: Superfloppy }`; otherwise return
          `ImageError::NotBootable`. A partition smaller than the file
          produces a region shorter than file size minus offset.
        - `pub fn write_config(image: &Path, control_port: u8, baud: u32) -> Result<(), ImageError>`
          — open the image read-write, wrap exactly the `FatRegion` range
          in a `Read + Write + Seek` view, mount it with
          `fatfs::FileSystem`, create `DOSKER` when absent, create or
          truncate `CONFIG.INI`, and write
          `CONTROL_PORT={control_port}\r\nBAUD={baud}\r\n`.
        - `pub fn read_file(image: &Path, dos_path: &str) -> Result<Vec<u8>, ImageError>`
          — translate `C:\DOSKER\CONFIG.INI` to `DOSKER/CONFIG.INI`,
          open the FAT root over the `FatRegion`, and return the bytes. A
          missing file is `ImageError::Corrupt` naming the DOS path.
        - The internal `PartitionFile` implements `Read`, `Write`, and
          `Seek` over a base offset and the exact region length, not the
          rest of the file.
    - **Create `crates/dosker-image/tests/support/mod.rs`**
        - `pub fn make_fat16_image(path: &Path, partition_offset: u64, partition_length: u64, total_size: u64)`
          — create the file, write a DOS MBR with one type `0x06`
          bootable partition at `partition_offset` and
          `partition_length` when the offset is non-zero, and format the
          partition as FAT16 with `fatfs::format_volume` and
          `FatType::Fat16`. The file may be larger than the partition;
          the helper writes the partition sector count into the MBR.
        - `pub fn write_fat_file(path: &Path, partition_offset: u64, name: &str, data: &[u8])`.
    - **Create `crates/dosker-image/tests/reference.rs`**
        - `parse_accepts_name_and_tag` — `freedos:freedos-test`.
        - `parse_defaults_to_latest_when_the_tag_is_absent` —
          `freedos-test` becomes `freedos-test:latest`.
        - `parse_rejects_an_empty_tag` — `freedos-test:`.
        - `parse_rejects_an_uppercase_name`.
        - `parse_rejects_an_empty_name`.
        - `parse_rejects_a_second_colon`.
        - `display_round_trips`.
        - `serde_round_trips_as_a_bare_string`.
    - **Create `crates/dosker-image/tests/fat.rs`**
        - `fat_region_reads_an_mbr_partition` — offset 1 MiB, length
          equal to the partition sector count times 512, and layout
          `MbrPartitioned`.
        - `fat_region_uses_the_partition_length_when_the_file_is_larger`
          — a partition with 2048 sectors inside a larger file returns
          `length == 2048 * 512`.
        - `fat_region_accepts_a_superfloppy` — FAT16 at 0 with length
          equal to the file size and layout `Superfloppy`.
        - `fat_region_rejects_a_non_bootable_file`.
        - `fat_region_rejects_a_truncated_file`.
        - `fat_region_rejects_a_partition_past_the_end_of_the_file`.
        - `write_config_creates_the_directory_and_file` — read back the
          exact bytes.
        - `write_config_uses_crlf_and_the_pinned_values` — the payload
          is `CONTROL_PORT=2\r\nBAUD=115200\r\n`.
        - `write_config_is_idempotent` — a second call leaves one file
          with the same content.
        - `write_config_preserves_existing_files` — a pre-existing
          `APP/KEEP.TXT` survives.
        - `read_file_returns_the_config_bytes`.
        - `read_file_reports_a_missing_file`.
    - **Verify:** `cargo test -p dosker-image --test reference --test fat`

- [x] **Task 2: Private disk copy and short engine sockets**
    - **Implements:** FR-018 (reflink or full copy into a private disk),
      FR-008 (per-container engine endpoints), FR-021 (Unix hosts);
      design §Host Platform Abstraction.
    - **Modify `crates/dosker-platform/src/lib.rs`**
        - Re-export `CopyOutcome`, `copy_reflink`, and `copy_full`.
        - Re-export `engine_socket_path` under `#[cfg(unix)]`.
    - **Modify `crates/dosker-platform/src/fs.rs`**
        - `pub enum CopyOutcome { Reflink, FullCopy }` with
          `Clone, Copy, Debug, PartialEq, Eq`.
        - `pub fn copy_reflink(source: &Path, destination: &Path) -> Result<CopyOutcome, PlatformError>`
          — create the destination parent, then on Linux open the source
          and destination and issue `FICLONE` (`0x4004_9409`); on macOS
          use `libc::clonefile` to a temporary sibling and rename it over
          the destination. On a reflink failure, fall back to
          `copy_full` and return `FullCopy`.
        - `#[cfg(windows)] pub fn copy_reflink(source: &Path, destination: &Path) -> Result<CopyOutcome, PlatformError>`
          — call `copy_full` and return `CopyOutcome::FullCopy`. The
          Windows workspace build therefore compiles and the same copy
          tests run.
        - `pub fn copy_full(source: &Path, destination: &Path) -> Result<(), PlatformError>`
          — `std::fs::copy`, open the destination, and `sync_all`.
        - Both functions leave the source bytes unchanged and truncate an
          existing destination before the full copy. After a successful
          copy they set the destination to mode `0o600` on Unix, so a
          read-only image mode never makes the private disk read-only.
    - **Modify `crates/dosker-platform/src/endpoint.rs`**
        - `#[cfg(unix)] pub fn engine_socket_path(root: &Path, id: &ContainerId, name: &str) -> PathBuf`
          — `<root>/containers/<id>/<name>.sock` when the byte length is
          at most `MAX_UNIX_SOCKET_PATH_BYTES`, otherwise
          `<short_socket_dir>/<id>-<name>.sock`.
        - Reuse `unix_path_len`, `short_socket_dir`, and
          `MAX_UNIX_SOCKET_PATH_BYTES`; a `name` is one of `agent`,
          `qmp`, `monitor`, or `gdb`.
    - **Create `crates/dosker-platform/tests/copy.rs`**
        - `copy_reflink_copies_the_contents` — the outcome is either
          variant and the bytes match.
        - `copy_reflink_replaces_an_existing_destination` — a longer old
          file is fully replaced.
        - `copy_reflink_leaves_the_source_unchanged` — the source mode
          and bytes are unchanged.
        - `copy_full_copies_the_contents`.
        - `copy_full_replaces_an_existing_destination`.
        - `copy_creates_the_destination_parent`.
        - `copy_makes_the_destination_writable` (`cfg(unix)`) — a source
          at mode `0o444` produces a destination at mode `0o600`.
        - `copy_reflink_uses_full_copy_on_windows` (`cfg(windows)`) —
          the outcome is `FullCopy` and the bytes match.
    - **Create `crates/dosker-platform/tests/engine_socket.rs`**
        - `engine_socket_uses_the_container_directory` (`cfg(unix)`).
        - `engine_socket_falls_back_for_a_long_root` (`cfg(unix)`).
        - `engine_socket_fallback_is_at_most_100_bytes` (`cfg(unix)`).
        - `engine_socket_names_are_unique` (`cfg(unix)`).
        - `engine_socket_is_usable_by_bind_and_connect` (`cfg(unix)`) —
          bind an `Endpoint::UnixSocket`, connect, round-trip one line,
          remove it.
    - **Verify:** `cargo test -p dosker-platform --test copy --test engine_socket`

- [x] **Task 3: Image record, local image store, and import**
    - **Implements:** FR-005 (import a prepared bootable DOS image),
      FR-006 (booter blob), FR-018 (read-only image); design
      §Build And Runtime Semantics §Build (`image import`).
    - **Create `crates/dosker-image/src/image.rs`**
        - `pub const IMAGE_SCHEMA_VERSION: u32 = 1;`.
        - `pub enum ImageMedia { Disk, Booter }`, deriving
          `Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize`,
          with `#[serde(rename_all = "snake_case")]`,
          `is_booter(self) -> bool`, and `expects_agent(self) -> bool`.
        - `pub struct ImageRecord` with the fields from Technical
          Context, deriving `Clone, Debug, PartialEq, Eq, Serialize,
          Deserialize`.
        - `ImageRecord::is_booter(&self) -> bool` and
          `ImageRecord::expects_agent(&self) -> bool`.
    - **Create `crates/dosker-image/src/store.rs`**
        - `pub const IMAGE_RECORD_FILE: &str = "image.json";` and
          `pub const IMAGE_DISK_FILE: &str = "disk.img";`.
        - `pub struct ImageStore { root: PathBuf }` with
          `new(root: PathBuf) -> Self`, `images_dir() -> PathBuf`,
          `image_dir(&ImageReference) -> PathBuf`,
          `disk_path(&ImageReference) -> PathBuf`.
        - `import(&self, source: &Path, reference: &ImageReference, media: ImageMedia, now: DateTime<Utc>) -> Result<ImageRecord, ImageError>`
          — reject a missing or empty source; for `Disk`, call
          `fat_region(source)` and reject `FatLayout::Superfloppy` with
          `ImageError::UnsupportedLayout`. A disk image must be
          MBR-partitioned because the QEMU command boots it as an IDE
          disk. A booter blob is opaque and hard-disk-only. Then create
          `<image_dir>/disk.img.tmp.<pid>`, copy while hashing with
          `sha2::Sha256`, `sync_all`, rename to `disk.img`; on Unix set
          `disk.img` to mode `0o444`; write `image.json` with
          `dosker_platform::fs::write_atomic`; on any failure remove the
          temporary file. A duplicate reference returns
          `ImageError::AlreadyExists`.
        - `load(&self, reference) -> Result<ImageRecord, ImageError>` —
          read and parse `image.json`; a schema version other than 1 is
          `ImageError::Corrupt`; a missing record is
          `ImageError::NotFound`.
        - `list(&self) -> Result<Vec<ImageRecord>, ImageError>` — walk
          `images/<name>/<tag>/image.json`, skip every other entry, and
          sort by name then tag.
        - `copy_disk_to(&self, reference, destination) -> Result<CopyOutcome, ImageError>`
          — call `dosker_platform::fs::copy_reflink` from the stored
          `disk.img`; the image is never opened for writing.
        - Image directories and the `images/` root are mode `0700` on
          Unix.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Add `pub mod image; pub mod store;` and the re-exports
          `ImageMedia`, `ImageRecord`, `ImageStore`, `IMAGE_RECORD_FILE`,
          and `IMAGE_DISK_FILE`.
    - **Create `crates/dosker-image/tests/store.rs`**
        - `import_copies_the_source_and_records_the_digest` — digest is
          `sha256:` plus 64 lowercase hex characters; size matches;
          `media` is `disk`.
        - `import_leaves_the_source_unchanged`.
        - `import_rejects_a_duplicate_reference`.
        - `import_booter_accepts_an_opaque_blob` — a 1 MiB random file
          imports with `media: booter` and no FAT validation.
        - `import_rejects_a_non_bootable_disk`.
        - `import_rejects_a_superfloppy_disk` — a FAT16 image at offset
          0 fails with `UnsupportedLayout` and does not create a record.
        - `imported_disk_is_read_only` (`cfg(unix)`) — mode `0o444`.
        - `load_rejects_an_unsupported_schema`.
        - `load_reports_a_missing_reference`.
        - `list_returns_images_sorted_by_name_and_tag`.
        - `copy_disk_to_returns_the_same_bytes`.
        - `copy_disk_to_does_not_modify_the_stored_image`.
        - `image_record_round_trips_through_json`.
    - **Verify:** `cargo test -p dosker-image --test store`

- [x] **Task 4: Engine contract extensions, record fields, and store removal**
    - **Implements:** FR-008 (engine lifecycle seam), FR-011 (debugger
      seam), FR-018 (container disk and booter fields), FR-019 (record
      remains recoverable); design §Component Interfaces §Engine Adapter.
    - **Modify `crates/dosker-engine/src/spec.rs`**
        - `pub enum DisplayMode { #[default] None, Window, Vnc }`,
          deriving
          `Clone, Copy, Debug, Default, PartialEq, Eq, Serialize,
          Deserialize`, with `#[serde(rename_all = "snake_case")]`,
          `as_str`, and `parse(&str) -> Option<Self>`. The default is
          `None`, which is the value `#[serde(default)]` uses on
          `ContainerRecord`.
        - `pub enum DebugKind { Gdb }` with the same derives and serde
          form, except `Default`.
        - `pub struct EngineCommand { pub program: PathBuf, pub args:
          Vec<String>, pub env: Vec<(String, String)> }` with
          `render(&self) -> String` that shell-quotes arguments that
          contain whitespace or quotes.
        - `pub struct EngineEndpoint { pub name: String, pub path:
          PathBuf }`.
        - `EngineSpec` gains `pub root: PathBuf`, `pub disk_path:
          PathBuf`, `pub booter: bool`, `pub display: DisplayMode`, and
          `pub vnc_port: Option<u16>`. Existing fields keep their names
          and types.
    - **Modify `crates/dosker-engine/src/adapter.rs`**
        - `EngineAdapter` gains
          `fn build_command(&self, spec: &EngineSpec) -> Result<EngineCommand, EngineError>`
          with a default that returns
          `EngineError::Unsupported("command inspection")`.
        - `EngineAdapter` gains
          `fn start_with_command(&mut self, spec: &EngineSpec, command: &EngineCommand) -> Result<Box<dyn EngineSession>, EngineError>`
          whose default ignores `command` and calls `start(spec)`. The
          supervisor builds once and passes the same command here, so a
          random `vnc_port` allocation is not repeated. `start` remains
          required.
        - `EngineSession` gains defaulted methods:
          `fn command(&self) -> Option<&EngineCommand> { None }`,
          `fn endpoints(&self) -> Vec<EngineEndpoint> { Vec::new() }`,
          `fn screenshot(&mut self, destination: &Path) -> Result<(), EngineError>`
          returning `Unsupported("screenshot")`,
          `fn debug_endpoint(&mut self, kind: DebugKind) -> Result<String, EngineError>`
          returning `Unsupported("debugger")`,
          `fn put_file(&mut self, path: &str, data: &[u8]) -> Result<u64, EngineError>`
          returning `Unsupported("file upload")`, and
          `fn get_file(&mut self, path: &str) -> Result<Vec<u8>, EngineError>`
          returning `Unsupported("file download")`.
    - **Modify `crates/dosker-engine/src/error.rs`**
        - Add `NoAgent` (`the image has no agent`),
          `Unsupported(&'static str)` (`unsupported engine feature: {0}`),
          `GuestRebooted` (`the guest rebooted during the command`), and
          `Agent { code: String, message: String }`
          (`agent error {code}: {message}`).
    - **Modify `crates/dosker-engine/src/lib.rs`**
        - Re-export `DebugKind`, `DisplayMode`, `EngineCommand`, and
          `EngineEndpoint`.
    - **Modify `crates/dosker-runtime/src/record.rs`**
        - Add `#[serde(default)] pub image_digest: String`,
          `#[serde(default)] pub booter: bool`,
          `#[serde(default)] pub display: DisplayMode`,
          `#[serde(default)] pub vnc_port: Option<u16>`,
          `#[serde(default)] pub cmd: Option<String>`, and
          `#[serde(default)] pub engine_command: Option<String>`.
        - `ContainerRecord::new` keeps its signature and sets the new
          fields to their empty values: `image_digest` empty, `booter`
          false, `display` `DisplayMode::None`, `vnc_port` `None`, `cmd`
          `None`, and `engine_command` `None`.
        - Add builder methods:
          `with_image(mut self, digest: String, booter: bool, cmd: Option<String>) -> Self`,
          `with_display(mut self, display: DisplayMode, vnc_port: Option<u16>) -> Self`,
          and `with_disk(mut self, path: PathBuf) -> Self`.
    - **Modify `crates/dosker-runtime/src/paths.rs`**
        - Add `pub fn disk_file(&self, id: &ContainerId) -> PathBuf`
          returning `<container>/disk.img`.
    - **Modify `crates/dosker-runtime/src/store.rs`**
        - Add `pub fn remove(&self, record: &ContainerRecord) -> Result<(), StoreError>`
          — for every recorded value in `record.sockets`, convert it to
          `Endpoint` and call `dosker_platform::control::remove_endpoint`,
          ignoring `NotFound`; this removes the control socket and every
          long-root engine socket. Then remove
          `containers/.names/<name>` only when its `id` file names this
          id, and remove the container directory recursively. A missing
          directory or socket is success.
    - **Modify `crates/dosker-runtime/src/supervisor.rs`**
        - Update the `EngineSpec` literal with `root`,
          `disk_path` (from the record, empty when absent), `booter`,
          `display`, and `vnc_port`. No behavior changes yet.
    - **Modify `crates/dosker-engine/tests/adapter.rs`**
        - Extend `sample_spec` with the new fields.
        - Add `display_mode_names_round_trip`.
        - Add `debug_kind_is_gdb`.
        - Add `engine_command_render_quotes_arguments`.
        - Add `default_session_features_are_unsupported` — a stub session
          reports no endpoints and `Unsupported` for screenshot, debug,
          upload, and download.
        - Add `no_agent_error_message`.
        - Add `guest_rebooted_error_message`.
    - **Modify `crates/dosker-test-support/src/mock_adapter.rs`**
        - Implement `build_command` returning an `EngineCommand` whose
          program is the mock engine path and whose env is the configured
          child env, so the supervisor can record a command for the mock
          engine.
        - Implement `start_with_command` storing the supplied
          `EngineCommand` in `MockSession`; `MockSession::command`
          returns it. `start` delegates to
          `start_with_command` after one `build_command`.
    - **Modify `crates/dosker-test-support/tests/mock_adapter.rs`**
        - Extend its `spec` helper with the new fields.
        - Add `mock_session_reports_no_endpoints`.
        - Add `mock_adapter_builds_a_command_for_inspection`.
    - **Modify `crates/dosker-runtime/tests/record.rs`**
        - Update `sample_record` and the serde round-trip for the new
          fields.
        - Add `new_record_starts_with_runtime_defaults`.
        - Add `with_image_sets_digest_booter_and_cmd`.
        - Add `with_display_sets_mode_and_port`.
        - Add `with_disk_sets_the_disk_path`.
    - **Modify `crates/dosker-runtime/tests/store.rs`**
        - Add `remove_deletes_the_container_disk_and_name_lock`.
        - Add `remove_removes_recorded_control_and_engine_sockets`
          (`cfg(unix)`) — create socket files under the container
          directory and the long-root fallback, record them, remove, and
          assert every path is gone.
        - Add `remove_keeps_other_containers`.
        - Add `remove_is_idempotent`.
        - Add `remove_leaves_a_name_lock_owned_by_another_id`.
    - **Verify:** `cargo test -p dosker-engine --test adapter && cargo test -p dosker-runtime --test record --test store && cargo test -p dosker-test-support --test mock_adapter`

- [x] **Task 5: Control protocol and supervisor command surface**
    - **Implements:** FR-009 (`exec` and `cp` through the agent),
      FR-010 (`attach` shell), FR-011 (`debug` endpoint), FR-018
      (`no_agent` for booters); design §Supervisor, §Error Handling And
      Edge Cases.
    - **Modify `crates/dosker-runtime/src/protocol.rs`**
        - `pub const MAX_CP_BYTES: u64 = 16 * 1024 * 1024;`.
        - `pub enum CpDirection { In, Out }`, deriving
          `Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize`,
          with `#[serde(rename_all = "snake_case")]`.
        - `Request` gains `Attach`, `Cp { direction: CpDirection, source:
          String, destination: String }`, `Debug { kind: DebugKind }`,
          and `Screenshot { path: PathBuf }`.
        - Update `parse_request`'s hardcoded known-type match with
          `attach`, `cp`, `debug`, and `screenshot`; without this the
          new variants are reported as `unknown_message`. The match runs
          before body deserialization.
        - `Reply` gains `Attach`, `Cp { bytes: u64 }`, `Debug { kind:
          DebugKind, endpoint: String }`, and `Screenshot { path:
          PathBuf }`.
        - `ErrorCode` gains `NoAgent`, `Unsupported`, `TooLarge`,
          `NoFile`, `GuestRebooted`, and `Busy`, serialized as
          `no_agent`, `unsupported`, `too_large`, `no_file`,
          `guest_rebooted`, and `busy`.
        - Re-export `CpDirection` and `MAX_CP_BYTES`.
    - **Modify `crates/dosker-runtime/src/supervisor.rs`**
        - Refactor the supervisor to `Arc<Mutex<SupervisorState>>` where
          `SupervisorState` holds `record`, `session`, `container_log`,
          and a `shutdown: bool`. The accept loop spawns one thread per
          connection and keeps polling `session.try_exit` under the
          lock. A connection thread holds the lock only while
          dispatching one request, never while waiting on an idle
          attached connection.
        - Add `pub const ATTACH_POLL: Duration = Duration::from_secs(1)`.
          An attached connection sets its read timeout to
          `ATTACH_POLL`; a timeout checks the shared `shutdown` flag and
          `session.try_exit`, then continues. This keeps the connection
          alive across an idle pause while allowing `stop` and `status`
          on another connection to be served and engine death to be
          noticed.
        - `dispatch_request` gains:
          - `Request::Attach` — reply `NotRunning` unless `running`;
            reply `NoAgent` when `record.booter`; otherwise reply
            `Reply::Attach` and mark the connection attached. The CLI
            owns the line loop and sends `exec` requests on the same
            connection; each exec uses its own `timeout_ms`. A read EOF,
            malformed line, or a client disconnect ends the connection;
            an idle connection stays open until `shutdown`.
          - `Request::Cp` — reply `NotRunning` unless `running`; reply
            `NoAgent` when `record.booter`; for `In`, call
            `std::fs::metadata(source)` and reject
            `metadata.len() > MAX_CP_BYTES` with `TooLarge` before
            reading, then read the file and call
            `session.put_file(destination, &data)`; for `Out`, call
            `session.get_file(source)`, reject a returned `Vec` over
            `MAX_CP_BYTES` with `TooLarge` before writing anything, then
            write the host destination with `std::fs::write`; reply
            `Reply::Cp { bytes }`. Both directions enforce the same
            bound without reading an oversized upload. Every engine
            error goes through `engine_error_code`, so a missing guest
            file is `NoFile` and an agent `TOO_LARGE` is `TooLarge`; a
            failed `cp` never calls `session.kill`, never marks `exited`,
            and leaves the container `running`.
          - `Request::Debug` — reply `NotRunning` unless `running`; call
            `session.debug_endpoint(kind)`; reply `Reply::Debug {
            kind, endpoint }`; `EngineError::Unsupported` maps to
            `ErrorCode::Unsupported`.
          - `Request::Screenshot` — reply `NotRunning` unless `running`;
            call `session.screenshot(&path)`; reply
            `Reply::Screenshot { path }`; `EngineError::Unsupported`
            maps to `ErrorCode::Unsupported`.
        - `serve` adds a `Dispatch::Attach(Response)` variant. After
          writing the `Reply::Attach` response it marks the connection
          attached and sets `ATTACH_POLL`; a read timeout checks
          `shutdown` and `session.try_exit` and then continues. Only EOF,
          a malformed line, a client disconnect, or a stop closes an
          attached connection. `serve` runs on its own thread, so a
          second connection can serve `stop`, `status`, or `exec` while
          the attach connection is idle.
        - `run_exec` replies `ErrorCode::NoAgent` before calling the
          session when `record.booter` is true.
        - `run_exec` error paths: `EngineError::GuestRebooted` calls
          `session.kill`, marks `exited` with `dirty=true`, and replies
          `ErrorCode::GuestRebooted`; `EngineError::Agent { code:
          "BUSY" }` replies `ErrorCode::Busy` and leaves the container
          `running`; every other non-timeout error calls `session.kill`
          before marking `exited`, then replies `engine_error_code`.
        - Add `fn engine_error_code(error: &EngineError) -> ErrorCode`
          with one mapping per variant: `NoAgent` -> `NoAgent`,
          `Unsupported` -> `Unsupported`, `ExecTimeout` -> `Timeout`,
          `GuestRebooted` -> `GuestRebooted`,
          `Agent { code: "TOO_LARGE" }` -> `TooLarge`,
          `Agent { code: "NO_FILE" }` -> `NoFile`,
          `Agent { code: "BUSY" }` -> `Busy`; every other error is
          `EngineError`. Add `engine_error_code_covers_each_typed_case`
          asserting each of the seven mappings.
    - **Modify `crates/dosker-runtime/src/lib.rs`**
        - Re-export the new protocol items.
    - **Modify `crates/dosker-test-support/src/mock_adapter.rs`**
        - `MockSession` gains `files: BTreeMap<String, Vec<u8>>`.
        - Implement `put_file` storing the bytes and returning the
          length, and `get_file` returning the stored bytes or
          `EngineError::Agent` with code `NO_FILE`.
        - Add `MockAdapter::with_endpoints(self, endpoints: Vec<EngineEndpoint>)`
          and return them from `MockSession::endpoints`.
        - Add `MockAdapter::with_files(self, files: BTreeMap<String, Vec<u8>>)`
          and seed `MockSession::files`, so a download can start from a
          known guest file.
    - **Modify `crates/dosker-runtime/tests/protocol.rs`**
        - Change `parse_request_reports_unknown_type` from `attach` to
          `fork`.
        - Add `parse_request_accepts_attach`.
        - Add `parse_request_accepts_cp_in_and_out`.
        - Add `parse_request_accepts_debug_gdb`.
        - Add `parse_request_accepts_screenshot`.
        - Add `response_round_trip_for_attach_cp_debug_screenshot`.
        - Add `no_agent_unsupported_too_large_no_file_guest_rebooted_and_busy_codes_are_snake_case`.
    - **Modify `crates/dosker-runtime/tests/supervisor.rs`**
        - Change `unknown_message_returns_unknown_message_and_keeps_connection`
          to send `{"type":"fork"}` instead of `{"type":"attach"}`.
          `attach` is a known request after this task; `fork` stays
          unknown and the connection must remain open for the following
          `ping`.
        - Do not add a disk helper here. This task's supervisor tests do
          not need a private disk; Task 8 adds the single
          `tests/support/mod.rs` helper and the `fatfs` dev-dependency.
        - Add `exec_on_a_booter_returns_no_agent`.
        - Add `attach_rejects_a_booter`.
        - Add `cp_rejects_a_booter`.
        - Add `attach_rejects_a_stopped_container`.
        - Add `attach_idle_connection_is_not_closed` — send `Attach`,
          wait 6 s (longer than `SERVER_READ_TIMEOUT`), then send
          `Exec ECHO HELLO` on the same connection and receive `HELLO`.
        - Add `attach_runs_a_slow_command` — send `Attach`, then
          `Exec SLEEP 1000` with `timeout_ms=5000`; the reply is not a
          timeout and the connection stays open.
        - Add `stop_succeeds_while_attach_connection_is_idle` — open
          `Attach` on connection A and leave it idle; send `Stop` on
          connection B; B receives `stopped` without waiting for the
          attach timeout, and A reaches EOF.
        - Add `cp_uploads_a_small_file` — the mock session stores the
          bytes.
        - Add `cp_downloads_a_small_file` — seed the mock session file.
        - Add `cp_rejects_an_upload_over_the_limit`.
        - Add `cp_rejects_a_download_over_the_limit` — the mock returns
          a `Vec` larger than `MAX_CP_BYTES` and no host file is written.
        - Add `cp_maps_an_agent_too_large_error` — an
          `EngineError::Agent { code: "TOO_LARGE" }` reply becomes
          `ErrorCode::TooLarge`.
        - Add `cp_missing_guest_file_keeps_the_container_running` — an
          `EngineError::Agent { code: "NO_FILE" }` reply becomes
          `ErrorCode::NoFile`, the record stays `running`, and the
          session is not killed.
        - Add `cp_host_destination_write_failure_keeps_the_container_running`
          — a missing host parent directory returns an
          `ErrorCode::Internal` reply naming the path, the record stays
          `running`, and the session is not killed.
        - Add `exec_surfaces_guest_rebooted` — a stub session returning
          `EngineError::GuestRebooted` replies `ErrorCode::GuestRebooted`.
        - Add `exec_surfaces_agent_busy` — a stub session returning
          `EngineError::Agent { code: "BUSY" }` replies
          `ErrorCode::Busy`.
        - Add `screenshot_returns_unsupported`.
        - Add `debug_returns_unsupported`.
    - **Verify:** `cargo test -p dosker-runtime --test protocol --test supervisor`

- [x] **Task 6: QEMU discovery, version, capabilities, and command line**
    - **Implements:** FR-011 (engine and debugger capability model),
      FR-021 (QEMU discovery on Linux and macOS); design §Engine
      Strategy, §Engine Capability Matrix.
    - **Modify `Cargo.toml`**
        - Add `"crates/dosker-engine-qemu"` to `workspace.members` and
          `dosker-engine-qemu = { path = "crates/dosker-engine-qemu" }`
          to `[workspace.dependencies]`.
    - **Create `crates/dosker-engine-qemu/Cargo.toml`**
        - Dependencies: `dosker-core.workspace`, `dosker-engine.workspace`,
          `dosker-platform.workspace`, `dosker-agent-proto.workspace`,
          `serde_json.workspace`, `thiserror.workspace`.
        - Dev-dependencies: `tempfile.workspace`, `serde_json.workspace`.
    - **Create `crates/dosker-engine-qemu/src/lib.rs`**
        - `pub mod adapter; pub mod discovery;`.
        - `#[cfg(unix)] pub mod command; #[cfg(unix)] pub mod paths;
          #[cfg(unix)] pub mod qmp; #[cfg(unix)] pub mod session;
          #[cfg(unix)] pub mod serial_log;`.
        - Re-export `QemuAdapter`, `QemuBinary`, `QemuVersion`,
          `QEMU_ENV`, and `DEFAULT_QEMU`; re-export `QemuPaths` under
          `#[cfg(unix)]`.
    - **Create `crates/dosker-engine-qemu/src/discovery.rs`**
        - `pub const QEMU_ENV: &str = "DOSKER_QEMU";`,
          `pub const DEFAULT_QEMU: &str = "qemu-system-i386";`, and
          `pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5)`.
        - `pub struct QemuVersion { pub major: u32, pub minor: u32, pub
          patch: u32 }` with `new`, `parse(line: &str) -> Option<Self>`,
          `Display`, and `Ord`.
        - `pub struct QemuBinary { pub path: PathBuf, pub version:
          QemuVersion }` with
          `discover() -> Result<Self, EngineError>`,
          `from_path(path: PathBuf) -> Result<Self, EngineError>`, and
          `version_string(&self) -> String`.
        - `discover` treats a `DOSKER_QEMU` value with no path separator
          as a bare program name and searches every `PATH` entry for it.
          A value with a path separator is path-like: an absolute value
          is used as-is and a relative one is resolved against the
          current directory. With no override it searches `PATH` for
          `DEFAULT_QEMU`. It then runs `--version` through a bounded
          wait: spawn with piped stdout/stderr, poll `try_wait` every
          `20 ms`, kill the child at `PROBE_TIMEOUT` (5 s), and read the
          captured output. A timeout is
          `EngineError::Setup("qemu --version timed out")`; a non-zero
          exit, a missing binary, or an unparsable version is a typed
          `EngineError` naming the path. `from_path` uses the same
          bounded wait.
    - **Create `crates/dosker-engine-qemu/src/paths.rs`**
        - `pub struct QemuPaths { pub agent_serial: PathBuf, pub qmp:
          PathBuf, pub monitor: PathBuf, pub gdb: PathBuf }`.
        - `QemuPaths::for_spec(spec: &EngineSpec) -> Self` — call
          `dosker_platform::endpoint::engine_socket_path(&spec.root,
          &spec.container_id, name)` for `agent`, `qmp`, `monitor`, and
          `gdb`.
        - `QemuPaths::remove_stale(&self)` — remove every socket file,
          ignoring `NotFound`.
    - **Create `crates/dosker-engine-qemu/src/command.rs`**
        - `pub fn build(spec: &EngineSpec, binary: &QemuBinary) -> Result<EngineCommand, EngineError>`
          — the command from Technical Context, using
          `QemuPaths::for_spec`. When `spec.display` is `Vnc` and
          `spec.vnc_port` is `None`, call `free_vnc_port` so the
          allocated port is already in `5900..=65535`.
        - `pub fn display_arguments(display: DisplayMode, vnc_port: Option<u16>) -> Result<Vec<String>, EngineError>`
          — `none` gives `["-display","none"]`; `window` gives
          `["-display","cocoa"]` on macOS and `["-display","gtk"]` on
          Linux; `vnc` gives `["-display","none","-vnc",
          "127.0.0.1:<display>"]`, where `<display> = port - 5900` and
          `port` must be in `5900..=65535`; when `vnc_port` is `None`,
          call `free_vnc_port` first. QEMU's `-vnc` suffix is a display
          number, not a TCP port.
        - `pub fn free_vnc_port() -> Result<u16, EngineError>` — bind
          `127.0.0.1:0`; if the assigned port is below 5900, bind a free
          port in `5900..=65535` and return it; drop the listener. This
          is the default when `--vnc-port` is absent.
        - `pub fn free_tcp_port() -> Result<u16, EngineError>` — bind
          `127.0.0.1:0`, read the port, and drop the listener; retained
          for non-VNC callers.
        - Every disk and booter blob is attached as
          `-drive file=<disk>,format=raw,if=ide,index=0`; superfloppy
          disk imports are rejected by `image import`, so the runner
          never needs `if=floppy`.
        - A booter spec emits `-serial none` twice and no agent socket.
    - **Create `crates/dosker-engine-qemu/src/adapter.rs`**
        - `pub struct QemuAdapter { binary: QemuBinary, version: String,
          processes: Arc<dyn ProcessSupervisor> }`.
        - `QemuAdapter::new(binary: QemuBinary, processes: Arc<dyn ProcessSupervisor>) -> Self`,
          `QemuAdapter::discover(processes: Arc<dyn ProcessSupervisor>) -> Result<Self, EngineError>`,
          and `binary(&self) -> &QemuBinary`.
        - `EngineAdapter` impl: name `qemu`; `version` returns the stored
          string; `capabilities` returns
          `[Boot, Serial, Stop, Headless, DebugNative, GdbStub,
          Screenshot, Vnc]`; `probe` checks `binary.path.is_file()`.
          Under `#[cfg(unix)]`, `build_command` delegates to
          `command::build` and `start` returns
          `EngineError::Unsupported("QEMU session")` until Task 7. Under
          `#[cfg(windows)]`, `build_command` returns
          `Unsupported("QEMU command line")` and `start` returns
          `Unsupported("QEMU runtime on this host")`. The crate compiles
          on Windows because every Unix-only module is cfg-gated.
    - **Create `crates/dosker-engine-qemu/tests/discovery.rs`**
        - One file-level `static ENV_LOCK: Mutex<()>` guards every test
          that touches `DOSKER_QEMU`, `PATH`, or the current directory.
          A file-local `EnvGuard` snapshots and restores those values on
          drop; on edition 2024, `std::env::set_var` and
          `set_current_dir` calls are inside `unsafe` blocks with the
          lock held.
        - `version_parses_a_three_part_version`.
        - `version_parses_a_distribution_suffix`.
        - `version_rejects_a_non_version_line`.
        - `discover_uses_the_environment_override` (`cfg(unix)`) — a
          temporary executable script prints the version line; the
          environment is set per child and restored through the shared
          lock and guard.
        - `discover_finds_a_bare_name_on_path` (`cfg(unix)`) — the
          `DOSKER_QEMU` value is only the program name, a temporary
          directory on `PATH` contains the script, and discovery
          resolves it; uses the same lock and guard.
        - `discover_resolves_a_path_like_relative_value` (`cfg(unix)`) —
          a value with a separator is resolved against the working
          directory; uses the same lock and guard, restoring the
          original current directory.
        - `discover_times_out_a_hanging_binary` (`cfg(unix)`) — a stub
          script sleeps for 10 s; discovery returns
          `EngineError::Setup` within 6 s and the child is killed; uses
          the same lock and guard when it sets `DOSKER_QEMU`.
        - `discover_reports_a_missing_binary` — sets `DOSKER_QEMU` and
          an empty `PATH`; uses the same lock and guard.
        - `probe_reports_the_parsed_version`.
    - **Create `crates/dosker-engine-qemu/tests/command.rs`** — starts
      with `#![cfg(unix)]`.
        - `headless_command_uses_display_none`.
        - `command_uses_the_private_disk_and_two_serial_ports`.
        - `command_uses_ide_for_the_disk_and_booter` — `if=ide` is
          present and `if=floppy` is absent for both media.
        - `command_uses_the_conformance_machine` — `-machine pc`,
          `-cpu 486`, and `-m 16` are present; no `-soundhw` or
          sound-device option is emitted; `-nic none` is present.
        - `command_exposes_qmp_monitor_and_gdb_sockets`.
        - `command_uses_the_host_agent_socket_for_a_normal_image`.
        - `booter_command_has_no_agent_socket`.
        - `command_translates_the_vnc_port_to_a_display_number` — a
          requested port `5905` emits `-vnc 127.0.0.1:5`.
        - `command_rejects_a_vnc_port_below_5900` — `5899` is an error.
        - `command_accepts_the_vnc_port_upper_bound` — `65535` emits
          display `59635`.
        - `command_uses_a_free_vnc_port_when_vnc_port_is_absent` — the
          emitted port is in `5900..=65535`.
        - `command_uses_the_window_display_on_each_host` —
          `cfg(target_os = "macos")` expects `cocoa`,
          `cfg(target_os = "linux")` expects `gtk`.
        - `command_disables_networking`.
        - `capabilities_are_the_proven_set`.
    - **Create `crates/dosker-engine-qemu/tests/paths.rs`** — starts
      with `#![cfg(unix)]`.
        - `paths_use_the_container_directory`.
        - `paths_fall_back_for_a_long_root`.
        - `paths_are_unique_per_container`.
        - `remove_stale_removes_existing_sockets`.
    - **Verify:** `cargo test -p dosker-engine-qemu --test discovery --test command --test paths`

- [x] **Task 7: QEMU session — serial agent, readiness, shutdown, QMP, screenshot, gdbstub**
    - **Implements:** FR-009 (agent-backed execution), FR-010 (agent
      link), FR-011 (gdbstub pass-through), FR-018 (readiness and disk
      lifecycle); design §Component Interfaces §Engine Adapter,
      §Runtime.
    - **Modify `crates/dosker-engine-qemu/src/lib.rs`**
        - Under `#[cfg(unix)]`, add `pub mod qmp; pub mod session; pub mod
          serial_log;` and re-export `QemuSession` and `QMP_CONNECT_TIMEOUT`.
        - `QemuPaths::remove_stale` returns `io::Result<()>`; propagate its
          error from `QemuSession::spawn`.
    - **Create `crates/dosker-engine-qemu/src/serial_log.rs`**
        - `pub struct SerialLog<T> { inner: T, log: File }` with
          `new(inner: T, log: File) -> Self`.
        - Implement `Read`, `Write`, and `dosker_agent_proto::Transport`
          by forwarding to `inner` and appending every read and written
          byte to `log`; timeout setters forward to `inner`.
        - The log is flushed after every write and after every read.
    - **Create `crates/dosker-engine-qemu/src/qmp.rs`**
        - `pub struct QmpClient` over `UnixStream` and `BufReader`.
        - `QmpClient::connect(path: &Path, timeout: Duration) -> Result<Self, EngineError>`
          — connect, read the greeting, and send `qmp_capabilities`.
        - `execute(&mut self, command: &str, arguments: Option<Value>) -> Result<Value, EngineError>`
          — write one JSON object, skip `event` lines, return the
          `return` value, and surface an `error` object as
          `EngineError::Protocol`.
        - `quit(&mut self) -> Result<(), EngineError>` — send `quit`;
          an EOF after the write counts as success, because QEMU may
          close the control socket while it exits.
        - `screendump(&mut self, path: &Path, format: Option<&str>) -> Result<(), EngineError>`
          — send `screendump` with `filename` and, when present, `format`.
        - `QMP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5)`.
    - **Modify `crates/dosker-agent-proto/src/client.rs`**
        - Add `pub fn shutdown_with_timeout(&mut self, grace_ms: u64, timeout: Duration) -> Result<ShutdownAck>`
          — the same `SHUTDOWN` request as `shutdown`, but it sends and
          waits for `SHUTDOWN_ACK` under the caller's deadline instead
          of the frozen 500 ms session timeout. The existing `shutdown`
          method keeps its signature and behavior.
        - Add `shutdown_with_timeout_waits_for_a_delayed_ack` in
          `crates/dosker-agent-proto/tests/client.rs` — the mock agent
          sends `SHUTDOWN_ACK` after 1 s and the call succeeds with a
          10 s timeout.
    - **Create `crates/dosker-engine-qemu/src/session.rs`**
        - `pub struct QemuSession` with `child: Child`, `binary:
          QemuBinary`, `spec: EngineSpec`, `command: EngineCommand`,
          `paths: QemuPaths`, `agent:
          Option<AgentClient<SerialLog<UnixStream>>>`, `qmp:
          Option<QmpClient>`, and `exit_code: Option<i32>`.
        - `pub fn spawn(binary: &QemuBinary, spec: &EngineSpec, processes: &dyn ProcessSupervisor, command: EngineCommand) -> Result<Self, EngineError>`
          — create `log_dir`, remove stale sockets, open
          `engine.log` append, spawn with stdin null and stdout/stderr
          appended to `engine.log`, and apply `prepare_engine`.
        - `impl Drop for QemuSession` — when the child is still alive
          (`try_wait` returns `Ok(None)`), call `Child::kill` and
          `Child::wait`. This is the last-resort cleanup for every
          supervisor error path, including macOS where there is no
          parent-death signal. It runs after the explicit kill paths and
          is idempotent.
        - `fn qmp(&mut self) -> Result<&mut QmpClient, EngineError>` —
          lazy connection. When `qmp` is `None`, call
          `QmpClient::connect(&paths.qmp, QMP_CONNECT_TIMEOUT)` (5 s);
          store it on success and return it. A connect or handshake
          failure returns the typed `EngineError` and leaves `qmp` as
          `None`, so the next call retries. `ready` does not connect QMP;
          QMP becomes `Some` on the first `stop` or `screenshot`.
        - `EngineSession` impl:
          - `command` returns `Some(&self.command)`.
          - `pid` and `process_group` return the child id on Unix and
            `None` for the group on Windows.
          - `ready(timeout)` — booter: sleep `BOOTER_ALIVE_PROBE` and
            return `Exited(code)` when the child exited, otherwise
            `Ok`; normal: on every `50 ms` agent-socket retry call
            `child.try_wait` and return `EngineError::Exited(code)`
            immediately when QEMU died, instead of waiting for the
            socket deadline. After connecting, open `serial.log`
            append, wrap the stream in `SerialLog`, call
            `AgentClient::handshake` with the remaining deadline, and
            map `HandshakeTimeout` to `ReadyTimeout`.
          - `exec(request)` — `NoAgent` for a booter; otherwise call
            `AgentClient::exec`, map `ProtoError::Timeout` to
            `ExecTimeout`, map `ProtoError::GuestRebooted` to
            `EngineError::GuestRebooted`, map `ProtoError::Agent` to
            `EngineError::Agent` (including a `BUSY` code), and return
            `ExecResult` with `String::from_utf8_lossy` output,
            `timed_out=false`, and the agent `truncated` flag.
          - `put_file` and `get_file` — `NoAgent` for a booter;
            `AgentClient::put(path, data, true)` and
            `AgentClient::get(path)`; return the byte count and bytes.
            A `ProtoError::TransferTooLarge` maps to
            `EngineError::Agent { code: "TOO_LARGE", message }`, so the
            supervisor can reply `too_large` for either direction.
          - `stop(grace)` — for a normal image, call
            `AgentClient::shutdown_with_timeout(grace_ms, grace)` so the
            `SHUTDOWN_ACK` may arrive anywhere inside the 10 s grace
            window rather than the frozen 500 ms session timeout; remember
            a failure. Then call `qmp()` and `quit()`, ignoring a QMP
            connect or quit failure. Poll `try_exit` every `20 ms` until
            the deadline. When the process exits within the grace period
            and the shutdown was acknowledged, return `Ok(exit_code)`, which
            the supervisor records as a clean `stopped`. When the
            process exits but the shutdown was not acknowledged, return
            `Err(EngineError::Protocol("shutdown was not acknowledged"))`
            so the supervisor records `exited` with `dirty=true`. When
            the deadline passes, call `kill` and return
            `Err(EngineError::StopTimeout(grace))`; the supervisor maps
            that to `exited` with `dirty=true` and keeps the disk. A
            booter skips the agent: a successful QMP `quit` is its
            acknowledgement, so a booter that exits within the grace
            period after a successful quit returns `Ok` and the
            supervisor records `stopped` with `dirty=false`; a failed
            QMP quit falls through to the timeout/kill path. A kill or
            timeout never returns `Ok`.
          - `kill` — `Child::kill`, `wait`, and return `137` on Unix or
            `-1` on Windows when there is no code.
          - `try_exit` — `Child::try_wait`.
          - `endpoints` — `serial_agent`, `qmp`, `monitor`, and `gdb`
            with their `QemuPaths` files; a booter omits `serial_agent`,
            because the engine has no agent socket. There is no COM1
            endpoint.
          - `screenshot(destination)` — call `qmp()`; a connect failure
            returns the typed `EngineError` and writes no file. On
            success call `screendump` with `format: "ppm"` when the
            parsed version is at least `7.1.0` and without it otherwise,
            then wait up to 5 s for a non-empty file; a timeout is
            `EngineError::Protocol`.
          - `debug_endpoint(Gdb)` — return the gdb socket path when the
            file exists, otherwise `Unsupported("debugger")`.
    - **Modify `crates/dosker-engine-qemu/src/adapter.rs`**
        - `start` calls `build_command` and then
          `start_with_command`.
        - `start_with_command` calls `QemuSession::spawn` with
          `self.processes.as_ref()` and the supplied command; it does
          not rebuild the command. On `cfg(windows)` both methods return
          `Unsupported("QEMU runtime on this host")`.
    - **Create `crates/dosker-engine-qemu/tests/serial_log.rs`** —
      starts with `#![cfg(unix)]`.
        - `serial_log_records_reads_and_writes` — a scripted
          `Transport` writes and reads bytes; the log file contains both
          directions in order.
        - `serial_log_forwards_timeouts` — the read and write timeout
          calls reach the inner transport.
    - **Create `crates/dosker-engine-qemu/tests/qmp.rs`** — starts with
      `#![cfg(unix)]`.
        - A test server binds a temporary Unix socket, writes the QMP
          greeting, reads capabilities, and answers scripted commands.
        - `qmp_connect_sends_capabilities`.
        - `qmp_execute_returns_the_return_value`.
        - `qmp_quit_sends_quit`.
        - `qmp_screendump_sends_filename_and_format`.
        - `qmp_ignores_event_lines`.
        - `qmp_surfaces_an_error_response`.
        - `qmp_reports_a_closed_socket`.
    - **Create `crates/dosker-engine-qemu/tests/qemu_session.rs`**
        - `#![cfg(unix)]`; every test returns early unless
          `DOSKER_INTEGRATION=1` and `DOSKER_QEMU_IMAGE` names a file.
        - `prepared_image_session_boots_execs_and_stops` — copy the
          image to a temp disk, build a spec, start, `ready(60s)`,
          `exec ECHO HELLO`, assert `HELLO` and status 0, then `stop`.
        - `booter_session_is_ready_without_an_agent` — `booter=true`;
          `ready` succeeds and `exec` returns `NoAgent`.
        - `booter_session_stops_cleanly` — `booter=true`; after a
          successful QMP quit, `stop(10s)` returns `Ok(0)` and does not
          return the `shutdown was not acknowledged` error.
        - `ready_reports_an_early_exit` — a command with an invalid QEMU
          argument exits immediately; `ready(60s)` returns
          `EngineError::Exited(code)` within 2 s instead of
          `ReadyTimeout`.
        - `session_exposes_sockets_for_screenshot_and_debug` — endpoints
          contain `serial_agent`, `qmp`, `monitor`, and `gdb`; the gdb
          path exists after boot.
        - `session_retains_the_private_disk` — the disk file still
          exists after `stop`.
        - `dropping_a_running_session_kills_qemu` — start and ready a
          session, drop it without `stop`, and assert the child PID is
          gone within 2 s.
        - `screenshot_connects_qmp_lazily` — the screenshot succeeds
          after a boot without a prior `stop`, and the QMP socket file
          exists.
    - **Verify:** `cargo test -p dosker-agent-proto --test client && cargo test -p dosker-engine-qemu --test serial_log --test qmp && cargo test -p dosker-engine-qemu --test qemu_session` (the session suite reports the gated tests as early returns without the environment)

- [x] **Task 8: Supervisor integration — image, disk, config, endpoints, CMD**
    - **Implements:** FR-008 (one supervised engine per container),
      FR-018 (private disk and `CONFIG.INI` before boot), FR-019
      (recoverable state); design §Process Model, §Runtime.
    - **Modify `crates/dosker-runtime/Cargo.toml`**
        - Add `dosker-image.workspace = true`.
        - Add `fatfs.workspace = true` under `[dev-dependencies]`; this
          is the only FAT test helper in the runtime crate.
    - **Modify `crates/dosker-runtime/tests/support/mod.rs`**
        - Create this once here, not in Task 5. Add
          `pub fn make_test_disk(root: &Path, id: &ContainerId) -> PathBuf`
          writing `containers/<id>/disk.img` with a locally formatted
          16 MiB FAT16 superfloppy and an `AUTOEXEC.BAT`, using the
          `fatfs` dev-dependency. Do not import helpers from
          `crates/dosker-image/tests`.
        - Add `pub fn read_config(disk: &Path) -> Vec<u8>` delegating to
          `dosker_image::read_file`.
    - **Modify `crates/dosker-runtime/src/supervisor.rs`**
        - Before `adapter.probe`:
          - Resolve `record.disk_path`; a missing path is a failed start.
          - When `record.booter` is false, call
            `dosker_image::write_config(&disk, DEFAULT_CONTROL_PORT,
            DEFAULT_BAUD)`; a failure is a failed start.
        - Build `EngineSpec` with `root`, `container_id`, `workdir`
          `C:\`, `env` empty, `cmd` from `record.cmd`, `expects_agent`
          `!record.booter`, `log_dir`, `disk_path`, `booter`, `display`,
          and `vnc_port`.
        - Call `adapter.build_command(&spec)` exactly once, store
          `record.engine_command = Some(command.render())`, append one
          deterministic line `engine <rendered command>` to
          `engine.log`, and pass that same `EngineCommand` to
          `adapter.start_with_command(&spec, &command)`. Never call
          `adapter.start` after building, because a second build would
          allocate a different free VNC port. The line makes
          `logs --engine` non-empty even when QEMU writes nothing to
          stdout or stderr.
        - After start, insert every `session.endpoints()` entry into
          `record.sockets` as `EndpointRecord::UnixSocket { path }`. The
          keys are `serial_agent`, `qmp`, `monitor`, and `gdb`; a booter
          has no `serial_agent`.
        - `run_stop` keeps its existing error path but is now exercised
          by `EngineError::StopTimeout`: a timeout or a forced kill calls
          `session.kill`, transitions to `exited`, sets `dirty=true`,
          clears the pids, and replies `exited`. It never reports
          `stopped` for a kill.
        - After the `running` transition, when `spec.cmd` is `Some` and
          the image is not a booter, call `session.exec` with the
          command, the image workdir, the empty env, and
          `DEFAULT_EXEC_TIMEOUT_MS`:
          - On success, append one `cmd exit=<code> timed_out=false
            command=<command>` line to `container.log`.
          - On `EngineError::ExecTimeout`, call `session.kill`, transition
            `running -> exited`, set `exit_code=Some(124)`, set
            `dirty=true`, clear the pids, set `finished_at`, save, and end
            the supervisor with code 0. The private disk is kept. This
            matches the command-timeout rule in the design.
          - On `EngineError::GuestRebooted`, call `session.kill`,
            transition `running -> exited`, set `dirty=true`, clear the
            pids, set `finished_at`, save, and end the supervisor with
            code 0. The private disk is kept. This is the same mapping
            as `run_exec`.
          - On `EngineError::Agent { code: "BUSY" }`, append `cmd busy
            command=<command>` and keep the container running.
          - On every other command error, call `session.kill`, append
            `cmd failed: <error> command=<command>`, transition to
            `exited`, set `dirty=true`, clear the pids, save, and end
            the supervisor. This is the generic kill-and-exit path.
            The mock CMD timeout is never relied on for these tests.
        - `failed_start` gains the container's `serial.log` path. It
          writes `start failed: <error>` to `container.log`, then, when
          the error is `EngineError::ReadyTimeout`, reads the last
          non-empty serial line and appends:
          `last serial line: <line>; retry with a larger --boot-timeout`.
          With no serial line it writes `last serial line: <none>; retry with a larger --boot-timeout`. The
          disk is kept and the existing `failed` transition runs.
    - **Modify `crates/dosker-runtime/tests/supervisor.rs`**
        - Add a test-only `StubSession` implementing `EngineSession` with
          a mode enum `ExecTimeout` / `ExecGuestRebooted` /
          `ExecProtocolError` / `StopTimeout` / `CleanStop`.
          `ready` returns `Ok`; `exec` returns
          `Err(EngineError::ExecTimeout(Duration::from_millis(1)))`,
          `Err(EngineError::GuestRebooted)`, or
          `Err(EngineError::Protocol("boom"))` in the three exec modes;
          `stop` returns
          `Err(EngineError::StopTimeout(Duration::from_millis(1)))` in
          `StopTimeout` mode and `Ok(0)` in `CleanStop` mode.
          `pid` and `process_group` return `None`, `try_exit` returns
          `Ok(None)`, and `kill` sets an `Arc<AtomicBool>` kill flag and
          returns `Ok(0)`. Add a `StubAdapter` implementing
          `EngineAdapter` whose `build_command` returns a dummy
          `EngineCommand` and whose `start` returns the selected
          `StubSession` plus the kill flag. The mock engine is not
          changed: its CMD timeout would be 30 s and its `stop` always
          succeeds.
        - Update every setup to create the test disk and set
          `record.disk_path` and `record.booter`.
        - Add `supervisor_writes_config_before_boot` — the disk contains
          `C:\DOSKER\CONFIG.INI` with the pinned values.
        - Add `supervisor_skips_config_for_a_booter`.
        - Add `supervisor_records_engine_endpoints` — a mock adapter
          configured with `serial_agent` and `qmp` endpoints records
          both under `sockets`.
        - Add `supervisor_records_the_engine_command` — the record has a
          rendered command and `engine.log` contains the `engine ` line.
          Update `supervisor_creates_engine_serial_and_container_logs`
          for the non-empty `engine.log`.
        - Add `supervisor_executes_the_recorded_command` — a
          `MockAdapter` stores the command it receives in
          `start_with_command`; the session's `command().render()` equals
          `record.engine_command`. A second build is a failure.
        - Add `supervisor_sends_cmd_after_ready` — a `cmd` record
          produces one `cmd` log line and the container stays running.
        - Add `supervisor_marks_exited_when_cmd_times_out` — use
          `StubAdapter` in `ExecTimeout` mode with `record.cmd =
          Some("SLEEP 5000")`; the record becomes `exited`,
          `dirty=true`, `exit_code=Some(124)`, pids cleared,
          `finished_at` set, and the disk still present.
        - Add `supervisor_marks_exited_dirty_when_stop_must_kill` — use
          `StubAdapter` in `StopTimeout` mode; the record becomes
          `exited`, `dirty=true`, pids cleared, and the disk present.
          This is the killed-stop proof.
        - Add `supervisor_stops_a_booter_cleanly` — use `StubAdapter` in
          `CleanStop` mode with `record.booter=true`; the record becomes
          `stopped`, `dirty=false`, and the disk remains.
        - Add `supervisor_marks_exited_when_cmd_reboots_the_guest` —
          use `StubAdapter` in `ExecGuestRebooted` mode with a `cmd`
          record; the record becomes `exited`, `dirty=true`, pids
          cleared, and the disk remains.
        - Add `exec_generic_error_kills_the_engine` — use `StubAdapter`
          in `ExecProtocolError` mode; the record becomes `exited` with
          `dirty=true` and the stub kill flag is set.
        - Add `supervisor_reports_the_last_serial_line_on_boot_timeout`
          — seed `serial.log` with `DOS IS BOOTING`, set the mock ready
          delay above a short timeout, and assert `container.log` names
          the line and `--boot-timeout`.
        - Add `supervisor_fails_when_the_disk_is_missing`.
    - **Verify:** `cargo test -p dosker-runtime --test supervisor`

- [x] **Task 9: CLI image import, create, start, restart, rm, and run**
    - **Implements:** FR-005 (`image import`), FR-006 (`--booter`),
      FR-008 (detached supervisor start), FR-018 (private disk), FR-019
      (`start` and `restart` recover stale state); design §CLI,
      §Runtime.
    - **Modify `crates/dosker-cli/Cargo.toml`**
        - Add `dosker-image.workspace = true`.
        - Add `fatfs.workspace = true` as a dev-dependency for the test
          image helper.
    - **Modify `crates/dosker-cli/src/context.rs`**
        - `Context` gains `pub images: ImageStore`; `with_console`
          constructs it from the same root.
    - **Modify `crates/dosker-cli/src/error.rs`**
        - Add `Image(#[from] ImageError)`.
        - `CliError::exit_code` maps
          `Image(ImageError::InvalidReference(_))` to `EXIT_USAGE` (2),
          matching the pinned table. Other image errors stay `EXIT_ERROR`
          (1).
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - Add `Command::Image(ImageArgs)`, `Create(CreateArgs)`,
          `Start(StartArgs)`, `Restart(RestartArgs)`, and `Rm(RmArgs)`.
        - `ImageArgs` has `#[command(subcommand)] command: ImageCommand`;
          `ImageCommand::Import(ImageImportArgs)` with `file: PathBuf`,
          `reference: String`, and `booter: bool` (`--booter`).
        - `CreateArgs { image: String, name: Option<String>, engine:
          Option<String>, display: String, vnc_port: Option<u16> }`.
        - `StartArgs { container: String, boot_timeout: u64 }` with
          `--boot-timeout` default 60.
        - `RestartArgs { container: String, boot_timeout: u64, time:
          u64 }` with `--boot-timeout` 60 and `--time` 10.
        - `RmArgs { container: String }`.
        - `RunArgs` gains `display: String` (`--display`, default
          `none`), `vnc_port: Option<u16>` (`--vnc-port`), and
          `boot_timeout: u64` (`--boot-timeout`, default 60).
        - `SuperviseArgs` gains `ready_timeout_ms: u64`
          (`--ready-timeout-ms`, default 60000).
        - Dispatch the new commands.
    - **Modify `crates/dosker-cli/src/commands/mod.rs`**
        - Declare `image`, `create`, `start`, `restart`, and `rm`.
    - **Create `crates/dosker-cli/src/commands/image.rs`**
        - `execute(args: ImageArgs, context: &mut Context, now:
          DateTime<Utc>) -> Result<u8, CliError>` — parse the reference,
          call `context.images.import(&source, &reference, media, now)`,
          and print `imported <name:tag> <digest> <size> bytes`.
        - `--booter` selects `ImageMedia::Booter`; otherwise
          `ImageMedia::Disk`.
    - **Create `crates/dosker-cli/src/commands/create.rs`**
        - `pub fn create_container(context: &mut Context, image:
          &str, name: Option<String>, engine: String, display:
          DisplayMode, vnc_port: Option<u16>, now: DateTime<Utc>) ->
          Result<ContainerRecord, CliError>`.
        - It validates the engine first with `adapter::is_known`; an
          unknown name is `CliError::Usage` and exits 2.
        - It parses the reference, loads the image record, generates the
          id and name, builds a `created` record with
          `with_image(record.digest, record.is_booter(), record.cmd)`,
          `with_display(display, vnc_port)`, creates the state entry,
          copies the image to `context.paths.disk_file(&id)` with
          `copy_disk_to`, sets `disk_path`, saves, and returns. On a copy
          or save failure it calls `store.remove` and returns the error.
        - `execute` prints the id.
    - **Create `crates/dosker-cli/src/commands/start.rs`**
        - `pub fn spawn_supervisor(context: &Context, root: &Path, id:
          &ContainerId, engine: &str, ready_timeout_ms: u64) ->
          Result<Child, CliError>` — the existing `run` spawn logic plus
          `--ready-timeout-ms`.
        - `pub struct StartOutcome { pub record: ContainerRecord, pub
          timed_out: bool }`.
        - `pub fn start_container(context: &mut Context, record:
          &ContainerRecord, ready_timeout: Duration) ->
          Result<StartOutcome, CliError>` — spawn and poll until
          `max(65 s, ready_timeout + 5 s)`. It returns the final record
          (`running`, `failed`, or `exited`) and prints nothing. A spawn
          or store error is the only `Err`.
          If the deadline passes while the record is still `starting`,
          do not send `Request::Stop`: `starting -> stopping` is
          forbidden and the supervisor is blocked in `ready`. Instead
          kill the supervisor's process group and the recorded engine
          group through `ProcessSupervisor::kill_group`, poll
          `process_alive` for both for at most 5 s, then call
          `recover_stale` (or save `exited` with `dirty=true` when
          recovery has no probe). Return
          `StartOutcome { record, timed_out: true }`; the status is
          `exited`. Before any kill, reload the record and re-check its
          status: a record that became `running`, `failed`, `stopped`, or
          `exited` is returned without killing and with `timed_out=false`.
          The supervisor saves the engine pids and sockets before
          `ready`, so a timeout while `starting` can kill the engine group.
          Output belongs to the caller: `start` prints `{name}
          {status}`; `run -d` prints exactly one ID line on success; a
          non-detached `run` proceeds to attach.
        - Extract the poll into a testable
          `wait_for_start(store, id, deadline, probe, processes)`
          helper. Add `start_container_times_out_without_stopping_from_starting`
          using a fabricated `starting` record and a fake probe whose
          process is alive until the timeout; assert the helper kills
          the groups directly, never sends Stop, and returns `exited`
          with `timed_out=true`. Add
          `a_starting_record_without_an_engine_pgid_kills_the_supervisor_group`
          and `a_record_that_became_running_at_the_deadline_is_not_killed`.
        - `execute(args: StartArgs, context: &mut Context, probe: &dyn HostProbe, now: DateTime<Utc>) -> Result<u8, CliError>`
          — resolve, recover stale,
          reject `starting`, `running`, `stopping`, and `failed` with a
          message naming `rm` for a failed container, print a warning
          when `record.dirty` is true, call `start_container`, print
          `{name} {status}` for `outcome.record`, and return 0 only for
          `running`. When the final status is `failed` or the outcome is
          timed out, also print the container log to stderr after the
          status line, so the boot-timeout diagnostic is visible.
    - **Create `crates/dosker-cli/src/commands/restart.rs`**
        - `execute` — resolve, recover stale; reject `failed` with a
          message naming `rm` and reject `starting`/`stopping`; print the
          same `record.dirty` warning as `start`; when running, send
          `Request::Stop { grace_ms: time * 1000 }` and wait for the
          reply. Start a second supervisor only when the reply is
          `Reply::Stop { status }` with `stopped` or `exited`, or a
          fresh `store.load` is terminal (`stopped`/`exited`); a timeout,
          transport error, or any other status aborts with exit 1 and
          leaves the existing supervisor alone. Then call
          `start_container` with the boot timeout and print
          `{name} {status}` from `outcome.record`; when the final status
          is `failed`, print the container log to stderr.
        - Add `restart_does_not_start_when_stop_times_out` — a stub
          control endpoint accepts the connection but never replies;
          `restart` exits 1 and the container record still has the
          original supervisor PID, proving no second supervisor was
          spawned.
    - **Create `crates/dosker-cli/src/commands/rm.rs`**
        - `execute` — resolve, recover stale; reject `starting`,
          `running`, and `stopping`; call `store.remove`; print the
          name.
    - **Modify `crates/dosker-cli/src/commands/run.rs`**
        - Replace the record creation with `create_container` and the
          supervisor wait with `start_container`.
        - Map `--display` with `DisplayMode::parse`; an invalid value is
          a usage error.
        - When the final record is `running`, print exactly one line:
          the container id. When it is `failed` or `exited` (including a
          `StartOutcome` timeout), print `{name} {status}` on stdout,
          print the container log to stderr, and return exit 1; never
          print the id for a failed start. Existing
          `run_reports_a_failed_start` keeps asserting `failed` on
          stdout.
        - Validate the selected engine with `adapter::is_known` before
          creating the record; an unknown engine is `CliError::Usage`
          and exits 2.
        - `-d` remains required in this task; Task 10 removes the
          requirement and attaches.
    - **Modify `crates/dosker-cli/src/commands/supervise.rs`**
        - Pass `Duration::from_millis(args.ready_timeout_ms)` to
          `SupervisorConfig`.
    - **Modify `crates/dosker-cli/tests/common/mod.rs`**
        - Add `pub fn make_disk_image(path: &Path)` creating a 32 MiB
          file with a DOS MBR and one bootable type `0x06` FAT16
          partition starting at LBA 2048 (offset 1 MiB), formatted with
          `fatfs`, plus an `AUTOEXEC.BAT`. It must be MBR-partitioned,
          not the Task 8 superfloppy helper, because `image import`
          rejects superfloppies. All Task 9 CLI fixtures use this
          helper; `detached_lifecycle.rs` uses it too.
        - Add `pub fn import_image(&self)` that runs `image import` and
          returns the reference.
        - Add `pub fn base_command(&self) -> Command` — set only
          `DOSKER_ROOT`; do not call `mock_engine_path()` and do not set
          `DOSKER_MOCK_ENGINE`. `command()` becomes `base_command` plus
          the mock engine, for mock tests only.
        - Change `spawn_with_stdin`, `spawn_with_stdin_chunks`, and
          `spawn_captured` to take a `Command` built by the caller
          (`command()` for mock tests, `command_with_qemu()` for QEMU
          tests) plus the remaining arguments. Task 10 passes
          `command()`; Task 13 passes `command_with_qemu()`. No QEMU
          helper calls `mock_engine_path()`.
        - `spawn_with_stdin(&self, command: Command, args: &[&str],
          stdin: &str) -> Output` — pipe the scripted stdin, capture
          stdout and stderr to files, and enforce a timeout.
        - `spawn_with_stdin_chunks(&self, command: Command, args:
          &[&str], chunks: &[(Duration, &str)]) -> Output` — write each
          scripted chunk after its delay; Task 10 uses it to pause an
          attached shell past the old 5 s idle timeout.
        - `spawn_captured(&self, command: Command, args: &[&str]) ->
          (Child, PathBuf, PathBuf, tempfile::TempDir)` — spawn a
          long-running command with file captures; Task 11 uses it for
          `logs --follow` and owns the TempDir.
        - `start()` imports a test image before `run -d`.
    - **Modify `crates/dosker-cli/tests/detached_lifecycle.rs`**
        - Import a generated FAT16 test image before `run -d` in the
          short-root and long-root setups, either through the shared
          `common` helper or a local `image import` step. For the
          redirected-`DOSKER_ROOT` test, import into the root that
          `run -d` uses; leave the host-default root empty so the
          existing empty-root assertion still holds.
        - Keep the existing state, orphan, and `DOSKER_ROOT` assertions;
          the only change is that the image now exists before `run` in
          the root the command uses.
    - **Create `crates/dosker-cli/tests/cli_image.rs`**
        - `image_import_prints_the_reference_and_digest`.
        - `image_import_booter_accepts_an_opaque_blob`.
        - `image_import_rejects_a_missing_file`.
        - `image_import_rejects_a_non_bootable_disk`.
        - `image_import_accepts_the_mbr_cli_fixture` — the fixture from
          `make_disk_image` imports successfully.
        - `image_import_rejects_a_superfloppy_disk` — a FAT16 image at
          offset 0 fails with `UnsupportedLayout`.
        - `image_import_rejects_a_duplicate_reference`.
        - `image_import_without_a_tag_uses_latest` — the output and
          record use `freedos-test:latest`.
        - `image_import_rejects_an_invalid_reference_with_exit_two` —
          an uppercase name or an empty tag returns
          `ImageError::InvalidReference` and exits 2.
    - **Create `crates/dosker-cli/tests/cli_lifecycle.rs`**
        - `create_copies_the_image_to_a_private_disk`.
        - `create_does_not_modify_the_image`.
        - `create_records_the_digest_and_booter_flag`.
        - `create_resolves_an_untagged_image_reference` — `create
          freedos-test` finds `freedos-test:latest`.
        - `create_rejects_a_missing_image`.
        - `create_rejects_a_duplicate_name`.
        - `create_rejects_an_unknown_engine` — exit 2.
        - `start_returns_a_stopped_container_to_running`.
        - `start_rejects_a_running_container`.
        - `start_rejects_a_failed_container` — the message names `rm`.
        - `start_warns_when_the_disk_is_dirty`.
        - `start_prints_name_and_status`.
        - `restart_stops_and_starts`.
        - `restart_starts_a_stopped_container`.
        - `restart_rejects_a_failed_container`.
        - `restart_warns_when_the_disk_is_dirty` — the same warning as
          `start`.
        - `rm_removes_the_container_and_disk`.
        - `rm_rejects_a_running_container`.
    - **Modify `crates/dosker-cli/tests/cli_run.rs`**
        - Update the fixture to import a test image first.
        - Add `run_records_the_display_and_vnc_port`.
        - Add `run_passes_the_boot_timeout_to_the_supervisor`.
        - Add `run_detached_prints_only_one_id_line`.
        - Update `run_reports_a_failed_start` — stdout contains the
          `{name} {status}` line with `failed`, stderr contains the
          container log, and no id is printed.
        - Add `run_reports_a_boot_timeout_with_the_last_serial_line` — a
          too-small `--boot-timeout` against the delayed mock ready
          event prints the container log containing
          `last serial line` and `--boot-timeout`.
        - Keep `run_requires_detach` until Task 10.
    - **Verify:** `cargo test -p dosker-cli --features mock-engine --test cli_image --test cli_lifecycle --test cli_run`

- [x] **Task 10: CLI exec, attach, cp, screenshot, debug, and non-detached run**
    - **Implements:** FR-009 (`exec` and `cp`), FR-010 (`attach`),
      FR-011 (`debug`), FR-018 (`no_agent` surfaces); design §CLI,
      §Runtime.
    - **Modify `crates/dosker-cli/src/commands/exec.rs`**
        - Map `ErrorCode::NoAgent` to `EXIT_NOT_RUNNING` with the
          message `the image has no agent`.
        - Map `ErrorCode::TooLarge`, `ErrorCode::Unsupported`,
          `ErrorCode::GuestRebooted`, and `ErrorCode::Busy` to exit 1,
          printing the supervisor message. `guest_rebooted` and `busy`
          are surfaced, not retried. `ErrorCode::EngineError` prints the
          supervisor message and keeps exit 125.
    - **Create `crates/dosker-cli/src/commands/attach.rs`**
        - `pub const ATTACH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);`.
        - `pub enum ShellAction { Exec { command: String, workdir:
          String }, Detach }`.
        - `pub struct LineShell { workdir: String }` with
          `new(workdir: &str) -> Self` and
          `handle(&mut self, line: &str) -> ShellAction` — `cd <path>`
          updates the workdir and returns `Exec` with an empty command;
          `exit`, `quit`, and `detach` return `Detach`; blank lines
          return `Exec` with an empty command; every other line is a
          command.
        - `execute` — resolve, recover stale, require `running`, connect
          to the control endpoint, set the read and write timeouts to
          `ATTACH_COMMAND_TIMEOUT + IO_MARGIN` (30 min plus the 5 s
          control-client margin), send `Request::Attach`, read the
          reply, then loop: print `C:\>`; read a line from stdin; on
          `Detach` break; on a non-empty command send `Request::Exec`
          with the shell workdir and `timeout_ms =
          ATTACH_COMMAND_TIMEOUT` on the same connection, read the
          reply, and print `output` verbatim; `NoAgent` prints the
          message and exits 125. A local read timeout exits 124. A
          failed connect exits 125. A `cd` or blank line
          changes only the local workdir and sends no request. An idle
          pause does not close the connection; the supervisor has
          disabled its idle read timeout.
        - `pub fn run_shell(endpoint: &EndpointRecord, workdir: &str,
          input: impl BufRead, output: &mut Console) -> Result<u8,
          CliError>` for `run` reuse; it uses the same attached-mode
          connection, `ATTACH_COMMAND_TIMEOUT + IO_MARGIN`, and idle
          behavior.
    - **Create `crates/dosker-cli/src/commands/cp.rs`**
        - `pub const CP_TIMEOUT: Duration = Duration::from_secs(30 * 60);`.
        - `pub enum CpOperand { Container { record: ContainerRecord,
          path: String }, Host(PathBuf) }`.
        - `parse_operand(context: &Context, value: &str) ->
          Result<CpOperand, CliError>` — a `:`-separated value whose
          prefix parses as a container reference is a container operand.
          A valid reference that does not resolve reports
          container-not-found, and other store errors propagate; only a
          prefix that is not a valid reference is a host path. An empty
          container-side path is a usage error. A Windows drive-letter
          path (`[A-Za-z]:[\\/]...`) is always a host path, even when a
          one-letter container happens to share the letter. Exactly one
          operand must be a container.
        - `execute` — resolve the container side, recover stale,
          require `running`, then send
          `Request::Cp { direction, source, destination }` with the DOS
          path on the container side and the absolute host path on the
          other side; print `copied <bytes> bytes`. Use
          `control_client::request(..., CP_TIMEOUT)` so a 16 MiB
          transfer at 115200 baud has room; `MAX_CP_BYTES` remains the
          hard bound.
        - The CLI performs the host-source check before sending a
          request. For direction `in`, `metadata(source)` must show a
          file no larger than `MAX_CP_BYTES`; a missing, non-file, or
          oversized source returns `CliError::Io` or a `too_large`
          message and sends no control message. For direction `out`, a
          missing host parent directory returns `CliError::Io`. The
          supervisor keeps a defensive error for a race but the CLI owns
          the user-facing check.
        - Map `NoAgent` to exit 125; `TooLarge`, `NoFile`, and other
          engine errors to exit 1 with the supervisor message. A failed
          `cp` never stops or removes the container.
        - Unit tests cover Windows drive-path recognition on a
          non-Windows host.
    - **Create `crates/dosker-cli/src/commands/screenshot.rs`**
        - `execute(args: ScreenshotArgs, context: &mut Context, probe: &dyn HostProbe, now: DateTime<Utc>) -> Result<u8, CliError>`
          — resolve, recover, require `running`, make the output path
          absolute, send `Request::Screenshot`, print `screenshot
          <path>`.
        - `ErrorCode::Unsupported` exits 1 with
          `screenshot is not supported by this engine`.
    - **Create `crates/dosker-cli/src/commands/debug.rs`**
        - `execute(args: DebugArgs, context: &mut Context, probe: &dyn HostProbe, now: DateTime<Utc>) -> Result<u8, CliError>`
          — resolve, recover, require `running`; when `--gdb` is set send
          `Request::Debug { kind: DebugKind::Gdb }` and print
          `target remote <endpoint>`; without `--gdb`, print the same
          endpoint with the label `native debugger`.
        - `ErrorCode::Unsupported` exits 1, names the engine, and names
          QEMU as the engine that provides the debugger capability.
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - Add `Attach(AttachArgs)`, `Cp(CpArgs)`, `Screenshot(ScreenshotArgs)`,
          and `Debug(DebugArgs)`.
        - `AttachArgs { container: String }`.
        - `CpArgs { source: String, destination: String }`.
        - `ScreenshotArgs { container: String, output: PathBuf }` with
          `-o/--output`.
        - `DebugArgs { container: String, gdb: bool }` with `--gdb`.
        - Dispatch the new commands.
    - **Modify `crates/dosker-cli/src/commands/run.rs`**
        - Remove the `-d` requirement. Without `-d`, after the container
          is running, call `attach::run_shell` against the record's
          endpoint; when the shell detaches, leave the container
          running and return 0. When the container is a booter, print
          an informational line about the missing agent, do not attach,
          and return 0. The non-detached path does not print the
          container id; the detached path keeps the single-ID output
          from Task 9.
    - **Modify `crates/dosker-cli/src/commands/mod.rs`**
        - Declare `attach`, `cp`, `screenshot`, and `debug`.
    - **Modify `crates/dosker-cli/tests/cli_exec.rs`**
        - Update the fixture to import a test image.
        - Add `exec_on_a_booter_exits_125_with_no_agent`.
        - Add `exec_prints_guest_rebooted_and_exits_one` — a stub
          listener returns `ErrorCode::GuestRebooted`.
        - Add `exec_prints_busy_and_exits_one` — a stub listener returns
          `ErrorCode::Busy`.
        - Add `exec_prints_an_engine_error_and_exits_125` — a stub
          listener returns `ErrorCode::EngineError`, the message is
          printed, and the exit code stays 125.
    - **Create `crates/dosker-cli/tests/cli_attach.rs`**
        - `attach_runs_a_command_and_exits_on_exit` — use
          `spawn_with_stdin(command(), ...)` with `ECHO HELLO\nexit\n`;
          stdout contains `HELLO`; the container is still running.
        - `attach_survives_an_idle_pause` — a scripted stdin writer
          waits longer than 5 s before sending `ECHO HELLO\nexit\n`;
          the command still succeeds.
        - `attach_runs_a_slow_command` — `SLEEP 1000` followed by
          `exit`; the command completes under
          `ATTACH_COMMAND_TIMEOUT`.
        - `attach_rejects_a_booter` — exit 125.
        - `attach_rejects_a_stopped_container` — exit 125.
        - `cd_updates_the_workdir` — a unit test of `LineShell`.
        - `exit_detaches`.
        - `blank_line_is_ignored`.
        - `run_shell_reads_scripted_input` — `run_shell` with a
          `Cursor` over `ECHO HELLO\nexit\n` and a scripted control
          endpoint prints `HELLO` and returns 0.
        - `run_shell_returns_timeout_when_the_guest_times_out` — a stub
          returns `Reply::Exec { timed_out: true }` and `run_shell`
          returns 124.
        - `run_shell_writes_guest_output_verbatim` — output without a
          trailing newline is not given one.
        - `run_shell_reports_an_unreachable_endpoint_as_not_running`
          (`cfg(unix)`) — a connect failure returns 125.
    - **Create `crates/dosker-cli/tests/cli_cp.rs`**
        - `cp_uploads_a_host_file_to_the_guest` — the mock session
          stores the bytes; a following download round-trips them.
        - `cp_downloads_a_guest_file_to_the_host` — a cross-process
          setup: upload a known host file to `C:\DOSKER\ROUND.TXT` with
          `cp`, then download that guest path to a new host file and
          compare the bytes. Do not seed the mock session inside the
          supervisor process.
        - `cp_rejects_two_host_paths`.
        - `cp_rejects_two_container_paths`.
        - `cp_rejects_an_empty_container_path` — a container operand
          with nothing after the colon exits 2.
        - `cp_reports_a_missing_container_reference` (`cfg(not(windows))`)
          — a valid reference that does not resolve is not a host path.
        - `cp_treats_a_non_reference_colon_path_as_host`
          (`cfg(not(windows))`).
        - `cp_missing_host_file_exits_one_without_a_request` — the
          error names the path and no control request is made.
        - `cp_rejects_a_file_over_the_limit`.
        - `cp_rejects_an_oversized_source_before_sending` — the host
          metadata check returns `too_large` and no request is made.
        - `cp_transfers_a_non_trivial_file` — a 256 KiB upload and
          download complete under `CP_TIMEOUT`.
        - `cp_treats_a_windows_drive_path_as_host` (`cfg(windows)`) — a
          `C:\...` value is not parsed as a container even when a
          one-letter container exists.
        - `cp_rejects_a_booter_with_no_agent`.
        - `cp_missing_guest_file_exits_one_and_keeps_running` — the
          error names the guest path; the container remains `running`.
        - `cp_host_destination_write_failure_exits_one_and_keeps_running`
          — a missing host parent directory does not stop the container.
    - **Create `crates/dosker-cli/tests/cli_screenshot.rs`**
        - `screenshot_reports_unsupported_for_the_mock_engine`.
        - `screenshot_rejects_a_stopped_container`.
    - **Create `crates/dosker-cli/tests/cli_debug.rs`**
        - `debug_gdb_prints_a_target_command` — a stub listener returns
          a debug reply.
        - `debug_reports_unsupported_for_the_mock_engine`.
        - `debug_rejects_a_stopped_container`.
    - **Modify `crates/dosker-cli/tests/cli_run.rs`**
        - Remove `run_requires_detach`.
        - Add `run_without_detach_attaches_and_leaves_the_container_running`
          — use `spawn_with_stdin(command(), ...)` with
          `ECHO HELLO\nexit\n`; stdout contains `HELLO` and the record
          stays `running`.
        - Add `run_without_detach_detaches_on_exit` — the attached
          prompt is printed, the container stays `running`, and the id
          is not printed.
        - Add `run_without_detach_on_a_booter` — a booter prints an
          informational line, returns 0, stays `running`, and does not
          print the id.
    - **Verify:** `cargo test -p dosker-cli --features mock-engine --test cli_exec --test cli_attach --test cli_cp --test cli_screenshot --test cli_debug --test cli_run`

- [x] **Task 11: CLI inspect, logs, and ps updates**
    - **Implements:** FR-008 (inspectable supervised state), FR-019
      (state recovered before inspection); design §Non-Functional
      Requirements §Observability, §CLI.
    - **Create `crates/dosker-cli/src/commands/inspect.rs`**
        - `execute(args: InspectArgs, context: &mut Context, probe:
          &dyn HostProbe, now: DateTime<Utc>) -> Result<u8, CliError>` —
          resolve, recover stale, print
          `serde_json::to_string_pretty(&record)` plus a newline.
        - `InspectArgs { container: String }`.
    - **Create `crates/dosker-cli/src/commands/logs.rs`**
        - `LogsArgs { container: String, follow: bool, engine: bool,
          serial: bool }` with `--engine` and `--serial` mutually
          exclusive; the default stream is `container.log`.
        - `execute` — resolve, recover stale, choose
          `engine.log`, `serial.log`, or `container.log`, print its
          current bytes verbatim, and when `--follow` is set print new
          bytes every 200 ms until interrupted.
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - Add `Inspect(InspectArgs)` and `Logs(LogsArgs)`.
        - Dispatch both.
    - **Modify `crates/dosker-cli/src/commands/mod.rs`**
        - Declare `inspect` and `logs`.
    - **Modify `crates/dosker-cli/src/commands/ps.rs`**
        - Keep the header `ID NAME IMAGE STATUS CREATED`; a record with
          the new fields still prints the same columns.
    - **Modify `crates/dosker-cli/tests/cli_ps.rs`**
        - Update constructed records with the new fields.
        - Add `ps_lists_a_created_container_with_all`.
        - Add `ps_header_is_unchanged`.
    - **Create `crates/dosker-cli/tests/cli_inspect.rs`**
        - `inspect_prints_the_record_json`.
        - `inspect_includes_the_effective_engine_command`.
        - `inspect_includes_the_private_disk_path`.
        - `inspect_unknown_container_exits_one`.
    - **Create `crates/dosker-cli/tests/cli_logs.rs`**
        - `logs_prints_the_container_log`.
        - `logs_prints_the_engine_log`.
        - `logs_prints_the_serial_log`.
        - `logs_rejects_both_engine_and_serial`.
        - `logs_unknown_container_exits_one`.
        - `logs_follow_prints_appended_lines` — use
          `spawn_captured(command(), ["logs","--follow",...])`, append a
          line to the log, wait for it in the captured stdout, then kill
          the child.
    - **Verify:** `cargo test -p dosker-cli --features mock-engine --test cli_inspect --test cli_logs --test cli_ps`

- [x] **Task 12: CLI engines, doctor, and the QEMU feature**
    - **Implements:** FR-011 (report engine capabilities), FR-021
      (engine discovery on Linux and macOS); design §CLI, §Engine
      Capability Matrix, §Security And Privacy.
    - **Modify `crates/dosker-cli/Cargo.toml`**
        - Add `dosker-engine-qemu.workspace = true` as a normal
          dependency, so `engines` and `doctor` report QEMU in every
          build.
        - Add `[features] qemu-engine = []`; the feature selects QEMU as
          the default engine. It does not gate the dependency.
    - **Modify `crates/dosker-cli/src/adapter.rs`**
        - `default_engine` returns `qemu` when `qemu-engine` is enabled,
          otherwise `mock` when `mock-engine` is enabled, otherwise
          `qemu`.
        - `is_known` accepts `qemu` in every build and `mock` under
          `mock-engine`.
        - `build` for `qemu` calls
          `QemuAdapter::discover(Arc::new(system()))` and returns the
          adapter.
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - Add `Engines(EnginesArgs)` and `Doctor(DoctorArgs)`; both have
          no fields.
        - Dispatch both.
    - **Create `crates/dosker-cli/src/commands/engines.rs`**
        - `execute(context: &Context) -> Result<u8, CliError>` — call
          `QemuBinary::discover`; on success print
          `qemu <version> <path> <capability,names>`; on
          `EngineError::NotFound` print `qemu not found: <error>` and
          return 1.
        - The capability list is the QEMU adapter set, joined by commas.
    - **Create `crates/dosker-cli/src/commands/doctor.rs`**
        - `execute(context: &Context) -> Result<u8, CliError>` —
          create the data root when absent, write and remove
          `.doctor.tmp` to prove writability, discover QEMU, and print:
          `data root: <path>`, `data root writable: yes|no`,
          `qemu: <version> (<path>)` or `qemu: not found (<error>)`,
          `capabilities: <list>`, and
          `security: DOSker does not sandbox untrusted images; use an outer VM for untrusted input.`
        - A root creation or write failure prints `data root writable:
          no` and returns 1 instead of propagating the I/O error.
        - Return 0 when QEMU is found and the root is writable, 1
          otherwise.
    - **Modify `crates/dosker-cli/src/commands/mod.rs`**
        - Declare `engines` and `doctor`.
    - **Create `crates/dosker-cli/tests/cli_engines.rs`**
        - A `#[cfg(unix)]` helper writes an executable script that
          prints `QEMU emulator version 11.1.3` for `--version`. Tests
          that use the helper are `#[cfg(unix)]`; the missing-QEMU
          tests run on every host.
        - `engines_reports_the_qemu_version_and_path`.
        - `engines_lists_the_proven_capabilities`.
        - `engines_exits_one_when_qemu_is_missing` — `DOSKER_QEMU`
          names a missing path and `PATH` is empty.
        - `doctor_reports_the_data_root_and_qemu`.
        - `doctor_prints_the_security_statement`.
        - `doctor_exits_one_when_qemu_is_missing`.
        - `doctor_reports_an_unwritable_root` — point `DOSKER_ROOT` at a
          file.
    - **Verify:** `cargo test -p dosker-cli --features mock-engine,qemu-engine --test cli_engines`

- [x] **Task 13: QEMU integration harness and the runtime acceptance tests**
    - **Implements:** all in-scope requirements end to end; design §M1
      definition of done, §Testing Strategy.
    - **Modify `crates/dosker-test-support/src/fixtures.rs`**
        - Add `pub const INTEGRATION_ENV: &str = "DOSKER_INTEGRATION";`,
          `pub const QEMU_IMAGE_ENV: &str = "DOSKER_QEMU_IMAGE";`, and
          `pub const QEMU_ENV: &str = "DOSKER_QEMU";`.
        - Add `pub fn integration_enabled() -> bool` — true only for the
          value `1`.
        - Add `pub fn qemu_image() -> Result<PathBuf, String>` — read
          `DOSKER_QEMU_IMAGE`; a relative value is resolved against the
          workspace root (`CARGO_MANIFEST_DIR/../..`). The error names
          the variable and the prepared image.
        - Add `pub fn qemu_binary() -> String` — read `DOSKER_QEMU` or
          return `qemu-system-i386`.
    - **Modify `crates/dosker-cli/tests/common/mod.rs`**
        - Add `pub fn command_with_qemu(&self) -> Command` that starts
          from `base_command()` (no `DOSKER_MOCK_ENGINE`), sets
          `DOSKER_QEMU` from `qemu_binary()`, and keeps `DOSKER_ROOT`.
        - Add `pub fn run_with_qemu(&self, args: &[&str]) -> Output`.
        - Reuse the `spawn_captured` helper added in Task 9 for
          long-running commands such as `logs --follow`.
    - **Create `crates/dosker-cli/tests/qemu_runtime.rs`**
        - `#![cfg(unix)]` and `mod common;`.
        - `fn image() -> Option<PathBuf>` returns `None` unless
          `integration_enabled()` and `qemu_image()` both succeed.
        - `prepared_image_boots_execs_and_keeps_its_disk` — import,
          `run -d`, `exec ECHO HELLO` returns `HELLO` and 0, write
          `C:\DOSKER\STATE.TXT` with `exec`, `stop`, `start`, `exec TYPE
          C:\DOSKER\STATE.TXT` returns the persisted text, `ps` shows
          `running`. `C:\DOSKER` is guaranteed by the prepared image; do
          not assume `C:\APP` exists.
        - `attach_provides_a_working_line_shell` — use
          `spawn_with_stdin(command_with_qemu(), ...)` with
          `ECHO HELLO\nexit\n`; `attach` prints `HELLO`; the container
          remains running.
        - `restart_returns_a_stopped_container_to_running` — stop, then
          `restart`, then `exec` succeeds.
        - `cp_round_trips_a_small_file` — copy a host file to
          `C:\DOSKER\INPUT.TXT`, `exec TYPE C:\DOSKER\INPUT.TXT` matches,
          create `C:\DOSKER\OUT.TXT` with `exec`, copy it out, and
          compare the host bytes.
        - `booter_exec_returns_no_agent` — import `--booter`, `run -d`,
          `exec` exits 125 and prints `no agent`.
        - `booter_screenshot_writes_a_file` — `screenshot -o` writes a
          non-empty file.
        - `screenshot_writes_a_file` — a normal container writes a
          non-empty PPM or PNG file.
        - `vnc_display_listens_on_the_requested_port` — reserve a free
          loopback port in `5900..=65535`, start with `--display vnc
          --vnc-port <port>`, connect to `127.0.0.1:<port>`, and read
          the RFB greeting prefix `RFB `. The command translates the
          port to `port - 5900` for QEMU and QEMU listens on `5900 +
          display = port`. Clean up with `stop` and `rm`.
        - `engines_and_doctor_report_qemu` — both exit 0 and name the
          QEMU version.
        - `gdbstub_is_reachable` — `debug --gdb` prints
          `target remote <path>`; the socket accepts a connection.
        - `logs_show_engine_and_serial_output` — `logs --engine`
          contains the deterministic `engine ` command line written by
          the supervisor, and `logs --serial` contains the agent
          handshake/command bytes after a boot and an `exec`.
        - `image_import_does_not_modify_the_source` — compare the source
          bytes before and after import.
        - Every test cleans up with `stop` and `rm`; a failed cleanup
          kills the recorded supervisor and engine PIDs.
    - **Verify:** `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE="$PWD/agent/fixtures/dosker-agent.img" cargo test -p dosker-cli --features qemu-engine --test qemu_runtime -- --nocapture`

- [x] **Task 14: Continuous integration for the QEMU runtime**
    - **Implements:** FR-021 (Linux and macOS runtime gates), design
      §Testing Strategy (“The QEMU integration test runs on Linux in CI.
      On macOS arm64 it runs in a nightly job because TCG is slower.”).
    - **Modify `.github/workflows/ci.yml`**
        - The `test` job build step uses
          `--features dosker-cli/mock-engine,dosker-cli/qemu-engine`.
        - The clippy step uses the same feature list.
        - The unit and mock test step stays
          `--features dosker-cli/mock-engine`. The QEMU dependency is
          normal, so this step still compiles `engines` and `doctor`;
          `qemu-engine` only selects QEMU as the default engine in the
          runtime jobs.
        - Add a step `QEMU adapter unit suites`:
          `cargo test -p dosker-engine-qemu`.
        - Change the existing `agent-qemu` job-level `timeout-minutes:
          30` to `45`; do not add a second `timeout-minutes` key.
        - After `Self-check`, add a step `Rust QEMU runtime suite` with
          a step-level `timeout-minutes: 30`, `DOSKER_INTEGRATION=1`,
          `DOSKER_QEMU_IMAGE` set to
          `agent/fixtures/dosker-agent.img`, and `DOSKER_QEMU` set to
          `qemu-system-i386`, running
          `cargo test -p dosker-cli --features qemu-engine --test qemu_runtime -- --nocapture`.
    - **Create `.github/workflows/qemu-nightly.yml`**
        - Triggers: `schedule` at `0 3 * * *` and `workflow_dispatch`.
        - Job `runtime-macos` on `macos-14`, `timeout-minutes: 60`.
        - Checkout, Rust 1.98.1 with `rustfmt` and `clippy`,
          `brew install qemu`, cache with `Swatinem/rust-cache@v2`.
        - Read the repository variable `DOSKER_QEMU_IMAGE_URL`; a ZIP is
          downloaded and extracted, a raw image is downloaded directly,
          and a missing value fails with a message naming the variable.
        - Export `DOSKER_INTEGRATION=1`, `DOSKER_QEMU_IMAGE`,
          `DOSKER_QEMU=qemu-system-i386`, and run
          `cargo test -p dosker-cli --features qemu-engine --test qemu_runtime -- --nocapture`.
    - **Modify `README.md`**
        - Extend the CI section with the Linux QEMU runtime suite, the
          macOS nightly workflow, and the `DOSKER_QEMU_IMAGE_URL`
          repository variable.
    - **Verify:** locally run
      `cargo fmt --all -- --check && cargo build --workspace --all-targets --features dosker-cli/mock-engine,dosker-cli/qemu-engine && cargo clippy --workspace --all-targets --features dosker-cli/mock-engine,dosker-cli/qemu-engine -- -D warnings && cargo test --workspace --features dosker-cli/mock-engine && cargo test -p dosker-engine-qemu`, then run the Task 13 integration command against a prepared image. The Linux and macOS legs run in CI once the workflows are pushed.

## Requirements Coverage

| Requirement | Tasks |
| --- | --- |
| FR-005 — import a ready bootable DOS image (agent already present; the injection layer is deferred) | 1, 3, 9, 13 |
| FR-006 — a raw booter image is an opaque blob with `booter=true` | 1, 3, 4, 8, 9, 10, 13 |
| FR-008 — headless container under a per-container supervisor, no central daemon | 4, 5, 8, 9, 10, 11, 13 |
| FR-009 — execute a command through the agent and return output and status | 5, 7, 8, 10, 13 |
| FR-010 — line-oriented `attach` shell | 5, 7, 8, 10, 13 |
| FR-011 — pass through the native debugger when the capability exists | 6, 7, 8, 10, 13 |
| FR-018 — read-only image and private writable disk | 1, 2, 3, 4, 5, 8, 9, 13 |
| FR-019 — recover from a dead supervisor on the next CLI call | 4, 9, 10, 11, 13 |
| FR-021 — Linux x86_64 and macOS arm64 hosts (the Windows half is Slice 4) | 6, 12, 13, 14 |

FR-005’s “add the agent through a layer” half is not in this slice. The
image is prepared by M0 and already contains the agent; the general
injection layer arrives with the FreeDOS base builder. FR-021’s Windows
half belongs to the Windows runtime track.

## Done When Map

| Roadmap acceptance clause | Proved by |
| --- | --- |
| A prepared FreeDOS image imports and boots headless | Task 3 image tests; Task 7 `prepared_image_session_boots_execs_and_stops`; Task 13 `prepared_image_boots_execs_and_keeps_its_disk` |
| `exec` returns output and status | Task 5 supervisor tests; Task 7 session test; Task 13 |
| `attach` gives a working shell | Task 10 `cli_attach`; Task 13 `attach_provides_a_working_line_shell` |
| `stop` flushes and `start` returns to the same disk | Task 9 lifecycle tests; Task 13 persistence clause |
| `restart` returns a stopped container to running | Task 9 `restart_*`; Task 13 |
| `cp` moves a small file in and out of a running container | Task 5 cp tests; Task 10 `cli_cp`; Task 13 |
| `exec` on a booter image returns `no_agent` | Task 5 `exec_on_a_booter_returns_no_agent`; Task 10 `exec_on_a_booter_exits_125_with_no_agent`; Task 13 |
| `screenshot` writes a file | Task 7 session test; Task 13 |
| `--display vnc` serves the guest on a loopback port | Task 6 command tests; Task 13 `vnc_display_listens_on_the_requested_port` |
| `engines` and `doctor` report the QEMU version and capabilities | Task 12 `cli_engines`; Task 13 |
| The gdbstub is reachable on a socket | Task 7 session test; Task 13 `gdbstub_is_reachable` |
| `create`, `stop`, `restart`, `rm`, `run`, `ps`, `inspect`, and `logs` work | Tasks 9, 10, 11; Task 13 |
| The private disk is a reflink or full copy and the image is read-only | Task 2 copy tests; Task 3 read-only test; Task 9 `create_does_not_modify_the_image` |
| A killed stop or command timeout keeps the disk and marks it dirty | Task 8 `supervisor_marks_exited_dirty_when_stop_must_kill` and `supervisor_marks_exited_when_cmd_times_out` |

## Out Of Scope

- The image builder, the deterministic FreeDOS base builder, general
  agent injection into an arbitrary DOS image, OCI storage, layers, the
  Doskerfile parser, `RUN`, `RUN --host`, the lock file, and the build
  cache.
- Volumes, NAT, published ports, `-v`, `-p`, host bind mounts, `vvfat`,
  and development mode.
- The Windows host runtime, named-pipe control, Windows Job Objects, and
  Windows engine TCP endpoints. The QEMU crate compiles on Windows with
  a stub `start`; the Windows runtime is the next slice.
- DOSBox-X and Bochs adapters. Their capabilities stay unproven and are
  not declared.
- `run --rm`, `port`, `images`, `rmi`, `tag`, `save`, `load`,
  `system prune`, and `volume`.
- `cp` on a stopped container. The agent path requires a running
  container; larger files need a volume.
- Full-screen terminal pass-through on COM1. `attach` is a line shell
  built on agent `exec`.
- Character encoding transcoding. Command output is decoded with
  replacement for the JSON control protocol.
- PNG screenshots. `screenshot` writes the QMP PPM form.
- Audio, VGA mode selection, `--bind`, and game-specific tuning.

## Risks

- **FAT writer scope.** `fatfs` reads and writes a FAT12/16/32 volume
  behind a partition view. The slice needs only FAT16 and a small file.
  The MBR rule accepts the first type `0x06` or `0x0E` partition and
  uses its sector count for the `FatRegion` length; a superfloppy BPB
  uses the file length. An unusual layout fails `image import` with
  `NotBootable` before any VM boots.
- **Reduced machine profile.** The runner uses the Slice 2 conformance
  profile `-machine pc -cpu 486 -m 16`, no SB16, no NE2000, and
  `-nic none`. The prepared image boots under it; a guest that depends
  on sound or networking waits for the later data/network slice.
- **QEMU display differences.** `-display gtk` may be absent from a
  minimal Linux build. `--display window` is an interactive path and is
  not exercised in CI; `none` and `vnc` are.
- **macOS TCG speed.** A macOS arm64 boot is slow. The integration suite
  runs nightly with a 60 s readiness budget and a 65 s start wait. A
  timeout reports the last serial line and the engine log.
- **QMP screenshot format.** `screendump` gained the `format` argument
  in QEMU 7.1. The adapter omits it on older versions and accepts a PPM
  or PNG header in the integration test.
- **Serial log overhead.** Every agent byte is written to `serial.log`.
  The stop-and-wait protocol is slow enough that the write cost is
  small; the log is flushed per frame.
- **Unix socket path length.** Engine sockets reuse the 100-byte rule
  and the `/tmp/dosker-<euid>/` fallback. A host without a usable short
  temp directory fails the start with the OS error.
- **Read-only image on non-Unix hosts.** The `0444` mode is Unix-only.
  The Windows read-only attribute and the Windows runtime are Slice 4.
- **Booter readiness race.** A booter is declared ready after a 250 ms
  liveness probe. A booter that exits after that window becomes
  `exited` through the supervisor's `try_exit` loop.
- **Dirty disk after a kill.** The record keeps `dirty=true`; `start`
  prints a warning. A repair command is deferred.
- **`cp` size.** The 16 MiB bound comes from the agent transfer limit.
  It is enforced on upload before the agent call and on download after
  the agent read, so a large guest file is not written to the host.
  Larger files fail with `too_large` and a message that points to a
  volume.
- **Guest reboot and BUSY.** A second agent `HELLO` during a command
  becomes `guest_rebooted`; the supervisor marks the container `exited`
  with `dirty=true` and keeps the disk. An agent `BUSY` reply is surfaced
  as `busy` and the container stays running. Neither is retried
  automatically.
- **Snapshot of the agent protocol.** The QEMU session depends on the
  Slice 2 `AgentClient` and `Hello` fields exactly as implemented. A
  protocol change is a version change and is out of scope.
- **`attach` line semantics.** `cd` is handled in the CLI; the guest
  does not see a persistent shell. A command that changes the guest
  current directory is visible only through an explicit `cd` line.
- **QEMU version parsing and probe hang.** A distribution may print a
  suffix. The parser reads the first three numeric components after
  `version`; an unparsable line is a probe error naming the binary. The
  `--version` probe is bounded at 5 s and the child is killed, so a
  broken binary cannot hang `engines` or `doctor`.

## Deviations

- `FR-005` is covered only for a prepared image. The agent injection
  layer, arbitrary DOS image import, and the OCI store are later slices.
- `FR-021` is covered for Linux x86_64 and macOS arm64. The Windows half
  and engine endpoints over localhost TCP are the Windows runtime slice.
- The QEMU adapter declares only the capabilities this slice proves.
  `host_dir_live`, `nat`, and `audio` remain undeclared.
- `image import` writes a local `images/<name>/<tag>/` record, not an OCI
  layout. Slice 5 replaces the store without changing the CLI form.
- `cp` requires a running container and uses the agent. The
  stopped-container FAT read path is deferred.
- `screenshot` writes PPM. A PNG option is deferred.
- `attach` is a line shell over `exec`; COM1 raw terminal pass-through
  is deferred.
- `run --rm` is not implemented; `rm` is explicit.
- The supervisor records the rendered engine command for `inspect`; the
  data model does not yet have a command field.
- QEMU creates and owns the serial, QMP, monitor, and gdbstub Unix
  sockets with `server=on`; the supervisor connects as a client. The
  contract's host-created engine-socket wording is realized this way for
  QEMU. A host-created socket would need a different QEMU endpoint mode.
- COM1 has no host endpoint in this slice. The supervisor records
  `serial_agent`, `qmp`, `monitor`, and `gdb` only; `attach` uses the
  agent line shell, not a COM1 terminal.
- `start` and `restart` reject a `failed` container; `rm` is the only
  route out of `failed`.
- `image import` rejects a superfloppy disk image with
  `UnsupportedLayout`; a booter blob is a hard-disk image. Every disk is
  attached with `if=ide`, so the runner never selects `if=floppy`.
- The reduced machine is the prepared-image conformance profile:
  `-machine pc -cpu 486 -m 16`, no SB16, no NE2000, and `-nic none`.
  The Slice 2 fixture boots under this exact profile. SB16, NE2000, and
  NAT arrive with the data/network slice. The QEMU command test and the
  prepared-image integration boot are the evidence.
- `logs --follow` detects a stdout reader that closes after startup, but
  cannot detect a process started with stdout already closed (`>&-`).
  The standard library reopens descriptors 0, 1, and 2 on `/dev/null`
  before `main`, which is a valid descriptor to poll; that case is out of
  scope. No test asserts it, because Linux would reopen it differently
  from macOS.
- The QEMU `qmp`, `session`, and `serial_log` modules are declared in
  Task 7 rather than Task 6 because their files arrive with the session.
  `QemuPaths::remove_stale` returns `io::Result<()>` so callers propagate
  non-`NotFound` failures.
- QEMU's `-vnc` suffix is a display number, not a TCP port. The runner
  translates a requested port in `5900..=65535` to `port - 5900`; the
  Technical Context and command tests use that form.
- The boot-timeout diagnostic with no serial line appends the
  `--boot-timeout` retry hint after `last serial line: <none>`.
- `spawn_captured` returns its `TempDir` so callers own the capture
  directory.
- The integration image helper fails when integration is enabled but the
  prepared image is unavailable, instead of silently skipping the suite.
- The attach socket read timeout adds `IO_MARGIN` over the forwarded
  command budget, and an implicit attach on a booter is treated as a
  successful start without a shell.
- `engines` and `doctor` use `&mut Context` for the console and obtain
  the QEMU capability set from `dosker-engine-qemu`.
- A start timeout re-checks the record before killing: a record that
  became `running` is returned without killing, and the supervisor saves
  engine pids/sockets before readiness so a `starting` timeout can kill
  the engine group. `stop` on a `created` container exits 125.
- Agent sends set the transport write timeout from the remaining
  deadline; a write timeout is reported as `ProtoError::Timeout`.
- The supervisor tracks terminal replies with `terminal_pending` and a
  `shutdown_code`; QMP connect/quit use deadline-aware helpers.
- The integration helper fails fast when integration is enabled but the
  prepared image is missing.
