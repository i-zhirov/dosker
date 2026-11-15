# Image Store And Tree Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-14
- **Status**: Implemented
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Roadmap**: `.sdd/specs/2026-11-01-dosker/roadmap.md` — slice: `image-store-and-tree`
- **Depends on**: `runtime-foundation`
- **Manual prerequisites**: none
- **Goal**: Deliver the canonical DOS tree, content-addressed layers, the local OCI image store, the build-cache record, OCI-backed image import, and the `images`, `rmi`, and `tag` commands, while keeping `create` and `run` working from an imported disk.

## Constraints

- Supported hosts: Linux x86_64, macOS arm64, Windows x86_64.
- OCI layer media type: `application/vnd.dosker.layer.v1.tar`.
- OCI image config media type: `application/vnd.dosker.image.config.v1+json`.
- OCI booter media type: `application/vnd.dosker.booter.v1.raw`.
- Outer manifest media type: `application/vnd.oci.image.manifest.v1+json`.
- Outer index media type: `application/vnd.oci.image.index.v1+json`.
- OCI layout version: `1.0.0`.
- Tag annotation: `org.opencontainers.image.ref.name`.
- Raw-disk annotation: `org.dosker.rawdisk.digest`.
- DOS name: 1 to 8 characters, then an optional `.` and 1 to 3 characters.
- Allowed DOS characters: `A-Z`, `0-9`, `$ # @ ! % & ' ( ) - { } ~ ^ _`, and the backtick character.
- The first DOS character cannot be a space or a period.
- DOS tree paths use uppercase components and `/` separators.
- The tree always represents `C:\`.
- Layer `parent` equals the previous layer digest, or is null for the first layer.
- Canonical tar order: `dosker-layer.json` first, then `files/<path>` in path order.
- Long layer paths use tar long-name formats: the writer emits a GNU `L` extension for a member path over 100 bytes; the reader accepts GNU `L` and PAX `x`/`g` extensions through `tar::Archive`. The canonical writer output uses GNU long-name records.
- Tar member mtime: the stored tree timestamp; the sidecar uses `1980-01-01T00:00:00Z`.
- Tar header: uid 0, gid 0, empty user and group names, file mode 0644, directory mode 0755.
- Fixed timestamp for `COPY` and `RUN --host`: `1980-01-01T00:00:00Z` (`315532800` seconds).
- Config digest: SHA-256 of canonical JSON with sorted keys.
- Effective boot metadata: the last non-null layer `boot`, or the config `boot` when no layer overrides it.
- A superfloppy disk image is accepted. Its `machine.disk_interface` is `floppy`; an MBR-partitioned image uses `ide`.
- The image disk interface flows from the image config through the container record and engine spec to the engine command: `ide` selects `if=ide`, `floppy` selects `if=floppy`.
- `rmi <name:tag>` is an untag operation and never deletes a manifest by tag. `rmi <sha256:...>` removes an untagged manifest and its unreferenced blobs; a digest that still has tags returns `ImageInUse`. The CLI help and image documentation state this explicitly.
- Import, tag, and remove hold `images/.lock` exclusively for the index read-modify-write; reads rely on the atomically replaced `index.json` and do not lock.
- Layer and tree timestamps are truncated to whole seconds before hashing or writing.
- A whiteout of a missing path is ignored.
- A file-versus-directory conflict is an error.
- A non-booter image is read-only. `create` writes only to the private container disk.
- Blob files are content-addressed by SHA-256 and mode `0444` on Unix.
- No central daemon. Images live under the resolved data root in `images/`.
- No transient references in code: do not put plan, design, or roadmap paths, task numbers, requirement IDs, or slice names into identifiers, test names, strings, or comments.

## Compatibility Ruling

A non-booter imported DOS image stores both representations:

- The canonical tree layer or layers, per the design's image model.
- The original raw disk as a read-only OCI blob for runtime compatibility.

The raw disk is referenced from the standard OCI manifest by the annotation
`org.dosker.rawdisk.digest`. It is not a layer, it is not in `layers`, and it
is not in `rootfs.diff_ids`. The manifest and config remain standard OCI
objects.

`dosker image import` reads the FAT tree and the boot metadata, creates the
canonical tree layer, stores the raw disk blob, and writes the annotation.
`create` resolves the annotation and copies the raw disk blob to the private
writable disk. The stored image stays read-only.

`tag` keeps the annotated raw disk blob. `rmi` by tag removes only the tag
and leaves the manifest addressable by digest. `rmi` by digest deletes the
manifest, config, tree blobs, and raw disk blob only when no remaining tag
references them. A tag reference and a digest reference resolve the same
image.

The raw-disk bridge is temporary. Its removal belongs to the later
tree-to-FAT projection work. This slice does not implement that removal.

This is the only approved deviation from the design's non-booter image shape.
Do not add another one.

## Technical Context

**Language and toolchain.** Rust 1.98.1, edition 2024, one virtual
workspace. The image crate remains synchronous and uses the standard library
for file I/O.

**Crate changes.**

| Crate | Change |
| --- | --- |
| `dosker-image` | Adds `canonical`, `tree`, `boot`, `config`, `manifest`, `layer`, `oci`, and `cache` modules. Adds `tar = "0.4"`, `base64 = "0.22"`, and `fs2 = "0.4"` dependencies. |
| `dosker-cli` | Adds `images`, `rmi`, and `tag` commands. `Context.images` becomes `OciStore`. `create` and `run` resolve tags and digests and carry the image disk interface into the container record. |
| `dosker-engine` | Adds `EngineSpec.disk_interface`. |
| `dosker-engine-qemu` | Emits `if=ide` or `if=floppy` from `EngineSpec.disk_interface`. |
| `dosker-runtime` | Adds `ContainerRecord.disk_interface` and passes it into `EngineSpec`. |
| workspace | Adds `tar = "0.4"`, `base64 = "0.22"`, and `fs2 = "0.4"` to the workspace dependency table. |

**OCI layout under the data root.**

```text
<root>/
  images/
    oci-layout
    index.json
    blobs/sha256/<hex>
  cache/<hex>.json
  containers/<id>/...
```

The old `images/<name>/<tag>/` directories are not read by the new store. The
design states that there is no migration for this new project. The cleanup
task removes the old store code and its tests.

**OCI object shape.**

- The index has `schemaVersion: 2`, the OCI index media type, and one
  descriptor per tag. Each descriptor carries
  `org.opencontainers.image.ref.name`.
- The manifest has `schemaVersion: 2`, the OCI manifest media type, a config
  descriptor, a layer list, and an optional annotation map.
- The config descriptor uses the DOSker image config media type.
- A tree layer uses the DOSker layer media type.
- A booter image has exactly one layer with the DOSker booter media type and
  `booter=true` in the config.
- A non-booter imported image has tree layers and the raw-disk annotation.
  The raw-disk digest is not a manifest layer.

**Pinned values.**

| Value | Setting |
| --- | --- |
| Canonical JSON | Sorted keys, no whitespace, UTF-8, no trailing newline |
| Config digest | `sha256:<hex>` over canonical config JSON |
| Layer digest | `sha256:<hex>` over canonical tar bytes |
| Manifest digest | `sha256:<hex>` over canonical manifest JSON |
| Raw disk digest | `sha256:<hex>` over the raw disk bytes |
| Cache key | 64 lowercase hex characters, no prefix |
| Partitioned disk interface | `ide` |
| Superfloppy disk interface | `floppy` |
| Blob mode | `0444` on Unix |
| Image directory mode | `0700` on Unix |
| Default machine CPU | `486dx2` |
| Default memory | `16` MiB |
| Default video | `vga` |
| Default sound | `sb16` |
| Default network | `ne2000` |
| Default serial ports | `2` |
| Default control port | `2` |
| Default disk interface | `ide` |
| Default baud | `115200` |
| CLI exit: success | 0 |
| CLI exit: image or runtime error | 1 |
| CLI exit: usage or invalid reference | 2 |
| `images` header | `REFERENCE DIGEST SIZE CREATED` |
| `rmi` output | `removed <ref>` or `untagged <ref>` |
| `tag` output | `tagged <target>` |

**Deliberate subset.** The build engine, Doskerfile parser, `COPY`, `RUN`,
`RUN --host`, agent injection, and cache invalidation are not built here. The
cache module stores and reads records but no build path calls it. The
tree-to-FAT projection is not built here. The raw-disk bridge keeps the
runtime path working until that projection lands.

**In-scope requirements.** FR-002 (content-addressed layers and tree
application), FR-003 (cache record keyed by parent tree digest, config digest,
normalized instruction, and input hashes; record only), FR-012 (local OCI
layout and DOSker media types), FR-017 (8.3 validation and traversal and
symlink rejection), and FR-018 (read-only image and private writable disk).

## Tasks

- [x] **Task 1: DOS 8.3 path model**
    - **Implements:** FR-017; design §Assumptions And Constraints, §Error Handling And Edge Cases; data-model §Validation Rules.
    - **Create `crates/dosker-image/tests/path.rs`**
        - `normalize_uppercases_a_lowercase_name` — `app/foo.txt` becomes `APP/FOO.TXT`.
        - `normalize_accepts_each_allowed_symbol` — every allowed symbol is accepted.
        - `normalize_rejects_a_name_longer_than_eight` — a nine-character base fails.
        - `normalize_rejects_an_extension_longer_than_three` — a four-character extension fails.
        - `normalize_rejects_a_leading_period_or_space` — `.FOO` and ` FOO` fail.
        - `normalize_rejects_a_second_dot` — `A.B.C` fails.
        - `normalize_rejects_an_empty_component` — `A//B` fails.
        - `normalize_rejects_a_dot_dot_component` — `A/../B` fails.
        - `normalize_accepts_a_leading_separator` — `/A` and `\A` become `A`, because the contract allows `["C:"] ("\" | "/")? component ...`.
        - `normalize_rejects_a_non_c_drive` — `D:\APP` fails.
        - `normalize_accepts_a_c_drive_and_either_separator` — `C:\app\foo.txt` and `c:/app/foo.txt` become the same path.
        - `normalize_accepts_the_drive_root` — `C:\`, `C:/`, and `C:` become the empty root path, and `to_dos_string` returns `C:\`.
        - `tree_member_requires_the_files_prefix` — `APP/FOO.TXT` fails; `files/APP/FOO.TXT` passes.
        - `tree_member_rejects_traversal_and_absolute_paths` — `files/../APP` and `files//APP` fail.
        - `dos_path_round_trips_through_serde` — the JSON form is the canonical `/` string.
    - **Create `crates/dosker-image/src/path.rs`**
        - `pub const DOS_NAME_MAX: usize = 8;`
        - `pub const DOS_EXTENSION_MAX: usize = 3;`
        - `pub const DOS_DRIVE: char = 'C';`
        - `DosName(String)` — newtype with `Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize` and `#[serde(try_from = "String", into = "String")]`.
        - `DosName::normalize(value: &str) -> Result<Self, ImageError>` — uppercases ASCII, then validates the base and optional extension.
        - `DosName::as_str(&self) -> &str`; `Display`; `TryFrom<String>`; `From<DosName> for String`.
        - `DosPath(String)` — same derives and serde attributes; stores uppercase `/`-separated components without a drive.
        - `DosPath::parse(value: &str) -> Result<Self, ImageError>` — accepts an optional `C:` drive, trims one leading separator whether or not a drive is present, treats the drive root as the empty path, and calls `parse_relative` for the rest.
        - `DosPath::parse_relative(value: &str) -> Result<Self, ImageError>` — splits on `/` or `\`, normalizes each component, rejects a leading separator, empty, `.` and `..` components, and joins with `/`. An empty input is the root path.
        - `DosPath::as_str(&self) -> &str`; `components(&self) -> impl Iterator<Item = &str>`; `parent(&self) -> Option<DosPath>`; `file_name(&self) -> Option<&str>`; `join(&self, name: &DosName) -> DosPath`.
        - `DosPath::to_dos_string(&self) -> String` — returns `C:\APP\FOO.TXT`.
        - `DosPath::to_tar_member(&self) -> String` — returns `files/APP/FOO.TXT`.
        - `pub fn normalize_tree_member(value: &str) -> Result<DosPath, ImageError>` — requires the `files/` prefix, trims one trailing `/`, rejects an empty member path and traversal, and returns a tree path.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Declare `pub mod path;` and re-export `DosName`, `DosPath`, `normalize_tree_member`.
    - **Modify `crates/dosker-image/src/error.rs`**
        - Add `InvalidDosPath { path: String, reason: String }` with message `invalid DOS path: {path}: {reason}`.
    - **Verify:** `cargo test -p dosker-image --test path`

- [x] **Task 2: Canonical JSON, tree model, and tree digest**
    - **Implements:** FR-002; design §Data Model, §Build And Runtime Semantics; data-model §Layer, §Tree, §Validation Rules.
    - **Create `crates/dosker-image/tests/tree.rs`**
        - `apply_delta_adds_files_and_directories` — a file and a directory appear at their normalized paths.
        - `apply_delta_creates_missing_parent_directories` — a file under a missing parent creates the parent with default attributes and the file timestamp.
        - `apply_delta_rejects_an_empty_path` — a root file or directory returns `LayerConflict`.
        - `apply_delta_replaces_content_and_attributes` — a later entry replaces the bytes and attributes.
        - `whiteout_removes_a_file` — the path is gone.
        - `whiteout_removes_a_directory_subtree` — the directory and its children are gone.
        - `whiteout_of_a_missing_path_is_ignored` — no error and no change.
        - `file_and_directory_conflict_is_an_error` — a file at a directory path, a directory at a file path, and a child under a file each return `LayerConflict`.
        - `lowercase_paths_are_normalized_before_application` — `app/foo.txt` lands at `APP/FOO.TXT`.
        - `tree_digest_is_stable_across_insertion_order` — two trees built in different orders have the same digest.
        - `tree_digest_changes_when_file_content_changes` — one byte changes the digest.
        - `tree_digest_changes_when_attributes_change` — the read-only bit changes the digest.
        - `tree_digest_truncates_subsecond_timestamps` — two trees that differ only below one second have the same digest.
        - `canonical_json_sorts_keys_and_has_no_whitespace` — nested objects are sorted and compact.
    - **Create `crates/dosker-image/src/canonical.rs`**
        - `pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ImageError>` — recursively sorts object keys and writes compact UTF-8 JSON.
        - `pub fn sha256_digest(bytes: &[u8]) -> String` — returns `sha256:<lowercase hex>`.
        - `pub fn canonical_digest<T: Serialize>(value: &T) -> Result<String, ImageError>` — canonical JSON then SHA-256.
        - `pub(crate) fn hex(bytes: &[u8]) -> String` — lowercase hex.
    - **Create `crates/dosker-image/src/tree.rs`**
        - `pub struct Attributes { pub read_only: bool, pub hidden: bool, pub system: bool, pub archive: bool }` — `Default`, serde.
        - `pub enum TreeKind { File, Dir }` — serde snake_case.
        - `pub struct TreeFile { pub path: DosPath, pub attributes: Attributes, pub timestamp: DateTime<Utc>, pub bytes: Vec<u8> }`.
        - `pub struct TreeDir { pub path: DosPath, pub attributes: Attributes, pub timestamp: DateTime<Utc> }`.
        - `pub enum TreeNode { File(TreeFile), Dir(TreeDir) }` — `kind(&self) -> TreeKind`, `attributes(&self) -> &Attributes`, `timestamp(&self) -> DateTime<Utc>`.
        - `pub struct LayerDelta { pub whiteouts: Vec<DosPath>, pub files: Vec<TreeFile>, pub dirs: Vec<TreeDir> }` — `LayerDelta::new() -> Self`, `whiteout(&mut self, path: DosPath)`, `add_file(&mut self, file: TreeFile)`, `add_dir(&mut self, dir: TreeDir)`.
        - `pub struct Tree { entries: BTreeMap<DosPath, TreeNode> }` — `Tree::new() -> Self`, `Default`, `get(&self, path: &DosPath) -> Option<&TreeNode>`, `iter(&self) -> impl Iterator<Item = (&DosPath, &TreeNode)>`, `files(&self) -> impl Iterator<Item = &TreeFile>`, `dirs(&self) -> impl Iterator<Item = &TreeDir>`, `is_empty(&self) -> bool`, `to_delta(&self) -> LayerDelta`, `digest(&self) -> Result<String, ImageError>`.
        - `Tree::apply_delta(&mut self, delta: &LayerDelta) -> Result<(), ImageError>` — whiteouts first; missing whiteout ignored; then directories and files; an empty path returns `LayerConflict`; an entry creates missing parent directories with default attributes and the entry timestamp; a file-versus-directory conflict returns `LayerConflict`; a child under a file returns `LayerConflict`.
        - `struct TreeDigestEntry { path: String, kind: TreeKind, attributes: Attributes, timestamp: DateTime<Utc>, size: u64, content: Option<String> }` — private canonical JSON shape for `Tree::digest`; `content` is the SHA-256 of file bytes and `None` for directories.
        - `Tree::digest(&self) -> Result<String, ImageError>` — canonical JSON of sorted `TreeDigestEntry` values then SHA-256. Each entry holds `path`, `kind`, `attributes`, `timestamp`, `size`, and a `content` SHA-256 for files. Timestamps are truncated to whole seconds before hashing.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Declare `pub mod canonical; pub mod tree;` and re-export `Attributes`, `LayerDelta`, `Tree`, `TreeDir`, `TreeFile`, `TreeKind`, `TreeNode`, `canonical_json`, `sha256_digest`.
    - **Modify `crates/dosker-image/src/error.rs`**
        - Add `LayerConflict { path: String, reason: String }` with message `layer path conflict at {path}: {reason}`.
    - **Verify:** `cargo test -p dosker-image --test tree`

- [x] **Task 3: Boot metadata model**
    - **Implements:** FR-012; design §API Contracts, §Data Model; data-model §BootMetadata, §Validation Rules.
    - **Create `crates/dosker-image/tests/boot.rs`**
        - `opaque_boot_has_the_opaque_layout` — `BootMetadata::opaque` sets `layout=Opaque`, the disk size, and no tree fields.
        - `validate_requires_partition_fields_for_mbr_partitioned` — a missing start or sector count fails.
        - `validate_allows_the_opaque_layout_without_partition_fields` — an opaque layout with no partition fields passes `BootMetadata::validate`; the booter/opaque rule belongs to `ImageConfig::validate`.
        - `blob_ref_round_trips_bytes_inline` — SHA-256 and base64 bytes round-trip.
        - `blob_ref_verify_rejects_different_bytes` — a different byte array fails.
        - `boot_metadata_round_trips_through_json` — every field survives serde.
        - `volume_label_is_limited_to_eleven_characters` — a longer label fails validation.
    - **Create `crates/dosker-image/src/boot.rs`**
        - `pub enum BootLayout { MbrPartitioned, Superfloppy, Opaque }` — serde snake_case.
        - `pub enum FatType { Fat12, Fat16, Fat32 }` — serde snake_case.
        - `pub struct Geometry { pub cylinders: u32, pub heads: u32, pub sectors: u32, pub sector_size: u32 }`.
        - `pub struct BlobRef { pub sha256: String, pub bytes_inline: Option<String> }` — `from_bytes(bytes: &[u8]) -> Self`, `verify(&self, bytes: &[u8]) -> bool`.
        - `pub struct BootMetadata { pub layout: BootLayout, pub disk_size: u64, pub geometry: Geometry, pub partition_start: Option<u32>, pub partition_sectors: Option<u32>, pub mbr: Option<BlobRef>, pub boot_sector: Option<BlobRef>, pub fat_type: Option<FatType>, pub volume_label: Option<String>, pub media_descriptor: Option<u8> }`.
        - `BootMetadata::opaque(disk_size: u64) -> Self` — `layout=Opaque`, zero geometry, no partition or boot fields.
        - `BootMetadata::validate(&self) -> Result<(), ImageError>` — the rules above.
    - **Modify `crates/dosker-image/Cargo.toml` and `Cargo.toml`**
        - Add `base64 = "0.22"` to the workspace table and to `dosker-image`.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Declare `pub mod boot;` and re-export `BlobRef`, `BootLayout`, `BootMetadata`, `FatType`, `Geometry`.
    - **Modify `crates/dosker-image/src/error.rs`**
        - Add `InvalidBootMetadata { reason: String }` with message `invalid boot metadata: {reason}`.
    - **Verify:** `cargo test -p dosker-image --test boot`

- [x] **Task 4: Image config and canonical config digest**
    - **Implements:** FR-012, FR-003; design §Data Model, §API Contracts; data-model §Image Config, §MachineProfile, §Validation Rules.
    - **Create `crates/dosker-image/tests/config.rs`**
        - `default_machine_profile_matches_the_pinned_values` — every default in Technical Context.
        - `machine_profile_rejects_an_out_of_range_value` — memory 0, zero serial ports, and an unknown disk interface fail.
        - `image_config_round_trips_through_json` — every field survives serde.
        - `config_digest_is_stable_across_independent_construction` — two equal configs produce the same digest.
        - `config_digest_changes_when_env_changes` — one environment value changes the digest.
        - `config_digest_changes_when_workdir_changes` — one workdir change changes the digest.
        - `config_validate_rejects_a_booter_with_a_tree_field` — a booter with a non-empty environment, workdir, command, exposed port, or volume fails.
        - `config_validate_rejects_a_booter_with_a_non_opaque_layout` — the booter/opaque rule lives here, not in `BootMetadata::validate`.
        - `config_validate_rejects_a_non_booter_with_an_opaque_layout` — a tree image cannot use the opaque layout.
        - `config_validate_rejects_a_disk_interface_that_does_not_match_the_layout` — `mbr_partitioned` requires `ide`; `superfloppy` requires `floppy`.
        - `port_declaration_rejects_port_zero` — port 0 fails; port 65535 passes.
    - **Create `crates/dosker-image/src/config.rs`**
        - `pub const IMAGE_CONFIG_SCHEMA_VERSION: u32 = 1;`
        - `pub enum DosFlavour { Freedos, Msdos, Drdos, Other, None }` — serde snake_case.
        - `pub struct MachineProfile { pub cpu: String, pub memory_mb: u16, pub video: String, pub sound: String, pub net: String, pub serial_ports: u8, pub control_port: u8, pub disk_interface: String, pub baud: u32 }` — `Default` with the pinned values; `validate(&self) -> Result<(), ImageError>`.
        - `pub struct HistoryEntry { pub created_by: String, pub created_at: DateTime<Utc>, pub empty_layer: bool, pub comment: Option<String> }`.
        - `pub enum PortProtocol { Tcp }` — serde lowercase.
        - `pub struct PortDeclaration { pub port: u16, pub protocol: PortProtocol }`.
        - `pub struct VolumeDeclaration { pub drive: String }`.
        - `pub struct ImageConfig { pub schema_version: u32, pub dos_flavour: DosFlavour, pub booter: bool, pub boot: BootMetadata, pub machine: MachineProfile, pub env: BTreeMap<String, String>, pub workdir: String, pub cmd: Option<String>, pub exposed_ports: Vec<PortDeclaration>, pub volumes: Vec<VolumeDeclaration>, pub engine_pin: Option<String>, pub requires: Vec<String>, pub agent_version: Option<String>, pub created_by: String, pub history: Vec<HistoryEntry> }`.
        - `ImageConfig::validate(&self) -> Result<(), ImageError>` — schema version, booter rules (empty environment, empty `workdir`, no command, no ports, no volumes, opaque boot layout), non-booter boot metadata (not the opaque layout) calls `boot.validate()`, non-booter `workdir` parses as a `DosPath` with the `C:` drive, `machine.disk_interface` matches the boot layout (`ide` for `mbr_partitioned`, `floppy` for `superfloppy`), port range, volume drive shape. History may be non-empty for both kinds. This method owns the booter/opaque rule; `BootMetadata::validate` checks only the metadata fields.
        - `ImageConfig::digest(&self) -> Result<String, ImageError>` — canonical config JSON then SHA-256.
        - `pub fn config_digest(config: &ImageConfig) -> Result<String, ImageError>` — calls `ImageConfig::digest`.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Declare `pub mod config;` and re-export `DosFlavour`, `HistoryEntry`, `ImageConfig`, `MachineProfile`, `PortDeclaration`, `PortProtocol`, `VolumeDeclaration`, `config_digest`.
    - **Modify `crates/dosker-image/src/error.rs`**
        - Add `InvalidImageConfig { reason: String }` with message `invalid image config: {reason}`.
    - **Verify:** `cargo test -p dosker-image --test config`

- [x] **Task 5: OCI manifest and index model**
    - **Implements:** FR-012; design §API Contracts, §Data Model; data-model §Image, §Layer, §Validation Rules.
    - **Create `crates/dosker-image/tests/manifest.rs`**
        - `media_type_constants_match_the_contract` — the five media types and the two annotations.
        - `manifest_serializes_standard_oci_fields` — `schemaVersion`, `mediaType`, `config`, `layers`, and `annotations`.
        - `manifest_omits_absent_annotations` — a manifest and its config/layer descriptors omit the `annotations` key instead of writing `null`.
        - `descriptor_omits_absent_annotations` — a descriptor with no annotations has no `annotations` key.
        - `index_serializes_standard_oci_fields` — `schemaVersion`, `mediaType`, and `manifests`.
        - `raw_disk_annotation_round_trips` — the annotation key and digest survive serde.
        - `manifest_validate_rejects_a_tree_image_without_layers` — a non-booter with no layers fails.
        - `manifest_validate_rejects_a_booter_layer_on_a_tree_image` — a non-booter with a booter layer fails.
        - `manifest_validate_rejects_a_tree_layer_on_a_booter` — a booter with a tree layer fails.
        - `manifest_validate_rejects_a_booter_with_two_layers` — a booter with two layers fails.
        - `index_finds_a_tag_and_a_digest` — `find_ref` and `find_digest` return the same descriptor.
        - `index_remove_ref_keeps_other_tags` — one tag is removed and the other remains.
        - `oci_layout_has_version_1_0_0` — the serialized layout file.
    - **Create `crates/dosker-image/src/manifest.rs`**
        - `pub const MEDIA_TYPE_LAYER: &str = "application/vnd.dosker.layer.v1.tar";`
        - `pub const MEDIA_TYPE_CONFIG: &str = "application/vnd.dosker.image.config.v1+json";`
        - `pub const MEDIA_TYPE_BOOTER: &str = "application/vnd.dosker.booter.v1.raw";`
        - `pub const MEDIA_TYPE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";`
        - `pub const MEDIA_TYPE_INDEX: &str = "application/vnd.oci.image.index.v1+json";`
        - `pub const ANNOTATION_REF_NAME: &str = "org.opencontainers.image.ref.name";`
        - `pub const ANNOTATION_RAW_DISK_DIGEST: &str = "org.dosker.rawdisk.digest";`
        - `pub const OCI_LAYOUT_VERSION: &str = "1.0.0";`
        - `pub struct Descriptor { pub media_type: String, pub digest: String, pub size: u64, #[serde(default, skip_serializing_if = "Option::is_none")] pub annotations: Option<BTreeMap<String, String>> }` — `#[serde(rename_all = "camelCase")]`. An absent annotation map is omitted from JSON, never serialized as `null`.
        - `pub struct Manifest { pub schema_version: u32, pub media_type: String, pub config: Descriptor, pub layers: Vec<Descriptor>, #[serde(default, skip_serializing_if = "Option::is_none")] pub annotations: Option<BTreeMap<String, String>> }` — `#[serde(rename_all = "camelCase")]`; `Manifest::new(config: Descriptor, layers: Vec<Descriptor>, annotations: Option<BTreeMap<String, String>>) -> Self`, `raw_disk_digest(&self) -> Option<&str>`, `validate(&self, booter: bool) -> Result<(), ImageError>`. An absent annotation map is omitted from JSON, never serialized as `null`.
        - `pub struct Index { pub schema_version: u32, pub media_type: String, pub manifests: Vec<Descriptor> }` — `#[serde(rename_all = "camelCase")]`; `Index::new() -> Self`, `add(&mut self, descriptor: Descriptor)`, `find_ref(&self, name: &str) -> Option<&Descriptor>`, `find_digest(&self, digest: &str) -> Vec<&Descriptor>`, `remove_ref(&mut self, name: &str) -> Option<Descriptor>`, `remove_digest(&mut self, digest: &str) -> Vec<Descriptor>`, `validate(&self) -> Result<(), ImageError>`.
        - `pub struct OciLayout { pub image_layout_version: String }` — `#[serde(rename = "imageLayoutVersion")]`; `OciLayout::new() -> Self`, `Default`.
        - `Manifest::validate(&self, booter: bool)` checks schema version 2, the manifest media type, the config media type, the layer media types, and the raw-disk annotation shape. A non-booter manifest has at least one tree layer and may carry the raw-disk annotation. A booter manifest has exactly one booter layer and must not carry the annotation.
        - `Index::validate` checks schema version 2, the index media type, and every descriptor.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Declare `pub mod manifest;` and re-export the constants and the three types.
    - **Modify `crates/dosker-image/src/error.rs`**
        - Add `UnsupportedMediaType(String)` and `InvalidManifest { reason: String }`.
    - **Verify:** `cargo test -p dosker-image --test manifest`

- [x] **Task 6: Canonical layer tar and parent rule**
    - **Implements:** FR-002, FR-003; design §Data Model, §Build And Runtime Semantics; data-model §Layer, §Tree, §Validation Rules.
    - **Create `crates/dosker-image/tests/layer.rs`**
        - `layer_tar_writes_the_sidecar_first` — the first member is `dosker-layer.json`.
        - `layer_tar_writes_members_in_path_order` — `files/APP` precedes `files/APP/FOO.TXT`.
        - `layer_tar_uses_the_stored_timestamps` — the tar mtime equals the whole-second tree timestamp.
        - `layer_tar_truncates_subsecond_timestamps` — a source timestamp with nanoseconds writes the same seconds to the tar header and sidecar.
        - `layer_tar_uses_the_fixed_sidecar_timestamp` — the sidecar mtime is `315532800`.
        - `layer_digest_is_stable_across_independent_writes` — the same delta written twice has one digest.
        - `layer_digest_changes_when_a_file_changes` — one byte changes the digest.
        - `layer_round_trips_through_tar` — metadata, paths, bytes, and whiteouts survive.
        - `layer_parent_must_match_the_previous_digest` — a wrong parent and a missing parent each fail.
        - `layer_reader_rejects_a_symlink_member` — a symlink tar member fails.
        - `layer_reader_rejects_a_traversal_member` — `files/../APP` fails.
        - `layer_reader_rejects_a_missing_sidecar` — a tar without the sidecar fails.
        - `layer_reader_rejects_a_wrong_schema_version` — a sidecar with a schema version other than 1 fails.
        - `layer_reader_rejects_a_size_mismatch` — the sidecar size must match the member.
        - `layer_writer_uses_a_gnu_long_name_for_a_long_path` — a member path over 100 bytes is written with a GNU `L` extension and reads back with the long path intact.
        - `layer_reader_accepts_a_gnu_long_path` — an archive with a GNU `L` extension resolves to the expected long `files/<path>` member.
        - `layer_reader_accepts_a_pax_long_path` — an archive with a PAX `x` `path` record resolves to the expected long `files/<path>` member.
        - `layer_long_path_round_trips` — write then read preserves the long path, bytes, attributes, and sidecar entry.
        - `layer_long_path_digest_is_stable` — two independent writes of the same long-path layer have the same digest.
        - `layer_apply_to_applies_whiteouts_before_entries` — a whiteout and a replacement in the same layer produce the replacement.
        - `effective_boot_uses_the_last_non_null_layer_override` — a later layer wins; the config is the fallback.
    - **Create `crates/dosker-image/src/layer.rs`**
        - `pub const LAYER_SCHEMA_VERSION: u32 = 1;`
        - `pub const LAYER_META_FILE: &str = "dosker-layer.json";`
        - `pub const LAYER_FILES_PREFIX: &str = "files/";`
        - `pub const FIXED_TIMESTAMP_SECONDS: i64 = 315_532_800;`
        - `pub fn fixed_timestamp() -> DateTime<Utc>` — `1980-01-01T00:00:00Z`.
        - `pub struct LayerEntry { pub path: DosPath, pub kind: TreeKind, pub attributes: Attributes, pub timestamp: DateTime<Utc>, pub size: u64, pub content: Option<String> }`.
        - `pub struct LayerMeta { pub schema_version: u32, pub parent: Option<String>, pub created_by: String, pub whiteouts: Vec<DosPath>, pub entries: Vec<LayerEntry>, pub boot: Option<BootMetadata> }`.
        - `pub struct Layer { pub meta: LayerMeta, files: BTreeMap<DosPath, Vec<u8>> }`.
        - `Layer::from_delta(delta: &LayerDelta, created_by: &str, parent: Option<String>, boot: Option<BootMetadata>) -> Result<Self, ImageError>` — sorts entries by path and whiteouts by path, truncates every timestamp to whole seconds, and builds the sidecar metadata.
        - `Layer::to_delta(&self) -> LayerDelta` — whiteouts, directories, and files from the stored metadata and bytes.
        - `Layer::apply_to(&self, tree: &mut Tree) -> Result<(), ImageError>` — whiteouts first, then entries.
        - `Layer::write_tar(&self) -> Result<Vec<u8>, ImageError>` — computes every member path (`dosker-layer.json` and `files/<dos_path>`, with a trailing `/` for directories). Paths that fit in the 100-byte ustar name field use that field; longer paths use the `tar` crate's GNU long-name extension (`L`). It sets every header field on the real member explicitly: path, mode, uid, gid, user name, group name, mtime, size, entry type, and checksum. Directory member paths end with `/`. The generated GNU extension header is deterministic.
        - `Layer::read_tar(bytes: &[u8]) -> Result<Self, ImageError>` — reads through `tar::Archive`, which resolves GNU long-name (`L`) and PAX (`x`/`g`) extension records and applies the extended path to the following member. It accepts only the sidecar and regular-file or directory members under `files/`; it rejects an extension record without a following member, a duplicate member, a member outside `files/`, an empty entry path, a missing sidecar, a sidecar schema version other than `LAYER_SCHEMA_VERSION`, a member kind that differs from the sidecar, a size mismatch, and a sidecar that names a different member set. It sorts the sidecar entries by path after validation.
        - `Layer::digest(&self) -> Result<String, ImageError>` — SHA-256 of the canonical tar bytes.
        - `Layer::validate_parent(&self, previous: Option<&str>) -> Result<(), ImageError>` — the parent must equal the previous digest, or be null for the first layer.
        - `Layer::meta_json(&self) -> Result<Vec<u8>, ImageError>` — canonical JSON of `LayerMeta`.
        - `pub fn effective_boot(config: &ImageConfig, layers: &[Layer]) -> BootMetadata` — the last non-null layer `boot`, or the config `boot`.
    - **Modify `crates/dosker-image/Cargo.toml` and `Cargo.toml`**
        - Add `tar = "0.4"` to the workspace table and to `dosker-image`.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Declare `pub mod layer;` and re-export `FIXED_TIMESTAMP_SECONDS`, `LAYER_META_FILE`, `LAYER_SCHEMA_VERSION`, `Layer`, `LayerEntry`, `LayerMeta`, `effective_boot`, `fixed_timestamp`.
    - **Modify `crates/dosker-image/src/error.rs`**
        - Add `LayerCorrupt { path: String, message: String }`, `UnsafeLayerEntry { path: String, reason: String }`, `MissingLayerSidecar`, and `DigestMismatch { expected: String, actual: String }`.
    - **Verify:** `cargo test -p dosker-image --test layer`

- [x] **Task 7: FAT reader for the tree and boot metadata**
    - **Implements:** FR-002, FR-012, FR-017; design §Build And Runtime Semantics, §Data Model; data-model §BootMetadata, §Tree, §Validation Rules.
    - **Modify `crates/dosker-image/tests/support/mod.rs`**
        - Add `write_fat_dir(path, partition_offset, name)` — creates a directory.
        - Add `write_fat_file_with_attributes(path, partition_offset, name, data, attributes: fatfs::FileAttributes)` — creates a file with the given attributes. The attributes value keeps the helper below clippy's argument limit.
        - Add `write_fat_long_name(path, partition_offset, name, data)` — creates a file with a long name.
        - Add `write_fat_volume_label(path, partition_offset, label: &str)` — writes a 32-byte root directory entry with the `VOLUME_ID` attribute and pads the label to 11 bytes.
    - **Modify `crates/dosker-image/tests/fat.rs`**
        - `read_tree_reads_files_directories_attributes_and_timestamps` — a file under `APP` has the expected path, attributes, and timestamp.
        - `read_tree_reads_file_bytes` — the bytes equal the fixture.
        - `read_tree_rejects_a_long_file_name` — a long-name fixture returns `InvalidDosPath`.
        - `read_tree_skips_the_volume_label` — `write_fat_volume_label` creates a root `VOLUME_ID` entry and the tree does not contain it.
        - `read_tree_preserves_an_empty_directory` — an empty directory appears in the tree.
        - `read_boot_metadata_reads_the_mbr_partition_and_boot_sector` — layout, disk size, partition start and sectors, geometry, FAT type, volume label, and media descriptor.
        - `read_boot_metadata_hashes_the_mbr_and_boot_sector` — the `BlobRef` digests match the bytes.
        - `read_boot_metadata_reads_a_superfloppy` — layout is `Superfloppy` and the MBR is absent.
        - `read_boot_metadata_rejects_a_non_bootable_file` — a blank file returns `NotBootable`.
    - **Modify `crates/dosker-image/src/fat.rs`**
        - `pub fn read_tree(image: &Path) -> Result<Tree, ImageError>` — mounts the FAT volume, walks directories recursively, normalizes every short name through `DosName::normalize`, maps `READ_ONLY`, `HIDDEN`, `SYSTEM`, and `ARCHIVE`, converts the modified DOS date and time to UTC, and reads every file byte.
        - `read_tree` rejects an entry whose `file_name()` differs from `short_file_name()` after ASCII case folding, with `InvalidDosPath`, because that entry has a long name. It skips a volume-label entry and rejects a non-regular entry and an invalid 8.3 name.
        - `read_tree` uses `modified()` for the timestamp. An invalid DOS date or time uses `1980-01-01T00:00:00Z`.
        - `pub fn read_boot_metadata(image: &Path) -> Result<BootMetadata, ImageError>` — reads the MBR partition and the FAT BPB, fills `disk_size`, `geometry`, `partition_start`, `partition_sectors`, `mbr`, `boot_sector`, `fat_type`, `volume_label`, and `media_descriptor`.
        - `read_boot_metadata` sets `layout=MbrPartitioned` for a partitioned disk and `layout=Superfloppy` for a volume at offset zero.
        - `read_boot_metadata` stores the MBR and boot sector as `BlobRef` values with inline base64 bytes.
        - `read_boot_metadata` reads the sector size, sectors per track, and heads from the FAT BPB. It computes cylinders as the total sector count divided by the product of heads and sectors per track, rounded down. It trims trailing spaces from the volume label.
        - Private helpers: `open_volume`, `walk_dir`, `entry_timestamp`, `entry_attributes`, `read_bpb_geometry`, `read_volume_label`, `read_boot_sector`.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Re-export `read_boot_metadata` and `read_tree`.
    - **Verify:** `cargo test -p dosker-image --test fat`

- [x] **Task 8: OCI image store, import, tags, and disk copy**
    - **Implements:** FR-002, FR-012, FR-018; design §Build And Runtime Semantics, §API Contracts, §Data Model; data-model §Image, §Layer, §Validation Rules.
    - **Modify `crates/dosker-image/tests/reference.rs`**
        - `parse_accepts_a_bare_name` — `freedos` becomes the tag `freedos:latest`, matching `ImageReference::parse`.
        - `parse_accepts_a_name_at_digest` — `freedos@sha256:<hex>`.
        - `parse_accepts_a_bare_digest` — `sha256:<hex>`.
        - `parse_rejects_an_uppercase_digest` — uppercase hex fails.
        - `parse_rejects_a_short_digest` — 63 hex characters fail.
        - `parse_rejects_an_empty_name_before_at` — `@sha256:<hex>` fails.
        - `display_round_trips_a_digest` — the digest string is unchanged.
    - **Create `crates/dosker-image/tests/oci.rs`**
        - `import_writes_the_oci_layout_and_manifest` — `oci-layout`, `index.json`, and the manifest, config, and layer blobs exist and validate.
        - `import_writes_the_raw_disk_annotation` — the manifest annotation equals the raw blob digest; the digest is not in `layers`.
        - `import_disk_reads_the_fat_tree_and_boot_metadata` — the config has the source boot metadata and the tree layer has every file and directory.
        - `import_booter_writes_one_opaque_layer` — one layer with the booter media type, `booter=true`, no tree, no raw annotation.
        - `import_accepts_a_superfloppy_disk` — a superfloppy imports with `boot.layout=Superfloppy`, `machine.disk_interface="floppy"`, a tree layer, and the raw-disk annotation.
        - `import_rejects_a_duplicate_reference` — `AlreadyExists`.
        - `import_rejects_a_non_bootable_disk` — `NotBootable`.
        - `import_leaves_the_source_unchanged` — the source bytes are unchanged.
        - `import_stores_read_only_blobs` — every blob is mode `0444` on Unix.
        - `import_is_deterministic_for_the_same_source_and_time` — two imports under different references have the same layer digest and the same raw disk digest.
        - `import_writes_canonical_config_and_manifest` — the config and manifest blob bytes equal `canonical_json` of the parsed values and contain no insignificant whitespace.
        - `oci_layout_descriptors_match_their_blobs` — every descriptor size and digest matches the file.
        - `read_blob_rejects_a_digest_mismatch` — a changed blob returns `DigestMismatch`.
        - `list_returns_tags_sorted` — `alpha:v1`, `alpha:v2`, `zeta:latest`.
        - `load_by_digest_and_tag_returns_the_same_manifest` — the manifest digests match.
        - `load_keeps_an_image_when_the_raw_disk_blob_is_missing` — `load` succeeds with `size=0`; `copy_disk_to` returns `MissingRawDisk`.
        - `tag_adds_a_reference_without_copying_blobs` — the blob count is unchanged.
        - `tag_replaces_an_existing_target` — the target points to the new manifest, and the replaced manifest stays reachable by digest.
        - `remove_untags_a_shared_manifest` — one tag is removed, the manifest and blobs remain.
        - `remove_by_tag_keeps_the_manifest_for_digest_lookup` — removing the only tag returns `untagged`, and `load` by digest still works with all referenced blobs present.
        - `remove_by_digest_deletes_the_manifest_and_unreferenced_blobs` — after untagging, `rmi` by digest removes the manifest, config, tree layers, and raw disk blob while keeping the index parseable.
        - `remove_by_digest_captures_references_before_deleting` — a manifest with a config, a tree layer, and a raw disk is removed by digest and all three captured blobs are deleted.
        - `remove_by_digest_keeps_a_shared_raw_disk_blob` — two manifests share a raw disk; removing one manifest by digest keeps it.
        - `remove_by_digest_keeps_a_blob_used_by_another_manifest` — a shared config or layer blob survives removal of one manifest.
        - `remove_by_digest_rejects_a_tagged_image` — `ImageInUse`.
        - `tag_and_remove_write_a_parseable_index` — after each operation, `index.json` parses and no temporary index file remains.
        - `concurrent_tag_calls_keep_both_index_entries` — two threads tag different targets from one store; both entries survive.
        - `index_lock_is_released_after_import` — a completed import can be followed immediately by a tag without blocking.
        - `copy_disk_to_copies_the_annotated_raw_disk` — the destination equals the source.
        - `copy_disk_to_fails_without_the_annotation` — `MissingRawDisk`.
        - `copy_disk_to_fails_when_the_raw_blob_is_missing` — `MissingRawDisk`.
        - `copy_disk_to_does_not_modify_the_blob` — a write to the destination leaves the blob unchanged.
        - `booter_copy_disk_to_copies_the_single_layer` — the destination equals the booter source.
    - **Create `crates/dosker-image/src/oci.rs`**
        - `pub struct OciStore { root: PathBuf }` — `new`, `layout_dir`, `blobs_dir`, `blob_path`, `ensure_layout`, `lock_index`, `write_index`, `put_blob`, `put_blob_from_file`, `read_blob`, `copy_blob_to`, `copy_disk_to`, `import`, `load`, `list`, `tag`, `remove`.
        - `pub struct IndexLock { file: File }` — holds the exclusive `images/.lock` handle and unlocks on drop.
        - `OciStore::new(root: PathBuf) -> Self` uses `<root>/images` as the layout root.
        - `OciStore::layout_dir(&self) -> PathBuf`, `blobs_dir(&self) -> PathBuf`, `blob_path(&self, digest: &str) -> Result<PathBuf, ImageError>`, `ensure_layout(&self) -> Result<(), ImageError>`.
        - `OciStore::ensure_layout` creates `images/`, `images/blobs/sha256/`, and `oci-layout` with the version.
        - `OciStore::lock_index(&self) -> Result<IndexLock, ImageError>` — creates the layout, opens `images/.lock` with create/read/write, and takes an exclusive `fs2` file lock. `IndexLock` unlocks on drop.
        - `OciStore::put_blob(&self, bytes: &[u8]) -> Result<Blob, ImageError>` — hashes, writes a temp file, verifies, renames, and sets mode `0444` on Unix. If the blob exists, it hashes the existing file and returns it only when both the digest and size match; otherwise it returns `DigestMismatch`.
        - `OciStore::put_blob_from_file(&self, source: &Path) -> Result<Blob, ImageError>` — copies while hashing and leaves the source unchanged.
        - `OciStore::read_blob(&self, digest: &str) -> Result<Vec<u8>, ImageError>` — rejects a malformed digest and a digest mismatch.
        - `OciStore::copy_blob_to(&self, digest: &str, destination: &Path) -> Result<CopyOutcome, ImageError>` — uses `copy_reflink`; the blob is read-only.
        - `OciStore::copy_disk_to(&self, reference: &ImageRef, destination: &Path) -> Result<CopyOutcome, ImageError>` — resolves the image, copies the raw disk digest to the destination, and returns `MissingRawDisk` when the annotation or blob is absent. The stored image is not opened for write.
        - `OciStore::import(&self, source: &Path, reference: &ImageReference, media: ImageMedia, now: DateTime<Utc>) -> Result<Image, ImageError>`:
            - Takes `lock_index` before checking the reference. A repeated reference returns `AlreadyExists` before any blob is written.
            - A disk import first stores the raw disk blob, then reads the FAT tree and boot metadata from that stored blob, so the tree and the raw disk describe the same bytes. It accepts both `BootLayout::MbrPartitioned` and `BootLayout::Superfloppy`. It creates one tree layer with `created_by="import"`, a null parent, and no boot override, and writes the raw-disk annotation. The config has `booter=false`, `boot` with the source boot metadata, `workdir` `C:\`, `cmd` `None`, the default machine profile with `machine.disk_interface="ide"` for `mbr_partitioned` and `"floppy"` for `superfloppy`, empty environment, ports, volumes, engine pin, and requires, and `agent_version` `None`.
            - A booter import stores one opaque blob with the booter media type, `booter=true`, `boot=BootMetadata::opaque(size)`, no tree, no raw-disk annotation, and the single layer digest as `raw_disk_digest`. Its environment, ports, volumes, engine pin, and requires are empty, its `workdir` is empty, `cmd` is `None`, `agent_version` is `None`, and the machine profile is the default.
            - The config has one history entry with `created_by="import"`, `created_at=now`, `empty_layer=false`, and the source path as the comment.
            - A disk import sets `dos_flavour=Other`; a booter sets `dos_flavour=None`.
            - The config blob is `canonical_json(&config)`; its descriptor uses the `Blob.digest` and `Blob.size` returned by `put_blob`. The layer descriptor uses the `Blob` returned by `put_blob` for the canonical tar. The manifest is built from that config descriptor and the layer descriptor, serialized with `canonical_json`, and stored with `put_blob`; the index descriptor uses that manifest `Blob.digest` and `Blob.size`. Do not use `serde_json::to_vec` for the config or manifest blob. The returned `Image.disk_interface` comes from `config.machine.disk_interface`.
            - The import calls `config.validate()` and `manifest.validate(media == ImageMedia::Booter)`, writes the config and manifest blobs, then writes the index with `write_index`, and removes blobs it created if a later step fails.
        - `OciStore::load(&self, reference: &ImageRef) -> Result<Image, ImageError>` — resolves a tag or digest, reads the manifest and config, calls `manifest.validate(config.booter)` and `config.validate()`, and builds the `Image` view. A digest reference falls back to the manifest blob when no index entry points to it. For a digest reference, `name` and `tag` come from the first tag that points to the manifest, or are empty. `source` and `created_at` come from the last history entry. For a booter, `raw_disk_digest` is the single layer digest and `disk_interface` is `ide`. For a non-booter, `raw_disk_digest` is the manifest annotation and `disk_interface` comes from `config.machine.disk_interface`. When the annotation or its blob is absent, `size` is 0; `load` still succeeds. `copy_disk_to` returns `MissingRawDisk`.
        - `OciStore::list(&self) -> Result<Vec<Image>, ImageError>` — one entry per index tag, sorted by name and tag. A missing layout or index returns an empty list.
        - `OciStore::write_index(&self, index: &Index) -> Result<(), ImageError>` is private — it serializes the index with `canonical_json` and writes `index.json` through `write_atomic`. Every caller holds `IndexLock`; no operation writes `index.json` in place, and no other module can call it without the lock.
        - `OciStore::tag(&self, source: &ImageRef, target: &ImageReference) -> Result<(), ImageError>` — takes `lock_index`, loads the source manifest descriptor, adds or replaces the target index entry, and writes the index with `write_index`; no blob is copied. A replaced manifest stays addressable by digest.
        - `OciStore::remove(&self, reference: &ImageRef) -> Result<RemoveOutcome, ImageError>` — takes `lock_index` first. A tag reference removes only the index entry, writes the index with `write_index`, and returns `untagged`; the manifest and its blobs stay reachable by digest. A digest reference first loads the manifest and captures its config digest, layer digests, and raw-disk digest. If any index tag still points to that digest, it returns `ImageInUse`. Otherwise it removes the manifest blob, scans every remaining manifest blob under `blobs/sha256/` (ignoring blobs that do not parse as a manifest), collects the config, layer, and raw-disk digests they reference, and deletes only the removed manifest's captured blobs that no remaining manifest references. It never deletes another manifest blob. It writes the index with `write_index` before returning.
        - `pub struct Image { pub reference: String, pub name: String, pub tag: String, pub media: ImageMedia, pub manifest_digest: String, pub config_digest: String, pub layer_digests: Vec<String>, pub raw_disk_digest: Option<String>, pub size: u64, pub source: String, pub disk_interface: String, pub agent_version: Option<String>, pub cmd: Option<String>, pub created_at: DateTime<Utc> }` — `size` is the raw disk blob size, or 0 when the raw disk blob is absent; `disk_interface` is `ide` for a booter and for a partitioned disk, or `floppy` for a superfloppy; `is_booter(&self) -> bool`, `expects_agent(&self) -> bool`.
        - `pub struct Blob { pub digest: String, pub size: u64, pub created: bool }`.
        - `pub struct RemoveOutcome { pub untagged: bool, pub removed_manifest: bool, pub deleted_blobs: usize }` — a tag removal returns `untagged=true`, `removed_manifest=false`, and `deleted_blobs=0`; a digest removal returns `removed_manifest=true` with the deleted count.
    - **Modify `crates/dosker-image/src/reference.rs`**
        - `ImageDigest(String)` — `parse`, `as_str`, `Display`, serde.
        - `pub enum ImageRef { Tag(ImageReference), Digest(ImageDigest) }` — `parse`, `Display`.
        - `ImageRef::parse` accepts `name` (as `name:latest`), `name:tag`, `name@sha256:<hex>`, and `sha256:<hex>`.
    - **Modify `crates/dosker-image/src/image.rs`**
        - Keep `ImageMedia` and its helpers.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Declare `pub mod oci;` and re-export `Blob`, `Image`, `ImageDigest`, `ImageRef`, `IndexLock`, `OciStore`, `RemoveOutcome`.
    - **Modify `crates/dosker-image/Cargo.toml` and `Cargo.toml`**
        - Add `fs2 = "0.4"` to the workspace table and to `dosker-image`.
    - **Modify `crates/dosker-image/src/error.rs`**
        - Add `MissingRawDisk(String)` with message `image {0} has no raw disk; import it again or rebuild it`.
        - Add `ImageInUse(String)` with message `image {0} is tagged; remove the tags first`.
        - Add `Lock { path: PathBuf, source: io::Error }` with a message that names the index lock.
    - **Verify:** `cargo test -p dosker-image --test oci --test reference`

- [x] **Task 9: Build cache record and key computation**
    - **Implements:** FR-003; design §Build And Runtime Semantics; data-model §Build Cache; contracts/doskerfile.md §Build Cache.
    - **Create `crates/dosker-image/tests/cache.rs`**
        - `cache_key_matches_the_pinned_concatenation` — the expected SHA-256 for a fixed input.
        - `cache_key_changes_when_the_parent_tree_digest_changes` — one input changes the key.
        - `cache_key_changes_when_the_config_digest_changes`.
        - `cache_key_changes_when_the_instruction_changes`.
        - `cache_key_changes_when_an_input_hash_changes`.
        - `cache_key_is_stable_for_the_same_input_hash_set` — the input hash order does not matter.
        - `cache_key_is_bare_lowercase_hex` — 64 lowercase hex characters with no `sha256:` prefix.
        - `cache_record_round_trips_through_json`.
        - `cache_store_round_trips_a_record` — store, load, remove.
        - `cache_store_creates_the_cache_directory` — the first `store` creates `<root>/cache`.
        - `cache_store_missing_directory_is_empty` — `load` returns `None` and `list` returns an empty vector when `<root>/cache` is absent.
        - `cache_store_rejects_a_corrupt_entry` — a wrong schema or key returns `Corrupt`.
    - **Create `crates/dosker-image/src/cache.rs`**
        - `pub const CACHE_SCHEMA_VERSION: u32 = 1;`
        - `pub struct CacheKeyInput<'a> { pub parent_tree_digest: &'a str, pub config_digest: &'a str, pub instruction: &'a str, pub input_hashes: &'a [String] }`.
        - `pub fn cache_key(input: &CacheKeyInput<'_>) -> String` — SHA-256 of `parent_tree_digest || "\n" || config_digest || "\n" || instruction || "\n" || sorted_input_hashes.join("\n")`, returned as 64 lowercase hex characters without a prefix.
        - `pub struct CacheRecord { pub schema_version: u32, pub key: String, pub layer_digest: String, pub instruction: String, pub created_at: DateTime<Utc> }` — `key` is bare lowercase hex.
        - `pub struct CacheStore { root: PathBuf }` — `CacheStore::new(root: PathBuf) -> Self`, `entry_path(&self, key: &str) -> PathBuf`, `load(&self, key: &str) -> Result<Option<CacheRecord>, ImageError>`, `store(&self, record: &CacheRecord) -> Result<(), ImageError>`, `remove(&self, key: &str) -> Result<(), ImageError>`, `list(&self) -> Result<Vec<CacheRecord>, ImageError>`.
        - `CacheStore::entry_path(&self, key: &str) -> PathBuf` — `<root>/cache/<key>.json`; the key is a valid Windows file name.
        - `CacheStore::store` creates `<root>/cache` with mode `0700` through `ensure_dir_mode` before `write_atomic`.
        - `CacheStore::load` returns `Ok(None)` when `<root>/cache` or the entry file is absent; `list` returns an empty vector when the directory is absent.
        - `CacheStore` does not read or delete layer blobs. The data model's blob-existence check and invalidation are deferred to the build slice, so this task adds no entry invalidation.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Declare `pub mod cache;` and re-export `CacheKeyInput`, `CacheRecord`, `CacheStore`, `cache_key`.
    - **Verify:** `cargo test -p dosker-image --test cache`

- [x] **Task 10: CLI image commands and runtime compatibility**
    - **Implements:** FR-012, FR-017, FR-018; design §CLI, §Build And Runtime Semantics; data-model §Container, §Validation Rules.
    - **Modify `crates/dosker-cli/tests/common/mod.rs`**
        - Add `images_dir(&self) -> PathBuf` — `<root>/images`.
        - Add `write_tree_only_image(&self, reference: &str)` — writes a minimal OCI image with a tree layer and no raw-disk annotation for the missing-annotation test.
        - Keep `import_image` and `import_image_as`; they drive the CLI and now use the OCI store.
    - **Modify `crates/dosker-cli/tests/cli_image.rs`**
        - `images_lists_imported_references` — the header and sorted references.
        - `images_lists_two_tags_for_the_same_digest` — one digest, two references.
        - `rmi_untags_an_image_with_another_tag` — `untagged`, the other tag still lists.
        - `rmi_untags_the_last_tag_and_keeps_the_manifest_addressable` — `untagged`; `load` by digest still works and the blobs remain.
        - `rmi_by_digest_removes_the_manifest_and_blobs` — after untagging, `rmi <digest>` prints `removed` and the blobs are gone.
        - `rmi_rejects_an_unknown_reference` — exit 1.
        - `rmi_rejects_an_invalid_reference` — exit 2.
        - `tag_adds_a_reference_and_create_uses_it` — `tagged`, then `create` from the target succeeds.
        - `tag_rejects_an_unknown_source` — exit 1.
        - `create_copies_the_annotated_raw_disk` — the container disk equals the source image.
        - `create_records_the_image_disk_interface` — an MBR image records `ide` and a superfloppy image records `floppy`.
        - `create_rejects_an_image_without_a_raw_disk` — exit 1 and a message that names the raw disk.
        - `create_fails_when_the_raw_disk_blob_is_missing` — exit 1 and a message that names the raw disk.
        - `booter_import_and_create_round_trip` — the booter container disk equals the source.
        - `image_import_accepts_a_superfloppy_disk` — a superfloppy imports with `disk_interface="floppy"`, and `create` copies the raw disk and records the same interface.
        - `stored_image_blobs_are_read_only` — mode `0444` on Unix.
        - `image_import_keeps_the_existing_output_shape` — `imported <ref> <digest> <size> bytes`.
        - `image_import_without_a_tag_uses_latest` — the existing test now loads through `OciStore` and checks `Image.name` and `Image.tag`.
        - Keep the existing import tests for a missing file, a non-bootable disk, a duplicate reference, and an invalid reference. Replace the superfloppy rejection with the accepting test above.
    - **Modify `crates/dosker-cli/tests/cli_lifecycle.rs`**
        - Replace `ImageStore` with `OciStore`; load the normal and booter images with `ImageRef::Tag`; check `Image.is_booter()` and `Image.cmd`.
        - `create_copies_the_image_to_a_private_disk` — load the source `Image`, read `raw_disk_digest`, and compare the container disk with the `OciStore` blob bytes. Remove the legacy image-directory path assertion.
        - `create_does_not_modify_the_image` — read the raw-disk blob through `OciStore` before and after `create`. Remove the legacy path assertion.
        - Compare `ContainerRecord.image_digest` with `Image.manifest_digest`, not the old raw blob digest.
        - `create_records_the_image_disk_interface` — an MBR image records `ide` and a superfloppy image records `floppy`.
        - Keep the container `disk_path` assertions; that field is part of the runtime record.
    - **Modify `crates/dosker-runtime/src/record.rs`**
        - Add `#[serde(default = "default_disk_interface")] pub disk_interface: String` with `fn default_disk_interface() -> String` returning `ide`; initialize it in `ContainerRecord::new`; add `with_disk_interface(mut self, interface: &str) -> Self`.
    - **Modify `crates/dosker-engine/src/spec.rs`**
        - Add `pub disk_interface: String` to `EngineSpec`; update every `EngineSpec` literal and sample to set it.
    - **Modify `crates/dosker-engine-qemu/src/command.rs`**
        - Build the IDE drive argument from `spec.disk_interface`: `ide` emits `if=ide`; `floppy` emits `if=floppy`. The rest of the drive string is unchanged.
    - **Modify `crates/dosker-runtime/src/supervisor.rs`**
        - Pass `record.disk_interface.clone()` into the `EngineSpec`.
    - **Modify `crates/dosker-engine-qemu/tests/command.rs`**
        - Replace `command_uses_ide_for_the_disk_and_booter` with `command_uses_ide_for_a_partitioned_disk` and `command_uses_floppy_for_a_superfloppy`; both assert the `-drive` value and the `if=` token.
    - **Modify `crates/dosker-runtime/tests/supervisor.rs`**
        - Add `supervisor_passes_the_disk_interface_to_the_spec` using the existing stub adapter.
    - **Create `crates/dosker-cli/src/commands/images.rs`**
        - `pub fn execute(_args: ImagesArgs, context: &mut Context) -> Result<u8, CliError>` — prints the header `REFERENCE DIGEST SIZE CREATED` and one row per tag with `image.reference`, `image.manifest_digest`, `image.size`, and `image.created_at` as RFC 3339 whole seconds; returns 0.
    - **Create `crates/dosker-cli/src/commands/rmi.rs`**
        - `pub fn execute(args: RmiArgs, context: &mut Context) -> Result<u8, CliError>` — parses `ImageRef`, calls `context.images.remove`, prints `removed <ref>` when `removed_manifest` is true and `untagged <ref>` otherwise; returns 0.
    - **Create `crates/dosker-cli/src/commands/tag.rs`**
        - `pub fn execute(args: TagArgs, context: &mut Context) -> Result<u8, CliError>` — parses the source as `ImageRef` and the target as `ImageReference`, calls `context.images.tag`, prints `tagged <target>`, returns 0.
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - Add `Images(ImagesArgs)`, `Rmi(RmiArgs)`, and `Tag(TagArgs)` to `Command`.
        - Add `pub struct ImagesArgs {}`, `pub struct RmiArgs { pub reference: String }`, and `pub struct TagArgs { pub source: String, pub target: String }`. `RmiArgs.reference` carries help text that a tag untags and a digest removes an untagged image.
        - Dispatch the three commands to `commands::images`, `commands::rmi`, and `commands::tag`.
    - **Modify `crates/dosker-cli/src/commands/mod.rs`**
        - Declare `pub mod images; pub mod rmi; pub mod tag;`.
    - **Modify `crates/dosker-cli/src/context.rs`**
        - Change `pub images: ImageStore` to `pub images: OciStore`; `with_console` constructs `OciStore::new(root)`.
    - **Modify `crates/dosker-cli/src/commands/image.rs`**
        - Keep the CLI form and output; `context.images.import` returns `Image`; print `image.name`, `image.tag`, `image.manifest_digest`, and `image.size`.
    - **Modify `crates/dosker-cli/src/commands/create.rs`**
        - `create_container` parses `ImageRef`; `context.images.load` returns `Image`; `context.images.copy_disk_to` copies the annotated raw disk; `ContainerRecord.with_image` uses `image.manifest_digest`, `image.is_booter()`, and `image.cmd`; call `.with_disk_interface(&image.disk_interface)` when building the record.
    - **Create `docs/images.md`**
        - Describe the local OCI layout under `<data-root>/images`, the DOSker media types, and the read-only blob rule.
        - Document `image import`, `images`, and `tag`.
        - State that `rmi <name:tag>` untags only, that the manifest and blobs remain digest-addressable, and that `rmi <sha256:...>` removes an untagged image and its unreferenced blobs. State that a tagged digest returns `ImageInUse`.
        - State that image mutations are serialized by `images/.lock`.
    - **Modify `README.md`**
        - Link to `docs/images.md` from the image/runtime section.
    - **Verify:** `cargo build -p dosker-test-support --bin mock-engine && cargo test -p dosker-cli --features mock-engine --test cli_image --test cli_run --test cli_lifecycle && cargo test -p dosker-engine-qemu --test command && cargo test -p dosker-runtime --test supervisor && cargo test -p dosker-engine --test adapter && git add -N docs/images.md && git diff --check && rg -n "rmi|images/.lock|OCI" docs/images.md README.md`

- [x] **Task 11: Remove the legacy directory store and run the full regression**
    - **Implements:** FR-018; design §Data Model, §Rollout And Compatibility; data-model §Image.
    - **Remove `crates/dosker-image/src/store.rs`**
        - The legacy `images/<name>/<tag>/` store, `ImageStore`, `image_dir`, `disk_path`, `IMAGE_RECORD_FILE`, and `IMAGE_DISK_FILE` go away. `OciStore` replaces them.
    - **Remove `crates/dosker-image/tests/store.rs`**
        - The legacy store tests are replaced by `tests/oci.rs`.
    - **Modify `crates/dosker-image/src/image.rs`**
        - Remove `ImageRecord` and `IMAGE_SCHEMA_VERSION`; keep `ImageMedia` and its helpers.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Remove `pub mod store;` and the `ImageStore`, `ImageRecord`, `IMAGE_RECORD_FILE`, and `IMAGE_DISK_FILE` re-exports.
    - **Check for stale references**
        - `rg "ImageStore|ImageRecord|IMAGE_RECORD_FILE|IMAGE_DISK_FILE|fn image_dir|fn disk_path" crates` returns no product-code hit. The `ContainerRecord.disk_path` field is not a legacy-store reference and stays.
    - **Verify:** `cargo fmt --all -- --check`; `cargo build --workspace --all-targets --features dosker-cli/mock-engine`; `cargo clippy --workspace --all-targets --features dosker-cli/mock-engine -- -D warnings`; `cargo test --workspace --features dosker-cli/mock-engine`

## Requirement Coverage

| Requirement | Tasks |
| --- | --- |
| FR-002 — content-addressed layers, layer application, canonical tar | 2, 5, 6, 7, 8, 10 |
| FR-003 — cache record keyed by parent tree digest, config digest, normalized instruction, and input hashes | 2, 4, 6, 9 |
| FR-012 — local OCI layout, DOSker media types, index, manifest, config, blobs, tags | 3, 4, 5, 8, 10 |
| FR-017 — uppercase 8.3 validation, traversal and symlink rejection | 1, 6, 7, 10 |
| FR-018 — read-only image and private writable disk per container | 8, 10, 11 |

FR-003 is record and key computation only. No build path calls the cache in
this slice, and no invalidation logic is added.

## Done When Map

| Roadmap acceptance clause | Proved by |
| --- | --- |
| Layers apply in order and produce a tree | Task 2 `apply_delta_*` tests; Task 6 `layer_apply_to_applies_whiteouts_before_entries` |
| Two independent writes of the same tree produce the same layer digest | Task 6 `layer_digest_is_stable_across_independent_writes`; Task 8 `import_is_deterministic_for_the_same_source_and_time` |
| Long layer paths round-trip through GNU or PAX tar extensions | Task 6 `layer_writer_uses_a_gnu_long_name_for_a_long_path`, `layer_reader_accepts_a_gnu_long_path`, `layer_reader_accepts_a_pax_long_path`, `layer_long_path_round_trips` |
| A whiteout removes a path and a missing whiteout is ignored | Task 2 `whiteout_removes_a_file`, `whiteout_removes_a_directory_subtree`, `whiteout_of_a_missing_path_is_ignored` |
| A file-versus-directory conflict is an error | Task 2 `file_and_directory_conflict_is_an_error` |
| The OCI index, manifest, config, and blobs validate against the OCI layout | Task 5 manifest and index tests; Task 8 `oci_layout_descriptors_match_their_blobs` |
| An imported disk and an imported booter blob round-trip through the store | Task 7 FAT reader tests; Task 8 import and copy tests, including `import_accepts_a_superfloppy_disk`; Task 10 CLI tests |
| A write to a stored image is rejected | Task 8 `import_stores_read_only_blobs`, `copy_disk_to_does_not_modify_the_blob`; Task 10 `stored_image_blobs_are_read_only` |
| `create` copies the annotated raw disk and the image stays read-only | Task 8 `copy_disk_to_copies_the_annotated_raw_disk`; Task 10 `create_copies_the_annotated_raw_disk` |
| The disk interface follows the image into the engine command | Task 8 `Image.disk_interface`; Task 10 `create_records_the_image_disk_interface`, `supervisor_passes_the_disk_interface_to_the_spec`, `command_uses_ide_for_a_partitioned_disk`, `command_uses_floppy_for_a_superfloppy` |
| `create` fails clearly when the annotation or blob is missing | Task 8 `copy_disk_to_fails_without_the_annotation`, `copy_disk_to_fails_when_the_raw_blob_is_missing`; Task 10 CLI tests |
| `tag` keeps the annotated blob and `rmi` respects shared references | Task 8 `tag_replaces_an_existing_target`, `remove_untags_a_shared_manifest`, `remove_by_tag_keeps_the_manifest_for_digest_lookup`, `remove_by_digest_deletes_the_manifest_and_unreferenced_blobs`, `remove_keeps_a_shared_raw_disk_blob`; Task 10 `tag_adds_a_reference_and_create_uses_it`, `rmi_untags_an_image_with_another_tag`, `rmi_untags_the_last_tag_and_keeps_the_manifest_addressable`, `rmi_by_digest_removes_the_manifest_and_blobs` |
| `images`, `rmi`, and `tag` work | Task 10 CLI tests and `docs/images.md` |
| Concurrent image mutations serialize through the index lock | Task 8 `concurrent_tag_calls_keep_both_index_entries`, `index_lock_is_released_after_import`; Task 10 `docs/images.md` |
| Existing `image import`, `create`, and `run` keep passing | Task 10 `cli_image`, `cli_run`, and `cli_lifecycle` suites; Task 11 full workspace suite |

## Out Of Scope

- The Doskerfile parser, `FROM`, `COPY`, `RUN`, `RUN --host`, `ENV`,
  `WORKDIR`, `CMD`, `MACHINE`, `ENGINE`, `EXPOSE`, `VOLUME`, `REQUIRES`, and
  the lock file.
- The build engine, cache invalidation wiring, and cache hits in a build.
- The tree-to-FAT projection and the removal of the raw-disk bridge.
- Agent injection into an imported image.
- Volumes, NAT, published ports, `--bind`, development mode, and the other
  engines.
- `save`, `load`, `system prune`, a registry, and long file names.
- Automatic migration of existing `images/<name>/<tag>/` directories. The
  design states that there is no migration for this new project.

## Risks

- **FAT reader fidelity.** `fatfs` exposes short names, attributes, and DOS
  timestamps, but not every source image uses only 8.3 names. The import
  rejects a long name instead of silently shortening it. A source with long
  names must be fixed and imported again. The raw-disk bridge applies only to
  a successful import, and the design already excludes long file names.
- **Canonical tar drift.** The `tar` crate writes a header block whose fields
  must all be set explicitly. The writer sets path, mode, uid, gid, user,
  group, mtime, size, and entry type, and the digest-stability tests catch a
  change in a future crate version.
- **Long-name canonical form.** The writer emits GNU `L` records for paths
  over 100 bytes; the reader also accepts PAX `x`/`g` records. Re-writing a PAX
  layer normalizes it to the GNU form, so its digest may change even though
  the tree is identical. The canonical digest tests pin the GNU output.
- **Raw-disk double storage.** An imported non-booter image stores the tree
  layer and the raw disk blob. The bridge is temporary and is removed with the
  tree-to-FAT projection. Until then, an import uses roughly twice the disk
  space of the tree alone.
- **Shared blob deletion.** A digest `rmi` removes the manifest, then scans
  every blob that parses as a manifest before it deletes a config, layer, or
  raw-disk blob. A tag `rmi` keeps the manifest and its blobs for digest
  lookup. A mistake could delete a shared blob; the tag, digest, and
  shared-raw-disk tests pin the scan.
- **Windows read-only blobs.** Unix blobs are mode `0444`; Windows relies on
  read-only opens and the absence of a write path. The Windows matrix leg runs
  the same store tests.
- **Superfloppy boot verification.** The disk interface is recorded and passed
  through to the QEMU command. This slice proves the command uses `if=floppy`;
  a real superfloppy boot remains an integration test with a suitable fixture.
- **No legacy data migration.** Existing `images/<name>/<tag>/` directories
  are ignored and then removed. A user must import the source again. The
  design states that there is no migration.
- **Tag replacement.** `tag` replaces an existing target entry and leaves the
  previous manifest untagged and reachable by digest. It is removed by a
  digest `rmi` or a later prune. The tag tests pin the replacement.
- **Digest references.** A digest reference resolves an untagged manifest
  after its last tag is removed. `rmi` by digest refuses an image that still
  has tags, and removes an untagged manifest and its unreferenced blobs. The
  tests pin both paths so a user cannot remove a tagged image by accident.

## Deviations

- **Raw-disk OCI blob for non-booter imports.** A non-booter imported DOS
  image stores its canonical tree layer and also the original raw disk as a
  read-only OCI blob, referenced by the manifest annotation
  `org.dosker.rawdisk.digest`. The blob is not a layer, it is not in
  `layers`, and it is not in `rootfs.diff_ids`. `create` copies that blob to
  the private writable disk. `tag` keeps the blob. `rmi` by tag removes only
  the tag and leaves the digest-addressable image; `rmi` by digest deletes the
  blob only when no tag remains. The bridge is temporary and is removed with
  the later tree-to-FAT projection. This is the only approved deviation from
  the design's non-booter image shape.

No other deviation from the design's non-booter image shape is introduced.
The legacy directory store is not read and is removed after the OCI store
covers import, tags, listing, removal, and disk copy; the design states that
there is no migration for this new project.
