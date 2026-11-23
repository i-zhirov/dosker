# DOSker Registry References And Validation Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> `Verify` command passes.

- **Date**: 2026-11-23
- **Status**: Implemented
- **Design**: `.sdd/specs/2026-11-22-doskerfile-registry/design.md`
- **Roadmap**: `.sdd/specs/2026-11-22-doskerfile-registry/roadmap.md` — slice: registry-references-and-validation
- **Depends on**: license-core
- **Manual prerequisites**: —
- **Goal**: Add the namespaced image-reference grammar, the registry-base identity and lock recording, and a `dosker registry validate` command that checks registry metadata, namespaces, tags, licenses, coverage, and the notice rule against a local registry checkout. The remote base resolution moves to the network-client slice.

## Constraints

- Bare `name:tag` stays local. `owner/name:tag` is namespaced. `host/owner/name:tag` names an explicit registry. `owner/name@sha256:<hex>` pins a digest. A bare `sha256:<hex>` stays a local digest reference.
- A namespace is a lowercase GitHub handle: `[a-z0-9](?:[a-z0-9-]{0,37}[a-z0-9])?`.
- An image name stays `[a-z0-9][a-z0-9_.-]{0,62}`. A tag stays `[A-Za-z0-9][A-Za-z0-9_.-]{0,127}`.
- A digest is `sha256:` followed by 64 lowercase hex characters.
- `DOSKER_REGISTRY` overrides the default registry base URL. The default is `https://i-zhirov.github.io/doskerfile-registry`.
- A lock base pin records the full base identity: `freedos`, `name`, `owner/name`, or `host/owner/name`, plus the tag and the manifest digest. The identity never includes the tag or digest suffix. A bare digest base has an empty name and tag.
- `registry.toml` requires `schema = 1`, a tag-shaped `version`, a non-empty unique `tags` list of at most 32 entries, and a `description` of 1 to 280 characters. `homepage` and `deprecated` are optional.
- `namespaces.toml` requires `schema = 1` and an `owners` map. Each key is a handle. Each owner has `github` equal to the key and a non-empty `proof` URL. `note` is at most 280 characters.
- Every image must declare at least one `LICENSE` entry, and `validate_licenses` must pass, including the contradictory status/basis/SPDX rule.
- A copyleft SPDX expression requires `SOURCE_URL`. An image with any copyleft or `abandonware` entry must contain `C:\DOSKER\NOTICE.TXT`.
- A booter image has no tree, so the validator cannot prove the notice. A booter with a copyleft or abandonware entry fails validation. A booter with only redistributable entries passes with `COVERS=*`.
- Coverage checks the resolved base and the `COPY` inputs only. It does not run `RUN` or `RUN --host`. The full-tree check runs at publish.
- A stored base tree is loaded for coverage and the notice check. The built-in `freedos:<v>` base is system content and is not loaded. A base that is not in the local store fails with a pull hint; the remote config fetch is a later slice.
- System top-level directories are `DOS` and `DOSKER`. System root files are `KERNEL.SYS`, `COMMAND.COM`, `CONFIG.SYS`, `AUTOEXEC.BAT`, and `AUTOEXEC.OLD`. Executable extensions are `EXE`, `COM`, and `BAT`.
- Every non-system top-level directory must match a `COVERS` glob for the directory itself or its `\*`. Every root-level executable must match a `COVERS` glob for the file.
- A booter image must declare `COVERS=*`.
- `COVERS` globs use `*` and `?` and match case-insensitively.
- `dosker registry validate` exits `0` when every image is valid, `1` when validation fails, and `2` on a usage error.
- The validator resolves a base from the local image store only. The remote `config.json` fetch is a later slice, together with the network client. A missing namespaced base is a clear validator error.
- On this macOS host, run every `Verify` command with the prefix `CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/usr/bin/cc CC=/usr/bin/cc`. The plan writes the portable command; the prefix is a local environment requirement.

## Plan Decisions

- `Repository`, `Namespace`, `RegistryHost`, `RegistryReference`, and `ReferenceSelector` live in `crates/dosker-image/src/reference.rs`. `ImageReference` gains a `Repository` in place of its single `name` field. `name()` keeps returning the last segment; `repository()` returns the full path.
- `RegistryReference::parse` splits a selector from the right. A `@sha256:` suffix is a digest selector. Otherwise, a `:` after the last `/` starts a tag selector; a `:` before the last `/` belongs to a host port. The remaining path has one segment (local name), two (`owner/name`), or three (`host/owner/name`); more is an error. A value that starts with `sha256:` and has no `/` is a bare local digest, not a registry reference.
- `RegistryReference::local_reference()` returns the local `ImageReference` for a tag selector, dropping the host. `RegistryReference::identity()` returns `host/repository` or `repository` without the selector. The lock uses `identity()`.
- The build base resolver and the validator use the local store only; auto-pull and the remote `config.json` fetch are later slices. The design's `validate.yml` remote base needs the network client; until then its base must be present locally.
- Registry metadata, namespace, validation, and coverage types live in a new `crates/dosker-build/src/registry.rs`. They are registry concepts, not image-store concepts.
- The validator reuses `BuildContext`/`plan_copy`/`Tree::apply_delta` for `COPY` and `merge_license_entry` for `LICENSE`. It skips `RUN` and `RUN --host`.
- The `COPY` notice exception is local to `plan_copy`. The global `is_reserved_path` stays unchanged so `RUN` and `RUN --host` keep the `C:\DOSKER` guard.
- The registry root is the explicit `--registry-root` when given, otherwise the nearest ancestor of the image directory that contains `images/`. A per-image validation still checks `namespaces.toml` and registry-wide tag uniqueness.
- `LicenseEntry` gains `is_copyleft()` in `dosker-core`, and `LicenseEntry::validate` uses it.

## Tasks

- [x] **Task 1: Reference repository and registry types**
    - **Implements:** FR-005, FR-017, design §Requirements, design §Component Interfaces
    - **Modify `crates/dosker-image/src/reference.rs`**
        - `pub const DEFAULT_REGISTRY: &str = "https://i-zhirov.github.io/doskerfile-registry";`
        - `pub const REGISTRY_ENV: &str = "DOSKER_REGISTRY";`
        - `pub fn default_registry_url() -> String` — read `REGISTRY_ENV`, ignore an empty value, trim a trailing `/`, otherwise return `DEFAULT_REGISTRY`.
        - `pub struct Namespace(String)` — `parse(&str) -> Result<Self, ImageError>`, `as_str`, `Display`, and serde as a string. Enforce the handle pattern in Constraints.
        - `pub struct Repository { namespace: Option<Namespace>, name: ImageName }` — `parse(&str) -> Result<Self, ImageError>`, `new(Option<Namespace>, ImageName)`, `namespace()`, `name()`, `is_namespaced()`, `Display`.
        - Change `ImageReference` to `{ repository: Repository, tag: ImageTag }`. `parse` accepts `[namespace/]name[:tag]` and rejects three path segments. Add `repository()`, `namespace()`, keep `name()` returning the last segment, keep `tag()`, `Display` prints the full path and tag, serde stays a string.
        - `pub struct RegistryHost(String)` — `parse`, `as_str`, `Display`. Accept a host name with an optional `:port`; reject `/`, empty, and whitespace.
        - `pub enum ReferenceSelector { Tag(ImageTag), Digest(ImageDigest) }`.
        - `pub struct RegistryReference { host: Option<RegistryHost>, repository: Repository, selector: ReferenceSelector }` — `parse`, `host()`, `repository()`, `selector()`, `local_reference() -> Option<ImageReference>` (tag selector only), `identity() -> String`, `Display`.
        - `RegistryReference::parse` treats a bare `sha256:<hex>` value as not a registry reference (the caller uses `ImageRef`).
        - `ImageRef::parse` accepts a namespaced tag, a bare digest, and a namespaced name with `@sha256:`; the digest form validates the repository and returns `Digest`.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Re-export `DEFAULT_REGISTRY`, `Namespace`, `REGISTRY_ENV`, `ReferenceSelector`, `RegistryHost`, `RegistryReference`, `Repository`, `default_registry_url`.
    - **Modify `crates/dosker-image/tests/reference.rs`**
        - `parse_accepts_a_namespaced_reference` — `owner/name:tag` keeps the owner and name.
        - `parse_defaults_the_namespace_tag` — `owner/name` gains `latest`.
        - `display_round_trips_a_namespaced_reference`.
        - `parse_rejects_a_second_slash` — a regression pin; `owner/name/extra:tag` fails.
        - `parse_rejects_an_uppercase_namespace` — a regression pin.
        - `registry_reference_parses_a_default_registry` — `owner/name:tag` has no host.
        - `registry_reference_parses_an_explicit_host`.
        - `registry_reference_parses_a_host_port` — `example.com:5000/owner/name:tag`.
        - `registry_reference_parses_a_digest_selector`.
        - `registry_reference_rejects_an_extra_segment`.
        - `registry_reference_rejects_a_bare_digest`.
        - `image_ref_accepts_a_namespaced_tag_and_digest`.
        - `default_registry_url_reads_the_environment` — set and restore `DOSKER_REGISTRY` under a mutex; assert the default when unset.
    - **Verify:** `cargo test -p dosker-image --test reference`

- [x] **Task 2: Namespaced image identity in the store and save name**
    - **Implements:** FR-005, design §Data Flow, design §Error Handling And Edge Cases
    - **Modify `crates/dosker-image/src/oci.rs`**
        - Set `Image.name` from `reference.repository().to_string()` in `load` for both the tagged and digest paths. The index annotation already stores `ImageReference::Display`.
        - Change only the call sites the compiler reports; do not refactor the archive or the read-only commands.
    - **Modify `crates/dosker-cli/src/commands/save.rs`**
        - The default output name uses `reference.repository().to_string().replace('/', "-")` plus the tag.
    - **Modify `crates/dosker-image/tests/oci.rs`**
        - `namespaced_tag_round_trips_through_the_store` — assert `Image.name == "owner/name"` and the reference.
    - **Modify `crates/dosker-image/tests/archive.rs`**
        - `namespaced_tag_round_trips_through_an_archive` — a regression pin for the existing archive round-trip.
    - **Modify `crates/dosker-cli/tests/cli_save_load.rs`**
        - `save_uses_a_safe_default_name_for_a_namespaced_reference`.
    - **Verify:** `cargo test -p dosker-image --test oci --test archive && cargo test -p dosker-cli --test cli_save_load`

- [x] **Task 3: Namespaced base resolution and lock identity**
    - **Implements:** FR-020, design §Data Flow, design §Component Interfaces
    - **Modify `crates/dosker-build/src/build.rs`**
        - `resolve_base` for `FromSource::Image(value)` first tries a bare digest (`ImageDigest::parse`), then a `RegistryReference`. A bare digest keeps the `ImageRef::Digest` path: `BasePin { name: "", tag: "" }` and the resolved digest. For a tag selector, build the local `ImageReference` from the repository and tag. For a digest selector, use the digest. Load from the local store.
        - For a namespaced or host-qualified base that is absent from the local store, return `BuildError::RegistryBaseNotLocal { reference }` with a message that a pull is required.
        - `load_stored_base` builds the `BasePin` with `RegistryReference::identity()` and the selector: `tag` is the tag for a tag selector and empty for a digest selector. The digest is the resolved manifest digest.
        - `base_identity` uses the `RegistryReference` and the resolved digest rather than `ImageReference::name()`.
    - **Modify `crates/dosker-build/src/error.rs`**
        - Add `RegistryBaseNotLocal { reference: String }` with a clear error message.
    - **Modify `crates/dosker-build/tests/build.rs`**
        - `build_from_a_namespaced_local_tag_records_the_full_base_pin` — `owner/name` name and tag.
        - `build_from_a_namespaced_local_digest_pins_an_empty_tag` — `owner/name` name and empty tag.
        - `build_from_a_host_qualified_local_tag_pins_the_host_and_repository` — a stored `owner/name:tag`, `FROM example.com/owner/name:tag`, name `example.com/owner/name`.
        - `build_from_a_bare_digest_keeps_an_empty_pin_name` — the name and tag are empty and the digest is set.
        - `build_rejects_a_missing_namespaced_base_with_a_pull_hint`.
    - **Modify `crates/dosker-build/tests/lock_resolution.rs`**
        - `a_hand_built_namespaced_pin_round_trips` — `resolve_lock` with `BasePin { name: "owner/name", tag, digest }`.
    - **Verify:** `cargo test -p dosker-build --test build --test lock_resolution`

- [x] **Task 4: Registry metadata, namespaces, and root discovery**
    - **Implements:** FR-013, FR-021 (metadata part), design §Component Interfaces
    - **Modify `crates/dosker-build/Cargo.toml`**
        - Add `toml.workspace = true`.
    - **Create `crates/dosker-build/src/registry.rs`**
        - `pub struct RegistryMetadata { schema: u32, version: String, tags: Vec<String>, description: String, homepage: Option<String>, deprecated: Option<String> }` with `#[serde(deny_unknown_fields)]`.
        - `pub fn parse_metadata(path: &Path) -> Result<RegistryMetadata, RegistryError>`.
        - `pub struct Namespaces { schema: u32, owners: BTreeMap<String, NamespaceOwner> }` and `pub struct NamespaceOwner { github: String, proof: String, note: Option<String> }` with `#[serde(deny_unknown_fields)]`.
        - `pub fn parse_namespaces(path: &Path) -> Result<Namespaces, RegistryError>`.
        - `pub enum RegistryError` with `Metadata { path, reason }`, `Namespaces { path, reason }`, `DuplicateTag { tag, first, second }`, `UnknownOwner { owner }`, `UnprovenOwner { owner }`, `BaseNotLocal { reference }`, `BooterNeedsWildcard`, `BooterNoticeUnverifiable { image }`, `Io { path, source }`, and `Doskerfile { path, reason }`.
        - `pub fn registry_root(image_dir: &Path, explicit: Option<&Path>) -> Result<PathBuf, RegistryError>` — explicit, then walk up to a directory containing `images/`, then an error.
        - `pub fn validate_metadata(metadata: &RegistryMetadata, path: &Path) -> Result<(), RegistryError>` — schema 1, tag-shaped `version`, non-empty unique tag-shaped `tags` with at most 32 entries, description 1 to 280, optional `homepage` with an `http`/`https` prefix, optional `deprecated` 1 to 280.
        - `pub fn validate_namespaces(namespaces: &Namespaces, path: &Path, owner: &str) -> Result<(), RegistryError>` — schema 1, parse every key with `Namespace::parse`, the owner exists and is a handle, `github == owner`, `proof` has an `http`/`https` prefix, `note` at most 280.
        - `pub fn scan_tags(root: &Path) -> Result<BTreeMap<String, PathBuf>, RegistryError>` — read every `images/*/*/registry.toml`, parse it, and reject a duplicate tag.
    - **Modify `crates/dosker-build/src/lib.rs`**
        - Add `pub mod registry;` and re-export the public types.
    - **Create `crates/dosker-build/tests/registry_metadata.rs`**
        - `a_valid_registry_toml_parses`.
        - `registry_toml_rejects_an_unknown_field`.
        - `registry_toml_rejects_a_bad_version_or_tag`.
        - `registry_toml_rejects_an_empty_description`.
        - `registry_toml_rejects_more_than_32_tags`.
        - `namespaces_require_a_matching_github_and_proof`.
        - `namespaces_reject_an_invalid_key_or_a_long_note`.
        - `a_duplicate_tag_names_both_images`.
        - `registry_root_walks_up_to_the_images_directory`.
        - `registry_root_honours_an_explicit_path`.
    - **Verify:** `cargo test -p dosker-build --test registry_metadata`

- [x] **Task 5: License coverage engine and the notice rule**
    - **Implements:** FR-026 (coverage part), FR-031 (validation part), design §Error Handling And Edge Cases
    - **Modify `crates/dosker-core/src/license.rs`**
        - `impl LicenseEntry { pub fn is_copyleft(&self) -> bool }` — parse a non-`NONE` SPDX expression and return true when any SPDX license id reports `is_copyleft()`.
        - Use `is_copyleft()` in `LicenseEntry::validate` for the `SOURCE_URL` rule.
    - **Modify `crates/dosker-core/tests/license.rs`**
        - `a_copyleft_expression_reports_copyleft` — `GPL-2.0-only` true, `MIT` false, `NONE` false.
        - `a_contradictory_status_basis_and_spdx_is_rejected` — the existing validation pins this rule for the validator.
    - **Extend `crates/dosker-build/src/registry.rs`**
        - `pub const NOTICE_PATH: &str = r"C:\DOSKER\NOTICE.TXT";`
        - `pub const SYSTEM_TOP_LEVEL: [&str; 2] = ["DOS", "DOSKER"];`
        - `pub const SYSTEM_ROOT_FILES: [&str; 5] = ["KERNEL.SYS", "COMMAND.COM", "CONFIG.SYS", "AUTOEXEC.BAT", "AUTOEXEC.OLD"];`
        - `pub const EXECUTABLE_EXTENSIONS: [&str; 3] = ["EXE", "COM", "BAT"];`
        - `pub fn covers_glob(pattern: &str, path: &str) -> bool` — case-insensitive `*` and `?` matcher.
        - `pub fn covers_directory(pattern: &str, dir: &str) -> bool` — `covers_glob(pattern, dir)` or `pattern` equals `dir` plus `\*`. A file glob does not cover its parent directory.
        - `pub fn coverage_misses(tree: &Tree, entries: &[LicenseEntry], booter: bool) -> Result<Vec<String>, RegistryError>` — `BooterNeedsWildcard` when a booter lacks a `*` cover; otherwise the uncovered top-level directories and root executables.
        - `pub fn requires_notice(entries: &[LicenseEntry]) -> bool` — any `is_copyleft()` or `abandonware`.
        - `pub fn notice_present(tree: &Tree) -> bool` — `tree.get(&DosPath::parse(NOTICE_PATH).expect("valid notice path"))` is a file.
    - **Modify `crates/dosker-build/src/copy.rs`**
        - Allow `C:\DOSKER` only as the ancestor of the exact `NOTICE_PATH` when the planned entry is the notice file. A directory destination (`COPY foo.txt C:\DOSKER\`) and a multi-source destination stay rejected. Do not change `normalize.rs::is_reserved_path`, so `RUN` and `RUN --host` keep the guard.
    - **Create `crates/dosker-build/tests/registry_coverage.rs`**
        - `a_covered_app_directory_passes` — `COVERS=C:\APP\*`.
        - `an_uncovered_app_directory_is_reported`.
        - `a_file_glob_does_not_cover_its_directory`.
        - `a_root_executable_needs_a_cover`.
        - `system_files_are_exempt`.
        - `a_booter_needs_a_wildcard_cover`.
        - `a_copyleft_entry_requires_a_notice_file`.
        - `an_abandonware_entry_requires_a_notice_file`.
        - `a_redistributable_tree_needs_no_notice`.
    - **Modify `crates/dosker-build/tests/copy.rs`**
        - `copy_accepts_the_notice_path` — a `COPY` to `C:\DOSKER\NOTICE.TXT` plans.
        - `copy_rejects_a_dosker_directory_destination` — `COPY foo.txt C:\DOSKER\` fails.
        - `copy_rejects_a_dosker_multi_source_destination` — `COPY a.txt b.txt C:\DOSKER\` fails.
        - `copy_rejects_another_dosker_file` — `C:\DOSKER\OTHER.TXT` fails.
    - **Verify:** `cargo test -p dosker-core --test license && cargo test -p dosker-build --test registry_coverage --test copy`

- [x] **Task 6: `validate_image` and `validate_registry`**
    - **Implements:** FR-004 (registry part), FR-013, FR-021, FR-026 (validator part), FR-031, design §Component Interfaces
    - **Extend `crates/dosker-build/src/registry.rs`**
        - `pub struct ImageValidation { pub image: String, pub version: String, pub tags: Vec<String>, pub license_summary: LicenseSummary, pub checked_paths: Vec<String> }`.
        - `pub enum RegistryTarget { Root(PathBuf), Image { root: PathBuf, image: PathBuf } }`.
        - `pub fn resolve_target(path: &Path, explicit_root: Option<&Path>) -> Result<RegistryTarget, RegistryError>` — a directory containing `images/` is a root; otherwise an `images/<owner>/<name>` directory is an image and the root is discovered.
        - `pub fn validate_target(target: &RegistryTarget, store: &OciStore) -> Result<Vec<ImageValidation>, RegistryError>`:
            - `Root`: parse `namespaces.toml`, validate every image owner, scan tags for duplicates, and validate every `images/*/*/`.
            - `Image`: parse `namespaces.toml`, validate the image owner from the path, scan tags for duplicates, then validate that image. A per-image invocation therefore catches an unknown owner and a duplicate tag.
        - `pub fn validate_image(root: &Path, image_dir: &Path, store: &OciStore) -> Result<ImageValidation, RegistryError>`:
            - Parse `registry.toml` and the `Doskerfile`.
            - Require at least one `LICENSE`; call `validate_licenses`; reject a duplicate name.
            - Open `BuildContext::open(image_dir)`.
            - Resolve the base: `freedos:<v>` uses `freedos_base_licenses()` and an empty tree; a stored local or namespaced image uses `load_config`/`load_tree`; a digest uses the local store; a local path base uses `read_tree`; a booter base has no tree.
            - Any base other than the built-in `freedos` must be in the local store. A missing base, bare or namespaced, returns `BaseNotLocal { reference }` with a pull hint.
            - Apply the instructions after `FROM`: `COPY` through `plan_copy` and `tree.apply_delta`; `WORKDIR` through `resolve_dos_path`; `LICENSE` through `merge_license_entry`; skip `RUN` and `RUN --host`; ignore the other config instructions for tree purposes.
            - Coverage through `coverage_misses`. The notice rule through `requires_notice`/`notice_present`.
            - A booter image has no tree. It must declare `COVERS=*`. A booter with any notice-requiring entry returns `BooterNoticeUnverifiable`.
            - Return the version, tags, summary, and checked paths.
        - `pub fn validate_registry(root: &Path, store: &OciStore) -> Result<Vec<ImageValidation>, RegistryError>` — the `Root` implementation; it validates every image owner through `validate_namespaces`.
    - **Create `crates/dosker-build/tests/registry_validation.rs`**
        - `a_valid_tree_image_validates` — the local fixture base has no licenses; the child's redistributable entry covers `C:\APP`; no notice is needed.
        - `an_image_without_a_license_fails`.
        - `a_contradictory_license_status_fails`.
        - `an_unknown_owner_fails_on_a_per_image_target`.
        - `an_unknown_owner_fails_on_a_root_target`.
        - `a_duplicate_tag_fails_on_a_per_image_target`.
        - `an_uncovered_copy_directory_fails`.
        - `a_copyleft_image_without_a_notice_fails`.
        - `a_booter_image_with_a_wildcard_validates` — redistributable entries only.
        - `a_booter_image_without_a_wildcard_fails`.
        - `a_booter_with_a_copyleft_entry_fails`.
        - `a_namespaced_base_carries_its_entries` — a licensed local base stored as `owner/name:tag`.
        - `a_missing_namespaced_base_reports_a_pull_hint`.
    - **Verify:** `cargo test -p dosker-build --test registry_validation`

- [x] **Task 7: `dosker registry validate` command**
    - **Implements:** FR-021, design §Component Interfaces
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - Add `Command::Registry(RegistryArgs)`.
        - `pub struct RegistryArgs { #[command(subcommand)] pub command: RegistryCommand }`.
        - `pub enum RegistryCommand { Validate(RegistryValidateArgs) }`.
        - `pub struct RegistryValidateArgs { pub path: PathBuf, #[arg(long)] pub registry_root: Option<PathBuf> }`.
        - Add the `Registry` arm to `dispatch`.
    - **Modify the `crates/dosker-cli/src/cli.rs` help test**
        - Check `registry` in the top-level help and `validate` in the rendered `registry` subcommand help.
    - **Modify `crates/dosker-cli/src/commands/mod.rs`**
        - `pub mod registry;`.
    - **Create `crates/dosker-cli/src/commands/registry.rs`**
        - `pub fn execute(args: RegistryArgs, context: &Context) -> Result<u8, CliError>` — call `resolve_target`, `validate_target`, print `validated <image>` per image, and on `RegistryError` print `error: {error}` and return `Ok(1)`.
    - **Modify `crates/dosker-cli/tests/common/mod.rs`**
        - Add a `registry_fixture` helper that writes `namespaces.toml`, `images/<owner>/<name>/Doskerfile`, `registry.toml`, and `NOTICE.TXT`, and stores an empty-tree base with no licenses as `owner/name:tag`.
    - **Create `crates/dosker-cli/tests/cli_registry.rs`**
        - `registry_validate_accepts_a_valid_fixture` — exit 0 and prints `validated`.
        - `registry_validate_rejects_an_uncovered_directory` — exit 1.
        - `registry_validate_rejects_a_duplicate_tag_from_one_image_directory` — exit 1.
        - `registry_validate_requires_a_path` — exit 2.
        - `registry_validate_honours_the_registry_root`.
    - **Verify:** `cargo test -p dosker-cli --lib --test cli_registry`

- [x] **Task 8: Documentation and workflow alignment**
    - **Implements:** FR-005, FR-020, FR-021, FR-026, design §Component Interfaces
    - **Modify `.sdd/specs/2026-11-01-dosker/contracts/doskerfile.md`**
        - Add the namespaced `FROM` forms: `owner/name`, `owner/name:tag`, `host/owner/name:tag`, and `owner/name@sha256:`.
        - Add a short `DOSKER_REGISTRY` note for the later client and the local-store base behavior.
        - Allow the exact `COPY` destination `C:\DOSKER\NOTICE.TXT`; every other path under `C:\DOSKER` stays reserved.
    - **Create `docs/registry.md`**
        - Reference forms, the default registry and override for the later client, `dosker registry validate <path>` with `--registry-root`, the metadata and namespace files, the license coverage and notice rules, the booter notice rule, and the local-store base rule (the remote fetch is a later slice).
    - **Modify `docs/images.md`**
        - Note that a namespaced reference is a valid local tag and how `tag`, `rmi`, `save`, and `load` treat it.
    - **Modify `README.md`**
        - Add one short paragraph that points to `docs/registry.md`.
    - **Modify `.sdd/specs/2026-11-22-doskerfile-registry/assets/workflows/validate.yml`**
        - Bump `DOSKER_VERSION` to the release that ships `registry validate`, and add a comment that the base must be present locally until the network client lands.
    - **Amend `.sdd/specs/2026-11-22-doskerfile-registry/design.md`**
        - State in the validate interface that the base must be present locally until the network client lands, and that the remote `DOSKER_REGISTRY` resolution is a later-slice follow-up. This is the plan's only design change.
    - **Verify:** `rg -n "owner/name" docs/registry.md && rg -n "registry validate" docs/registry.md && rg -n "owner/name" .sdd/specs/2026-11-01-dosker/contracts/doskerfile.md && rg -n "docs/registry.md" README.md && rg -n "namespaced" docs/images.md && rg -n "must be present locally|0\\.1\\.3" .sdd/specs/2026-11-22-doskerfile-registry/assets/workflows/validate.yml && rg -n "present locally" .sdd/specs/2026-11-22-doskerfile-registry/design.md`

## Definition Of Done

The slice is complete when every task is checked and, with the local macOS
prefix, `cargo test -p dosker-core -p dosker-image -p dosker-build -p dosker-cli`
passes. The suite proves that:

- a namespaced reference parses and round-trips through the local store and archive;
- a namespaced base pins `owner/name` or `host/owner/name`, with the tag or digest separate;
- a bare digest `FROM` still resolves;
- `dosker registry validate` accepts a valid fixture and rejects a missing license, a contradictory license, an unknown owner, a duplicate tag, an uncovered directory, a missing notice, and a booter without `COVERS=*`;
- a per-image invocation still catches a registry-wide duplicate tag and an unknown owner;
- a missing namespaced base reports a pull hint; the remote `config.json` fetch is a later slice.

The `validate.yml` workflow in the design spec sets `DOSKER_REGISTRY` and
calls `dosker registry validate "$dir"` for each image. The root discovery
makes the invocation work for a locally present base. The workflow and the
design now state that the base must be present locally until the network
client lands. The pinned `DOSKER_VERSION` must ship `registry validate`.

