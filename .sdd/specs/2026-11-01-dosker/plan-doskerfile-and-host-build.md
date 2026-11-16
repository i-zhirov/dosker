# Doskerfile And Host Build Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-15
- **Status**: Implemented
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Roadmap**: `.sdd/specs/2026-11-01-dosker/roadmap.md` — slice: `doskerfile-and-host-build`
- **Depends on**: `image-store-and-tree`
- **Manual prerequisites**: none
- **Goal**: Deliver the Doskerfile parser, normalized instructions, config-only build state, host `COPY` and `RUN --host` over the canonical tree, the version 1 lock file, build-cache wiring, OCI image persistence, and `dosker build`.

## Constraints

- Supported hosts: Linux x86_64, macOS arm64, Windows x86_64.
- Doskerfile syntax version 1. The file is UTF-8 with LF or CRLF line endings; CRLF is normalized to LF.
- An instruction is one logical line. A trailing backslash continues the instruction on the next physical line and is removed before tokenization.
- A line whose first non-space character is `#` is a comment. Blank lines are ignored. Leading and trailing whitespace is trimmed.
- Instruction names are case-insensitive and uppercase in the normalized form. The first instruction is `FROM`; a Doskerfile has exactly one `FROM`.
- Arguments are whitespace-separated. Double quotes group a token and allow spaces. Inside quotes, `\"` is a literal quote and `\\` is a literal backslash; no other quoted escape exists.
- `$NAME` and `${NAME}` expand from the current `ENV` values. `\$` is a literal `$`. An undefined name is an error. Expansion applies to every instruction except the name itself and `FROM`.
- Placeholder expansion runs on the raw argument text before tokenization. `\$` is consumed as a literal `$` and is not re-expanded. An unquoted expanded value can split into multiple tokens; a quoted placeholder remains one token.
- `ENV` values expand from earlier `ENV` values. A later `ENV` with the same key replaces the value.
- Grammar forms: `FROM`, `COPY`, `RUN`, `RUN --host`, `ENV`, `WORKDIR`, `CMD`, `MACHINE`, `ENGINE`, `ENGINE-CONFIG`, `REQUIRES`, `EXPOSE`, `VOLUME`.
- `FROM` accepts `freedos:<version>`, a local `name:tag`, a local `name@sha256:<digest>`, a local `.img` path, and `booter:<path>`.
- A local `FROM` path is imported into a deterministic local image reference `dosker-build:<source sha256 hex>` and used as-is; no agent layer is added. `FROM freedos:<version>` remains rejected until the FreeDOS builder lands.
- `COPY` takes one or more sources and one destination. Sources are host paths inside the context; globs are unsupported. A source directory copies its contents. Multiple sources require a destination ending in `\` or `/`.
- A `COPY` destination must be on `C:`. Every destination component is normalized to uppercase 8.3. Existing files are replaced; existing directories are merged.
- `COPY` files get the archive attribute. Directories get default attributes. Hidden, system, and read-only attributes cannot be set.
- `COPY` timestamps are fixed to `1980-01-01T00:00:00Z` (`315_532_800` seconds).
- The destination path must not be under `C:\DOSKER`. DOSker owns that directory for the agent, `CONFIG.INI`, and the wrapped `AUTOEXEC`.
- `RUN --host` executes in the staged tree root. The shell is `/bin/sh -c` on Unix and `cmd.exe /C` on Windows.
- `RUN --host` receives `DOSKER_ROOT`, `DOSKER_WORKDIR`, and `DOSKER_ENV_<NAME>` for each image `ENV` value, plus a small host allowlist; other host variables are cleared.
- A `RUN --host` non-zero exit fails the build. The default timeout is 300 seconds; `dosker build --timeout N` overrides it.
- `RUN --host` computes the tree delta after exit. New and changed files become a layer; deletions become whiteouts.
- A `RUN --host` new or renamed path must be valid 8.3. A symlink created outside `C:\DOSKER` is an error. New and changed timestamps are fixed to `1980-01-01T00:00:00Z`.
- Changes under `C:\DOSKER` are discarded and the staged agent files are restored on the next stage.
- Config-only instructions are `ENV`, `WORKDIR`, `CMD`, `MACHINE`, `ENGINE`, `ENGINE-CONFIG`, `REQUIRES`, `EXPOSE`, and `VOLUME`. They update the build config and add a history entry; they create no OCI filesystem layer and never change the head layer.
- `WORKDIR` ensures the directory exists in the in-memory staged tree and records `config.workdir`; it creates no layer. A later filesystem layer under that path materializes the directory in the image tree; a build that ends after `WORKDIR` has the config value but no empty directory in the image tree.
- The normalized instruction has an uppercase name, one space between tokens, expanded placeholders, uppercase 8.3 tree paths, `MACHINE` keys sorted, `REQUIRES` capabilities sorted and deduplicated, and no trailing whitespace.
- Cache key: `sha256(parent_tree_digest || "\n" || build_config_digest || "\n" || normalized_instruction || "\n" || sorted(input_hashes))`.
- `build_config_digest` covers the effective `ENV`, `WORKDIR`, `MACHINE`, `ENGINE`, `REQUIRES`, `EXPOSE`, `VOLUME`, and `CMD` values seen so far. It excludes `ENGINE-CONFIG`, `boot`, `history`, and `agent_version`.
- `input_hashes` is empty for config-only instructions, the context digest plus source file hashes for `COPY`, and the context digest plus staged tree digest for `RUN --host`.
- The build context is the directory that contains the Doskerfile. `.doskerignore` uses gitignore pattern syntax. A source must resolve inside the context after normalization. `..` is rejected. A symlink anywhere in a source path is rejected.
- The context digest is the canonical JSON digest of the sorted non-ignored context entries, excluding the root `Doskerfile`, `Doskerfile.lock`, and `.doskerignore`. A source walk rooted at the context also excludes those three root files.
- The ignore walker uses only `<context>/.doskerignore`; it does not read `.gitignore`, global git excludes, or skip hidden files by default.
- DOS 8.3: a 1 to 8 character stem, then an optional `.` and a 1 to 3 character extension. Allowed characters are `A-Z`, `0-9`, `$ # @ ! % & ' ( ) - { } ~ ^ _`, and the backtick. A name cannot start with a space or a period.
- Lock file: `Doskerfile.lock`, TOML, version 1. `[[base]]` has `name`, `tag`, `digest`; `[engines.<name>]` has `version`, `source`; `[agent]` has `version`.
- A missing lock pin is resolved and written back. A base tag that moved, an engine version mismatch, or an agent version mismatch stops the build unless `--update` is given.
- The lock agent pin is the base image config's `agent_version`. When the base has no agent version, the lock has no agent section.
- Only `--update` is in scope. `--allow-engine-mismatch` is not added.
- The default engine is `qemu`. Machine defaults: `486dx2`, 16 MiB, `vga`, `sb16`, `ne2000`, 2 serial ports, control port 2, `ide`, 115200 baud.
- CLI: `dosker build [-t tag] [--update] [--no-cache] [--timeout N] [CONTEXT]`. The context defaults to `.`; the timeout defaults to 300 seconds.
- CLI exit codes: 0 success, 1 build or image error, 2 usage error.
- CLI output: `FROM <source>`; `Step <i>/<n>: <normalized instruction>`; `Step <i>/<n>: cached` or `Step <i>/<n>: built`; `built <tag> <manifest digest>` or `built <manifest digest>`.
- A non-booter build keeps the base image's `org.dosker.rawdisk.digest` annotation. The tree layers are the build result; the tree-to-FAT projection remains out of scope.
- `ImageConfig.engine_config` is `BTreeMap<String, Vec<String>>`, serialized with `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]`. A line is valid only when its trimmed value is non-empty.
- Images live under `<root>/images`, cache records under `<root>/cache`, and in-progress staging under `<root>/tmp`.
- No transient references in code, identifiers, test names, strings, or comments.

## Technical Context

**Language and toolchain.** Rust 1.98.1, edition 2024, one virtual workspace. The build engine is synchronous and uses the standard library for file I/O.

**Crate changes.**

| Crate | Change |
| --- | --- |
| `dosker-core` | Adds `doskerfile` and `lockfile` modules, `DoskerfileError`, `LockFileError`, and the `toml` dependency. |
| `dosker-image` | Adds `ImageConfig.engine_config`, `CommittedImage`, `OciStore::put_layer`, `layer_descriptor`, `load_layer`, `load_manifest`, `load_config`, `load_tree`, and `commit_image`; adds `ImageError::NoTreeImage`. |
| `dosker-engine` | Adds `EngineSource`. |
| `dosker-engine-qemu` | Adds `engine_source` classification in `discovery`. |
| `dosker-build` (new) | Adds the build context, normalized-instruction renderer, `COPY`, `RUN --host`, config-only state, lock resolution, cache wiring, and build orchestration. |
| `dosker-cli` | Adds the `build` command, `BuildArgs`, `commands/build.rs`, and `CliError::Build`. |
| workspace | Adds the `dosker-build` member and dependency, `toml = "0.8"`, and `ignore = "0.4"`. |

**Build engine home.** The design names the parser, lock file, and domain types under `dosker-core`, and names the Build engine as a component without a crate. This plan keeps the parser and lock file in `dosker-core` and adds `dosker-build` for the engine. The parser emits structured instructions and normalized token text; `dosker-build` finalizes tree paths because `dosker-core` cannot depend on `dosker-image` without a dependency cycle.

**Resolved design seams.**

- **Modified tree bootability before the FAT projection.** A non-booter build keeps the base image's raw disk and manifest annotation. `dosker run` therefore boots the base disk while the built image's tree layers hold the `COPY` and `RUN --host` changes. This is the temporary bridge approved for the imported-image store; the tree-to-FAT projection and bridge removal belong to the later in-DOS `RUN` slice. The boot evidence accepts that the guest sees the base disk; the tree changes are proven by loading the built image's tree.
- **`ENGINE-CONFIG` storage.** `ImageConfig` gains `engine_config: BTreeMap<String, Vec<String>>`. It is the only image-level home for config-only engine lines; it is additive, defaulted, omitted when empty, and does not change the config schema version. Repeated lines for one engine are appended in order.
- **`FROM freedos:<version>`.** The parser accepts it; the build rejects it with an unsupported-base error because the FreeDOS builder is a later slice.
- **A local `.img` or `booter:` path in `FROM`.** The parser accepts both. The build hashes the source, imports it with `OciStore::import` under `dosker-build:<source sha256 hex>` (using `fixed_timestamp()` so the import digest is stable), verifies the annotated raw-disk digest on reuse, and uses the imported image as-is with no agent layer. A stored booter tag remains supported for config-only instructions. `FROM ./path` without a `.img` suffix is a parse error.
- **`RUN` without `--host`.** The parser accepts it; the build rejects it with an error that names `RUN --host`. In-DOS `RUN` belongs to the later slice.
- **Agent injection for an imported image.** A stored or locally imported image is used as-is. The build does not add an agent layer; `agent_version` is preserved from the base config. Agent injection belongs to the later slice.
- **Config-only cache records.** A config-only cache record stores the current head layer digest in `layer_digest` (or the empty string when there is no head). A hit means the config step is known and still applied; a filesystem step stores its real layer digest and updates the head.
- **Filesystem layers.** A filesystem hit or miss calls one state method that applies the layer to the staged tree, pushes the descriptor, updates `head_layer_digest`, and adds a non-empty history entry. Config-only instructions never call it.
- **WORKDIR.** `WORKDIR` is config-only. It ensures the directory exists in the in-memory tree with `fixed_timestamp()` and updates `config.workdir`, but it adds no layer and does not change the head. A later filesystem layer under it materializes the directory; a build ending after `WORKDIR` leaves the config value without an empty directory entry.
- **Engine resolution.** The lock pins the selected engine even for a host-only build. `dosker build` therefore requires the selected engine binary (QEMU by default) so it can record the version. Unit tests inject a resolver; CLI tests use a fake QEMU version script on Unix.
- **Agent pin.** `resolve_lock` receives the base image config's `agent_version`; it compares and writes that value and omits the agent section when it is absent.
- **Only `--update`.** The CLI adds `--update` and does not add `--allow-engine-mismatch`.
- **Runtime application of image settings.** This slice records `ENV`, `WORKDIR`, and `MACHINE` in the image config. Runtime application of `ENV` and `WORKDIR` and the machine profile remains outside this slice; `CMD` is already carried by the runtime.

**Test portability.** The parser, context, copy, cache, lock, and build suites run on all three hosts. Symlink-creation tests are `#[cfg(unix)]`; the Windows matrix compiles the same rejection path. The `RUN --host` file add, replace, delete, environment, non-zero exit, and output tests use the platform shell. The timeout test runs on both hosts: Unix kills the process group; Windows assigns the child to a kill-on-close Job Object and closes it. The homebrew source test is `#[cfg(target_os = "macos")]`. The CLI `build` tests that need a fake QEMU version script are `#[cfg(unix)]`; the `dosker-build` suites carry the cache, lock, and COPY behavior on Windows. The real-engine boot test is gated by `DOSKER_INTEGRATION=1`.

**Pinned build paths.**

```text
<context>/Doskerfile
<context>/Doskerfile.lock
<context>/.doskerignore
<root>/images/...
<root>/cache/<hex>.json
<root>/tmp/dosker-build-<random>
```

**Pinned format details.**

- Normalized token quoting: a token is quoted when it is empty or contains whitespace, `"`, or `\`; inside quotes, `"` and `\` are escaped with a backslash.
- A `COPY` normalized destination is absolute, uppercase 8.3, and ends with `\` when it denotes a directory.
- A `WORKDIR` normalized path is absolute, uppercase 8.3.
- `EXPOSE` always renders the protocol, for example `EXPOSE 80/tcp`.
- `MACHINE` keys are sorted; a duplicate key within one instruction keeps the last value.
- `REQUIRES` renders a comma-separated sorted list with no spaces.
- The lock file ends with a newline.
- A local `FROM` import reference is `dosker-build:<source sha256 hex>`; the base lock pin uses that name and tag and the imported image's manifest digest.
- A digest base lock pin uses the name before `@` and an empty tag.

## Interfaces

Shared definitions the tasks consume:

- `dosker_core::doskerfile`: `Doskerfile`, `Instruction`, `InstructionKind`, `FromSource`, `EngineName`, `DoskerfileError`.
- `dosker_core::lockfile`: `LockFile`, `BasePin`, `EnginePin`, `AgentPin`, `LockFileError`, `LOCK_FILE_NAME`, `LOCK_VERSION`.
- `dosker_image`: `ImageConfig.engine_config`, `CommittedImage`, the new `OciStore` methods, `ImageError::NoTreeImage`.
- `dosker_engine::EngineSource`; `dosker_engine_qemu::discovery::engine_source`.
- `dosker_build`: `BuildError`, `BuildContext`, `ResolvedSource`, `SourceKind`, `SourceFile`, `CopyPlan`, `plan_copy`, `HostRunRequest`, `run_host`, `BuildState`, `BuildCache`, `CacheLookup`, `EngineResolver`, `HostEngineResolver`, `ResolvedEngine`, `resolve_lock`, `write_lock`, `Builder`, `BuildOptions`, `BuildOutcome`, `BuildReporter`.
- `dosker_cli`: `BuildArgs` and `commands::build::execute`.

## Tasks

- [x] **Task 1: OCI build-write and tree-load API**
    - **Implements:** FR-002, FR-012; design §Data Model, §API Contracts, §Build And Runtime Semantics; data-model §Layer, §Image Config.
    - **Add tests in `crates/dosker-image/tests/oci.rs`**
        - `put_layer_writes_a_canonical_layer_blob` — the returned descriptor names the layer media type and the stored tar digest.
        - `load_layer_round_trips_a_layer` — a stored layer reads back with the same sidecar and files.
        - `layer_descriptor_reports_the_blob_size` — the descriptor size equals the blob length.
        - `load_manifest_resolves_a_tag_and_a_digest` — both references return the same manifest.
        - `load_config_returns_the_stored_config` — the config fields survive the round trip.
        - `load_tree_applies_layers_in_order` — a two-layer image produces the second layer's tree.
        - `load_tree_rejects_a_broken_parent_link` — a layer whose parent is wrong returns a layer error.
        - `load_tree_rejects_a_booter_image` — a booter image returns `NoTreeImage`.
        - `commit_image_writes_config_manifest_and_index` — the blobs exist and the tag resolves.
        - `commit_image_replaces_an_existing_tag` — a second commit moves the tag to the new manifest.
        - `commit_image_without_a_tag_leaves_the_index_unchanged` — the manifest is loadable by digest and no tag is added.
        - `commit_image_keeps_the_raw_disk_annotation` — a supplied raw-disk annotation is written and is not a layer.
        - `commit_image_rejects_a_booter_with_tree_layers` — a non-booter layer descriptor fails validation.
    - **Modify `crates/dosker-image/src/oci.rs`**
        - Add `pub struct CommittedImage { pub manifest_digest: String, pub config_digest: String, pub layer_digests: Vec<String> }`.
        - Add `OciStore::put_layer(&self, layer: &Layer) -> Result<Descriptor, ImageError>` — writes the canonical tar and returns a tree-layer descriptor.
        - Add `OciStore::layer_descriptor(&self, digest: &str) -> Result<Descriptor, ImageError>` — returns a tree-layer descriptor whose size is the blob length.
        - Add `OciStore::load_layer(&self, digest: &str) -> Result<Layer, ImageError>` — reads the blob and decodes the canonical tar.
        - Add `OciStore::load_manifest(&self, reference: &ImageRef) -> Result<Manifest, ImageError>`.
        - Add `OciStore::load_config(&self, reference: &ImageRef) -> Result<ImageConfig, ImageError>`.
        - Add `OciStore::load_tree(&self, reference: &ImageRef) -> Result<Tree, ImageError>` — validates each tree layer's parent link and applies it in order.
        - Add `OciStore::commit_image(&self, reference: Option<&ImageReference>, config: &ImageConfig, layers: Vec<Descriptor>, annotations: BTreeMap<String, String>) -> Result<CommittedImage, ImageError>` — validates the config and manifest, writes the config and manifest blobs, and adds or replaces the tag under the index lock when a tag is given.
        - Refactor `load` onto a private `resolve(&self, reference: &ImageRef) -> Result<(Descriptor, Manifest, ImageConfig), ImageError>` without changing its observable output.
    - **Modify `crates/dosker-image/src/error.rs`**
        - Add `NoTreeImage(String)` with message `image {0} has no tree`.
    - **Modify `crates/dosker-image/tests/support/mod.rs`**
        - Add `pub fn sample_boot() -> BootMetadata` — a valid MBR-partitioned boot with dummy geometry, MBR, boot sector, FAT16, and label.
        - Add `pub fn sample_config(boot: BootMetadata) -> ImageConfig` — a valid tree or booter config with default machine values and one history entry. The new `commit_image` tests use this helper, so `tests/oci.rs` contains no duplicated `ImageConfig` literal.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Re-export `CommittedImage`.
    - **Verify:** `cargo test -p dosker-image --test oci`

- [x] **Task 2: Engine-config field on the image config**
    - **Implements:** FR-001; design §Build And Runtime Semantics, §API Contracts; data-model §Image Config.
    - **Add tests in `crates/dosker-image/tests/config.rs`**
        - `engine_config_defaults_to_empty` — a config without the field parses with an empty map.
        - `engine_config_round_trips` — repeated lines for one engine keep their order.
        - `empty_engine_config_is_omitted_from_json` — canonical JSON of a config with no engine lines has no `engine_config` key.
        - `engine_config_rejects_an_unknown_engine` — a key outside `qemu`, `dosbox-x`, and `bochs` fails validation.
        - `engine_config_rejects_an_empty_line` — an empty raw line fails validation.
        - `engine_config_rejects_a_whitespace_line` — a line of spaces fails validation.
        - `booter_config_accepts_engine_config` — a booter config with an engine line validates and round-trips.
    - **Modify `crates/dosker-image/src/config.rs`**
        - Add `pub engine_config: BTreeMap<String, Vec<String>>` with `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]`.
        - Extend `ImageConfig::validate` to reject an unknown engine key and a line whose trimmed value is empty.
    - **Modify `crates/dosker-image/src/oci.rs`**
        - Initialize `engine_config: BTreeMap::new()` in `import_config`.
    - **Modify `crates/dosker-image/tests/support/mod.rs`**
        - Add `engine_config: BTreeMap::new()` to `sample_config`.
    - **Modify `crates/dosker-image/tests/config.rs` and `crates/dosker-image/tests/layer.rs`**
        - Add `engine_config: BTreeMap::new()` to each `ImageConfig` literal.
    - **Verify:** `cargo test -p dosker-image --test config`

- [x] **Task 3: Doskerfile lexer, placeholders, and FROM/COPY/RUN grammar**
    - **Implements:** FR-001; contract §Lexical Rules, §Placeholder Expansion, §Grammar, §FROM, §COPY, §RUN.
    - **Create `crates/dosker-core/tests/doskerfile.rs`**
        - `parse_ignores_comments_and_blank_lines` — a comment and a blank line do not produce instructions.
        - `parse_normalizes_crlf` — CRLF input and LF input produce the same instructions.
        - `parse_strips_a_utf8_bom` — a BOM before `FROM` is ignored.
        - `parse_joins_a_continued_line` — a trailing backslash joins the next physical line.
        - `parse_reports_the_first_line_of_a_continued_instruction` — an error on a continued instruction names its first line.
        - `parse_groups_a_quoted_token` — `"a b"` is one token.
        - `parse_concatenates_quoted_and_unquoted_parts` — `a"b c"d` is one token `ab cd`.
        - `parse_handles_escaped_quote_and_backslash` — `\"` and `\\` are literal inside quotes.
        - `parse_rejects_an_unterminated_quote` — an open quote reports the line.
        - `parse_rejects_an_invalid_quote_escape` — `\n` inside quotes is an error.
        - `parse_keeps_a_literal_dollar` — `\$NAME` stays `$NAME`.
        - `parse_rejects_an_undefined_placeholder` — an unknown name reports the line and name.
        - `from_must_be_the_first_instruction` — a leading `COPY` is an error.
        - `a_second_from_is_rejected` — the second `FROM` reports its line.
        - `from_does_not_expand_placeholders` — `$BASE` remains literal in `FROM`.
        - `from_classifies_a_tag_a_digest_a_path_and_a_booter` — each form produces the matching `FromSource`.
        - `from_classifies_freedos` — `FROM freedos:1.3` produces `Freedos { version: "1.3" }`.
        - `from_accepts_an_absolute_img_path` — `/tmp/base.img` and `C:\base.img` produce `LocalPath`.
        - `from_rejects_a_non_img_path` — `./base` and `C:\base` report a line error.
        - `parse_accepts_a_lowercase_instruction_name` — `from demo:latest` and `copy a b` parse.
        - `copy_requires_a_source_and_destination` — one argument is an error.
        - `run_recognizes_the_host_flag` — the first `--host` sets the host flag and is not part of the command.
        - `run_requires_a_command` — a bare `RUN` is an error.
        - `an_unknown_instruction_names_the_line` — an unknown name reports the line and name.
        - `parse_rejects_invalid_utf8` — invalid bytes report a byte offset.
        - `parse_rejects_an_empty_file` — comments only is an empty Doskerfile.
        - `parse_rejects_a_lone_carriage_return` — a CR without LF reports the line.
    - **Create `crates/dosker-core/src/doskerfile.rs`**
        - `pub const DOSKERFILE_NAME: &str = "Doskerfile";`
        - `pub enum EngineName { Qemu, DosboxX, Bochs }` — `parse(&str) -> Option<Self>`, `as_str(self) -> &'static str`.
        - `pub enum FromSource { Freedos { version: String }, Image(String), LocalPath(String), BooterPath(String) }` — `text(&self) -> String`. A bare local path must end with `.img`; `booter:` accepts any non-empty path; `freedos:` requires a non-empty version.
        - `pub struct Instruction { pub line: usize, pub kind: InstructionKind }`.
        - `pub enum InstructionKind` with parsed values:
            - `From(FromSource)`
            - `Copy { sources: Vec<String>, destination: String }`
            - `Run { host: bool, command: Vec<String> }`
            - `Env { key: String, value: String }`
            - `Workdir { path: String }`
            - `Cmd { command: Vec<String> }`
            - `Machine { settings: BTreeMap<String, String> }`
            - `Engine { name: EngineName }`
            - `EngineConfig { engine: EngineName, raw: String }`
            - `Requires { capabilities: Vec<String> }`
            - `Expose { port: u16, protocol: String }`
            - `Volume { drive: String }`
        - `InstructionKind::name(&self) -> &'static str` returns the uppercase name.
        - `pub struct Doskerfile { pub instructions: Vec<Instruction> }` — `parse_bytes(&[u8]) -> Result<Self, DoskerfileError>`, `parse_str(&str) -> Result<Self, DoskerfileError>`.
        - `pub enum DoskerfileError { Line { line: usize, reason: String }, Empty, Utf8 { offset: usize } }` with displays `Doskerfile line {line}: {reason}`, `Doskerfile is empty`, and `Doskerfile is not valid UTF-8 at byte {offset}`.
        - Internal scanner: physical-line normalization, comment and blank filtering, continuation, and tokenization with quoted escapes.
        - Expansion runs on the raw argument text before tokenization. The left-to-right expander consumes `\$` as a literal `$` and never re-expands it. An unquoted expanded value is tokenized after expansion, so its spaces split it into multiple tokens; a quoted placeholder remains one token.
    - **Modify `crates/dosker-core/src/lib.rs`**
        - Declare `pub mod doskerfile;` and re-export `Doskerfile`, `Instruction`, `InstructionKind`, `FromSource`, `EngineName`, `DoskerfileError`, and `DOSKERFILE_NAME`.
    - **Verify:** `cargo test -p dosker-core --test doskerfile`

- [x] **Task 4: Config-only instruction grammar**
    - **Implements:** FR-001; contract §ENV, §WORKDIR, §CMD, §MACHINE, §ENGINE, §ENGINE-CONFIG, §REQUIRES, §EXPOSE, §VOLUME.
    - **Add tests in `crates/dosker-core/tests/doskerfile.rs`**
        - `parse_accepts_every_config_instruction` — all nine forms parse.
        - `parse_expands_dollar_name` — `$NAME` uses the current env.
        - `parse_expands_braced_name` — `${NAME}` uses the current env.
        - `parse_accepts_a_placeholder_inside_quotes` — `RUN ECHO "$NAME"` expands and stays one token.
        - `unquoted_placeholder_with_space_splits_into_tokens` — `ENV WORDS=one two`, then `RUN ECHO $WORDS` produces command tokens `ECHO`, `one`, `two`.
        - `env_value_keeps_its_spaces` — `ENV GREETING=hello world` stores `hello world`.
        - `env_key_must_match_the_identifier_pattern` — a key with a hyphen fails.
        - `a_later_env_replaces_the_value` — the second value wins.
        - `env_expands_an_earlier_value` — `ENV B=$A` uses the earlier `A`.
        - `workdir_takes_one_path` — a second argument fails.
        - `cmd_takes_a_command` — the command tokens are retained.
        - `machine_keys_are_sorted_and_duplicates_keep_the_last` — the map has one sorted entry per key.
        - `machine_rejects_an_unknown_key` — `MACHINE foo=bar` reports the line.
        - `engine_accepts_the_three_names` — `qemu`, `dosbox-x`, and `bochs` parse case-insensitively.
        - `engine_rejects_an_unknown_name` — `ENGINE virtualbox` reports the line.
        - `engine_config_keeps_the_raw_text` — internal spacing and quotes are retained.
        - `engine_config_requires_a_raw_text` — an engine with no raw text fails.
        - `requires_sorts_and_deduplicates` — `REQUIRES gdbstub,headless,gdbstub` becomes `headless,gdbstub`.
        - `expose_defaults_to_tcp` — `EXPOSE 80` stores `80/tcp`.
        - `expose_rejects_a_non_tcp_protocol` — `EXPOSE 80/udp` reports the line.
        - `volume_rejects_c` — `VOLUME C:` reports the line.
        - `volume_rejects_an_invalid_drive` — `VOLUME 1:` reports the line.
        - `config_instructions_expand_placeholders` — `WORKDIR $DIR` uses the env.
        - `undefined_placeholder_reports_the_line` — a config instruction with an unknown name reports its line.
    - **Modify `crates/dosker-core/src/doskerfile.rs`**
        - Extend the instruction-name dispatch and argument parsers for `ENV`, `WORKDIR`, `CMD`, `MACHINE`, `ENGINE`, `ENGINE-CONFIG`, `REQUIRES`, `EXPOSE`, and `VOLUME`.
        - Maintain the running `ENV` map so later instructions expand earlier values.
    - **Verify:** `cargo test -p dosker-core --test doskerfile`

- [x] **Task 5: Version 1 lock file model**
    - **Implements:** FR-013; design §Error Handling And Edge Cases; data-model §Lock File.
    - **Create `crates/dosker-core/tests/lockfile.rs`**
        - `lock_file_round_trips_through_toml` — version, base, engines, and agent survive serialization.
        - `lock_file_rejects_an_unknown_version` — version 2 returns `UnsupportedVersion`.
        - `lock_file_allows_missing_base_engine_and_agent_sections` — a version-only lock parses with empty sections and no agent.
        - `lock_file_rejects_an_unknown_field` — a typo in a table is an error.
        - `empty_sections_are_omitted_from_toml` — a new lock serializes as `version = 1` plus a newline.
        - `base_pin_keeps_an_empty_tag` — a digest pin with an empty tag round-trips.
        - `engine_pin_keeps_version_and_source` — both fields survive.
    - **Create `crates/dosker-core/src/lockfile.rs`**
        - `pub const LOCK_FILE_NAME: &str = "Doskerfile.lock";`
        - `pub const LOCK_VERSION: u32 = 1;`
        - `pub struct LockFile { pub version: u32, pub base: Vec<BasePin>, pub engines: BTreeMap<String, EnginePin>, pub agent: Option<AgentPin> }` — `new() -> Self`, `from_toml(&str) -> Result<Self, LockFileError>`, `to_toml(&self) -> Result<String, LockFileError>`; `Default`.
        - `pub struct BasePin { pub name: String, pub tag: String, pub digest: String }`.
        - `pub struct EnginePin { pub version: String, pub source: String }`.
        - `pub struct AgentPin { pub version: String }`.
        - `pub enum LockFileError { Invalid { reason: String }, UnsupportedVersion(u32) }` with displays `invalid lock file: {reason}` and `unsupported lock file version: {0}`.
        - Serde attributes: `deny_unknown_fields`, `default` and skip-empty for `base`, `engines`, and `agent`.
    - **Modify `crates/dosker-core/src/lib.rs`**
        - Declare `pub mod lockfile;` and re-export `LockFile`, `BasePin`, `EnginePin`, `AgentPin`, `LockFileError`, `LOCK_FILE_NAME`, and `LOCK_VERSION`.
    - **Modify `crates/dosker-core/Cargo.toml` and the workspace `Cargo.toml`**
        - Add `toml.workspace = true` to `dosker-core`; add `toml = "0.8"` to `[workspace.dependencies]`.
    - **Verify:** `cargo test -p dosker-core --test lockfile`

- [x] **Task 6: Build context, `.doskerignore`, and source validation**
    - **Implements:** FR-003, FR-017; design §Build And Runtime Semantics; contract §Build Context; data-model §Validation Rules.
    - **Create `crates/dosker-build/Cargo.toml`, `crates/dosker-build/src/lib.rs`, `crates/dosker-build/src/error.rs`, and `crates/dosker-build/src/reporter.rs`**
        - Dependencies: `dosker-core`, `dosker-image`, `dosker-engine`, `dosker-engine-qemu`, `dosker-platform`, `serde`, `chrono`, `thiserror`, `ignore`, `sha2`, and `tempfile`; dev-dependencies `serde_json` and `fatfs`.
        - `BuildError` variants: `Doskerfile(#[from] DoskerfileError)`, `LockFile(#[from] LockFileError)`, `Image(#[from] ImageError)`, `Platform(#[from] PlatformError)`, `Io { path: PathBuf, source: io::Error }`, `Reporter(io::Error)`, `Context { reason: String }`, `LockMismatch { message: String }`, `UnsupportedBase { reason: String }`, `Engine { engine: String, reason: String }`, `Instruction { line: usize, instruction: String, reason: String }`, `Cache { message: String }`.
        - `BuildError::Instruction` display: `Doskerfile line {line}: {instruction}: {reason}`.
        - `pub trait BuildReporter { fn line(&mut self, text: &str) -> io::Result<()>; fn output(&mut self, bytes: &[u8]) -> io::Result<()>; }` in `reporter.rs`; re-export it from the crate root.
        - `lib.rs` declares `pub mod context; pub mod error; pub mod reporter;` and re-exports `BuildContext`, `ResolvedSource`, `SourceKind`, `SourceFile`, `BuildError`, and `BuildReporter`.
    - **Create `crates/dosker-build/src/context.rs`**
        - `pub struct BuildContext { root: PathBuf, ignore: Gitignore, digest: String }` — `open(root: &Path) -> Result<Self, BuildError>`, `root(&self) -> &Path`, `digest(&self) -> &str`, `doskerfile_path(&self) -> PathBuf`, `lock_path(&self) -> PathBuf`, `resolve_source(&self, source: &str) -> Result<ResolvedSource, BuildError>`, `read_source_files(&self, source: &ResolvedSource) -> Result<Vec<SourceFile>, BuildError>`, `source_hashes(&self, sources: &[ResolvedSource]) -> Result<Vec<String>, BuildError>`.
        - `pub struct ResolvedSource { pub host_path: PathBuf, pub relative: String, pub kind: SourceKind }`.
        - `pub enum SourceKind { File, Directory }`.
        - `pub struct SourceFile { pub relative: String, pub bytes: Vec<u8> }`.
        - Context digest: canonical JSON of sorted `(path, digest)` entries for non-ignored regular files and symlinks, excluding the root `Doskerfile`, `Doskerfile.lock`, and `.doskerignore`.
        - The ignore matcher is built with `GitignoreBuilder::new(root)` and only `<root>/.doskerignore`. It does not read `.gitignore`, global git excludes, or skip hidden files. The walk uses `read_dir`, so a hidden file is included unless `.doskerignore` excludes it.
        - Source resolution accepts a relative source and an absolute source inside the context. It rejects a missing path, a `..` component, an absolute path outside the context, a symlink in any component, a glob character, and a source excluded by `.doskerignore`.
        - Directory reads skip ignored children, skip the three root metadata files when the walk starts at the context root, include hidden files, and reject a symlink or a non-file/non-directory child.
        - `source_hashes` prefixes the context digest and adds `file:<relative>:<sha256>` and `dir:<relative>` entries; it uses the same root metadata exclusion as `read_source_files`.
    - **Create `crates/dosker-build/tests/context.rs`**
        - `context_digest_is_stable_for_the_same_files`
        - `context_digest_changes_when_a_file_changes`
        - `context_digest_ignores_the_lock_and_ignore_file`
        - `context_digest_skips_ignored_files`
        - `resolve_source_rejects_a_missing_path`
        - `resolve_source_rejects_traversal`
        - `resolve_source_rejects_a_symlink`
        - `resolve_source_rejects_a_path_outside_the_context`
        - `resolve_source_accepts_an_absolute_path_inside_the_context`
        - `resolve_source_rejects_a_glob`
        - `resolve_source_rejects_an_ignored_source`
        - `read_source_files_walks_a_directory_and_skips_ignored_children`
        - `read_source_files_includes_a_hidden_file`
        - `read_source_files_rejects_a_symlink_child`
        - `source_hashes_include_the_context_and_file_content`
        - `source_hashes_skip_root_context_metadata`
        - `context_digest_includes_a_gitignored_file`
    - **Modify workspace `Cargo.toml`**
        - Add `crates/dosker-build` to `members`, add `dosker-build = { path = "crates/dosker-build" }`, and add `ignore = "0.4"`.
    - **Verify:** `cargo test -p dosker-build --test context`

- [x] **Task 7: Normalized instruction rendering and build config digest**
    - **Implements:** FR-001, FR-003; contract §Normalization And Cache Keys.
    - **Create `crates/dosker-build/src/normalize.rs`**
        - `pub fn render_tokens(tokens: &[String]) -> String` — one space between tokens; quote a token when empty or when it contains whitespace, `"`, or `\`; escape `"` and `\` inside quotes.
        - `pub fn resolve_dos_path(workdir: &DosPath, raw: &str) -> Result<DosPath, BuildError>` — absolute paths replace the workdir; relative paths join it; `..` and a non-`C:` drive are errors.
        - `pub fn resolve_copy_destination(workdir: &DosPath, raw: &str) -> Result<(DosPath, bool), BuildError>` — returns the path and whether the destination is a directory from a trailing separator or a root form.
        - `pub fn is_reserved_path(path: &DosPath) -> bool` — true for `DOSKER` and its descendants.
        - `pub fn normalize_instruction(instruction: &Instruction, workdir: &DosPath) -> Result<String, BuildError>` — renders every non-`COPY` instruction; `COPY` is rendered by `plan_copy`.
        - `pub fn build_config_digest(config: &ImageConfig) -> Result<String, BuildError>` — canonical JSON digest of `env`, `workdir`, `machine`, `engine_pin`, `requires`, `exposed_ports`, `volumes`, and `cmd`.
    - **Modify `crates/dosker-build/src/lib.rs`**
        - Declare `pub mod normalize;` and re-export `render_tokens`, `resolve_dos_path`, `resolve_copy_destination`, `is_reserved_path`, `normalize_instruction`, and `build_config_digest`.
    - **Create `crates/dosker-build/tests/normalize.rs`**
        - `render_tokens_quotes_whitespace_and_escapes`
        - `normalize_run_host_keeps_the_host_flag`
        - `normalize_workdir_resolves_a_relative_path`
        - `normalize_machine_sorts_keys`
        - `normalize_requires_is_sorted_and_deduplicated`
        - `normalize_expose_makes_tcp_explicit`
        - `normalize_env_quotes_a_value_with_spaces`
        - `normalize_from_quotes_a_path_with_spaces`
        - `resolve_copy_destination_requires_c_drive`
        - `resolve_copy_destination_rejects_traversal`
        - `resolve_copy_destination_marks_a_trailing_separator`
        - `build_config_digest_changes_for_each_effective_field`
        - `build_config_digest_ignores_history_and_boot`
    - **Verify:** `cargo test -p dosker-build --test normalize`

- [x] **Task 8: COPY over the canonical tree**
    - **Implements:** FR-002, FR-017; contract §COPY; data-model §Tree, §Validation Rules.
    - **Create `crates/dosker-build/src/copy.rs`**
        - `pub struct CopyPlan { pub delta: LayerDelta, pub normalized_sources: Vec<String>, pub normalized_destination: DosPath, pub normalized_instruction: String, pub input_hashes: Vec<String> }`.
        - `pub fn plan_copy(context: &BuildContext, tree: &Tree, workdir: &DosPath, sources: &[String], destination: &str, line: usize) -> Result<CopyPlan, BuildError>` — resolves every source and destination before changing the tree, builds only the added or changed entries, and returns the normalized instruction and cache inputs; every rejection is a line-numbered `BuildError::Instruction` naming `COPY`, the rejected source or destination path, and the reason.
        - Destination rules: `C:` only; relative destinations join the workdir; a trailing separator marks a directory; multiple sources require a directory; a file copied into a directory keeps its normalized file name.
        - Directory rules: contents copy into the destination; existing directories merge; a new destination directory is added; file-versus-directory conflicts are errors.
        - Attribute and timestamp rules: files get archive and `fixed_timestamp()`; directories get default attributes and `fixed_timestamp()`.
        - Rejections: a destination under `DOSKER`, an invalid 8.3 name, a symlink, a traversal path, a source outside the context, an ignored source, and two sources at one destination.
        - `input_hashes`: the `source_hashes` entries, whose first entry is `context:<digest>`.
    - **Modify `crates/dosker-build/src/lib.rs`**
        - Declare `pub mod copy;` and re-export `CopyPlan` and `plan_copy`.
    - **Create `crates/dosker-build/tests/copy.rs`**
        - `copy_file_to_a_file_destination`
        - `copy_file_to_a_directory_destination_uses_the_file_name`
        - `copy_directory_copies_its_contents`
        - `copy_merges_an_existing_directory`
        - `copy_replaces_an_existing_file`
        - `copy_sets_archive_and_the_fixed_timestamp`
        - `copy_rejects_multiple_sources_without_a_directory_destination`
        - `copy_rejects_a_destination_under_dosker`
        - `copy_rejects_an_invalid_source_name`
        - `copy_rejects_an_invalid_destination_name`
        - `copy_rejects_a_symlink`
        - `copy_rejects_a_traversal_path`
        - `copy_rejects_a_source_outside_the_context`
        - `copy_rejects_two_sources_at_one_destination`
        - `copy_into_a_new_directory_adds_the_directory`
        - `copy_dot_skips_root_metadata` — `COPY .` does not include the root `Doskerfile`, `Doskerfile.lock`, or `.doskerignore`.
        - `copy_to_the_root_does_not_add_a_root_entry`
        - `copy_rejects_a_target_under_dosker`
        - `copy_preserves_an_empty_source_directory`
        - `copy_resolves_a_relative_destination_against_the_workdir`
        - `copy_input_hashes_change_for_an_empty_directory`
        - `copy_empty_directory_into_a_new_nested_destination_adds_the_ancestors`
        - `copy_rejects_a_file_at_an_ancestor_path`
    - **Verify:** `cargo test -p dosker-build --test copy`

- [x] **Task 9: RUN --host over a staged tree**
    - **Implements:** FR-002, FR-003, FR-017; contract §RUN --host; design §Build And Runtime Semantics.
    - **Create `crates/dosker-build/src/run_host.rs`**
        - `pub struct HostRunRequest<'a> { pub line: usize, pub command: &'a str, pub workdir: &'a DosPath, pub env: &'a BTreeMap<String, String>, pub timeout: Duration }`.
        - `pub fn run_host(tree: &Tree, request: &HostRunRequest<'_>, work_root: &Path, reporter: &mut dyn BuildReporter) -> Result<LayerDelta, BuildError>` — stages the tree, runs the command, captures output, computes the delta, and returns it without mutating the tree.
        - Stage: a temporary directory under `work_root` represents `C:\`; every tree file and directory is materialized.
        - Shell: `/bin/sh -c` on Unix, `cmd.exe /C` on Windows.
        - Timeout: on Unix, `ProcessSupervisor::prepare_engine` puts the shell in its own process group and `kill_group` kills it. On Windows, create a `JobObject::kill_on_close`, assign the child, and close the job on timeout so the shell tree is terminated. A timeout returns `BuildError::Instruction` naming `RUN --host` and the seconds.
        - Environment: `env_clear()` then `DOSKER_ROOT` (staged root), `DOSKER_WORKDIR` (DOS path), `DOSKER_ENV_<NAME>` for each image env value, and the platform allowlist (`PATH`, `HOME`, `TMPDIR`, `USER`, `LANG`, `LC_ALL`, `LC_CTYPE` on Unix; `PATH`, `PATHEXT`, `SystemRoot`, `WINDIR`, `TEMP`, `TMP`, `USERPROFILE`, `COMSPEC` on Windows).
        - Delta: new and changed files use the archive attribute and `fixed_timestamp()`; new directories use default attributes and `fixed_timestamp()`; missing paths produce the topmost whiteout; kind changes whiteout the old path before adding the new entry.
        - Validation: an invalid 8.3 path, a symlink outside `DOSKER`, a non-file/non-directory entry, or a duplicate normalized path is an error. The `DOSKER` subtree is skipped and never whiteouted.
        - Output: stdout then stderr are written to `reporter.output` after exit; a failure error includes the last 4096 output bytes.
    - **Modify `crates/dosker-build/src/lib.rs`**
        - Declare `pub mod run_host;` and re-export `HostRunRequest` and `run_host`.
    - **Create `crates/dosker-build/tests/run_host.rs`**
        - `run_host_adds_a_file`
        - `run_host_replaces_a_file`
        - `run_host_deletes_a_file_with_a_whiteout`
        - `run_host_deletes_a_directory_with_a_whiteout`
        - `run_host_discards_changes_under_dosker`
        - `run_host_provides_root_workdir_and_env`
        - `run_host_uses_the_staged_tree_as_the_working_directory`
        - `run_host_rejects_a_symlink`
        - `run_host_rejects_an_invalid_8_3_name`
        - `run_host_fails_on_a_nonzero_exit`
        - `run_host_times_out` — uses `sleep 5` on Unix and `ping -n 5 127.0.0.1 > NUL` on Windows with a short timeout.
        - `run_host_streams_output_to_the_reporter`
        - `run_host_returns_no_change_for_an_untouched_tree`
    - **Verify:** `cargo test -p dosker-build --test run_host`

- [x] **Task 10: Config-only instruction state**
    - **Implements:** FR-001, FR-002; contract §ENV, §WORKDIR, §CMD, §MACHINE, §ENGINE, §ENGINE-CONFIG, §REQUIRES, §EXPOSE, §VOLUME, §Normalization And Cache Keys; data-model §MachineProfile, §Validation Rules.
    - **Create `crates/dosker-build/src/state.rs`**
        - `pub struct BuildState { pub config: ImageConfig, pub tree: Option<Tree>, pub layers: Vec<Descriptor>, pub head_layer_digest: Option<String>, pub selected_engine: EngineName }`.
        - `BuildState::new(config: ImageConfig, tree: Option<Tree>, base_layers: &[Descriptor], selected_engine: EngineName) -> Self` — initializes `head_layer_digest` from `base_layers.last()`.
        - `BuildState::parent_tree_digest(&self) -> Result<String, BuildError>` — the tree digest, or the empty string when there is no tree.
        - `BuildState::config_digest(&self) -> Result<String, BuildError>` — delegates to `build_config_digest`.
        - `BuildState::apply_layer(&mut self, layer: Layer, descriptor: Descriptor, normalized: &str, now: DateTime<Utc>) -> Result<(), BuildError>` — applies the layer to the staged tree, pushes the descriptor, sets `head_layer_digest` to the descriptor digest, sets `config.created_by`, and adds a non-empty history entry. It is the only method that changes the head or the layer list.
        - `BuildState::apply_config(&mut self, instruction: &Instruction, normalized: &str, now: DateTime<Utc>) -> Result<(), BuildError>` — applies one config-only instruction, sets `config.created_by`, and records an empty-layer history entry. It never touches `layers` or `head_layer_digest`.
        - `ENV`: insert or replace `config.env`.
        - `WORKDIR`: resolve the path, ensure the directory exists in the staged tree (no layer), set `config.workdir`; a file at the path is an error. A new `TreeDir` uses `Attributes::default()` and `fixed_timestamp()`, never `now`, because `Tree::digest` includes directory timestamps and later cache keys must stay stable. A later filesystem layer under it materializes it in the image tree; a build ending after `WORKDIR` leaves only the config value.
        - `CMD`: replace `config.cmd`.
        - `MACHINE`: merge the settings into `config.machine`, parsing `cpu`, `memory` (an `M` suffix), `video`, `sound`, `net`, `serial`, `control_port`, `disk`, and `baud`, then validate the profile.
        - `ENGINE`: set `config.engine_pin` and `selected_engine`.
        - `ENGINE-CONFIG`: append the raw line to `config.engine_config[engine]` in order.
        - `REQUIRES`: validate each name with `Capability::from_name`, merge, sort, and deduplicate `config.requires`.
        - `EXPOSE`: merge and sort `config.exposed_ports`.
        - `VOLUME`: reject `C:` and a duplicate drive, merge and sort `config.volumes`.
        - A booter config rejects the config-only tree instructions `ENV`, `WORKDIR`, `CMD`, `EXPOSE`, and `VOLUME` with an instruction error; it accepts `MACHINE`, `ENGINE`, `ENGINE-CONFIG`, and `REQUIRES`. `COPY` and `RUN` are rejected by the orchestration task, not here.
    - **Modify `crates/dosker-build/src/lib.rs`**
        - Declare `pub mod state;` and re-export `BuildState`.
    - **Create `crates/dosker-build/tests/config_instructions.rs`**
        - `env_updates_the_config_and_history`
        - `workdir_resolves_and_creates_the_directory`
        - `workdir_adds_history_but_no_layer`
        - `workdir_does_not_change_the_head_layer`
        - `filesystem_layer_updates_the_head_and_tree`
        - `config_only_instructions_do_not_change_the_head_layer`
        - `cmd_replaces_an_earlier_command`
        - `machine_merges_and_validates`
        - `machine_rejects_an_out_of_range_value`
        - `engine_sets_the_pin`
        - `engine_config_appends_lines_in_order`
        - `requires_merges_and_sorts`
        - `requires_rejects_an_unknown_capability`
        - `expose_merges_and_sorts`
        - `volume_rejects_c_and_duplicates`
        - `a_booter_rejects_tree_config_instructions`
        - `a_booter_accepts_machine_engine_engine_config_and_requires`
    - **Verify:** `cargo test -p dosker-build --test config_instructions`

- [x] **Task 11: Lock resolution and engine source**
    - **Implements:** FR-013; design §Error Handling And Edge Cases; data-model §Lock File, §EngineRecord.
    - **Create `crates/dosker-build/src/lock.rs`**
        - `pub struct ResolvedEngine { pub name: String, pub version: String, pub source: String }`.
        - `pub trait EngineResolver { fn resolve(&self, engine: &str) -> Result<ResolvedEngine, BuildError>; }`.
        - `pub struct HostEngineResolver { data_root: PathBuf }` — `new(data_root: PathBuf) -> Self`; `qemu` uses `QemuBinary::discover` and classifies the source with `engine_source`, passing `from_env = true` when `DOSKER_QEMU` is set; other engines return an engine error that names the engine.
        - `pub fn resolve_lock(context_dir: &Path, base: BasePin, agent_version: Option<&str>, engine: &ResolvedEngine, update: bool) -> Result<LockFile, BuildError>` — reads an existing lock and compares the base digest by `(name, tag)`, the engine version by engine name, and the agent version only when `agent_version` is `Some`. A mismatch fails unless `update`; missing pins are added; the returned lock contains the current base, the selected engine, and the agent pin from `agent_version` (or no agent section when it is absent).
        - A tag base pin has the reference name and tag. A digest base pin has the name before `@` and an empty tag. A tag movement check applies only to a matching non-empty tag; a digest base never fails because a tag moved.
        - `pub fn write_lock(context_dir: &Path, lock: &LockFile) -> Result<(), BuildError>` — writes `<context>/Doskerfile.lock` atomically with a trailing newline.
    - **Modify `crates/dosker-build/src/lib.rs`**
        - Declare `pub mod lock;` and re-export `EngineResolver`, `HostEngineResolver`, `ResolvedEngine`, `resolve_lock`, and `write_lock`.
    - **Create `crates/dosker-engine/src/source.rs`**
        - `pub enum EngineSource { Env, Managed, Homebrew, Path }` — `as_str(self) -> &'static str`, `parse(&str) -> Option<Self>`.
    - **Modify `crates/dosker-engine/src/lib.rs` and `crates/dosker-engine-qemu/src/discovery.rs`**
        - `pub fn engine_source(path: &Path, data_root: &Path, from_env: bool) -> EngineSource` — `Env` for an override, `Managed` under `<data_root>/engines`, `Homebrew` under the macOS Homebrew prefixes, otherwise `Path`.
        - Re-export `EngineSource` and `engine_source`.
    - **Create `crates/dosker-build/tests/lock_resolution.rs`**
        - `a_missing_lock_file_resolves_all_pins`
        - `matching_pins_stay_unchanged`
        - `base_tag_movement_fails_without_update`
        - `base_tag_movement_updates_with_update`
        - `engine_version_mismatch_fails_without_update`
        - `engine_version_mismatch_updates_with_update`
        - `agent_version_mismatch_fails_without_update`
        - `missing_agent_pin_is_written`
        - `a_base_without_an_agent_version_omits_the_agent_pin`
        - `a_base_without_an_agent_version_removes_a_stale_agent_pin`
        - `digest_base_ignores_a_tag_pin_for_the_same_name`
        - `the_lock_file_ends_with_a_newline`
        - `the_lock_file_round_trips_through_disk`
    - **Add tests in `crates/dosker-engine-qemu/tests/discovery.rs`**
        - `source_reports_an_environment_override`
        - `source_reports_a_managed_binary`
        - `source_reports_a_homebrew_binary_on_macos` — `#[cfg(target_os = "macos")]`.
        - `source_reports_a_plain_path`
    - **Verify:** `cargo test -p dosker-build --test lock_resolution` and `cargo test -p dosker-engine-qemu --test discovery`

- [x] **Task 12: Build-cache wiring**
    - **Implements:** FR-003; contract §Normalization And Cache Keys; data-model §Build Cache.
    - **Create `crates/dosker-build/src/cache.rs`**
        - `pub enum CacheLookup { HitLayer { layer: Layer, descriptor: Descriptor }, HitConfig, Miss }`.
        - `pub struct BuildCache<'a> { store: &'a OciStore, cache: &'a CacheStore, no_cache: bool }` — `new(store: &'a OciStore, cache: &'a CacheStore, no_cache: bool) -> Self`.
        - `BuildCache::lookup(&self, key: &str, normalized: &str, previous: Option<&str>, tree_step: bool) -> Result<CacheLookup, BuildError>`:
            - `no_cache` returns `Miss`.
            - A missing record returns `Miss`.
            - A corrupt record is removed and returns `Miss`.
            - A record whose instruction differs is removed and returns `Miss`.
            - A filesystem record loads the layer, verifies its digest and parent, and returns `HitLayer`; a missing or invalid blob is removed and returns `Miss`.
            - A config-only record verifies the head blob when non-empty and returns `HitConfig`.
        - `BuildCache::store(&self, key: &str, layer_digest: &str, normalized: &str, now: DateTime<Utc>) -> Result<(), BuildError>` — writes a version 1 `CacheRecord`.
    - **Modify `crates/dosker-build/src/lib.rs`**
        - Declare `pub mod cache;` and re-export `BuildCache` and `CacheLookup`.
    - **Create `crates/dosker-build/tests/cache_wiring.rs`**
        - `a_matching_cache_record_returns_the_layer`
        - `a_missing_layer_blob_is_a_miss`
        - `a_layer_parent_mismatch_is_a_miss`
        - `a_corrupt_record_is_removed_and_rebuilt`
        - `a_different_instruction_is_a_miss`
        - `no_cache_bypasses_lookup`
        - `config_only_records_use_the_head_layer`
        - `storing_a_record_writes_a_valid_entry`
    - **Verify:** `cargo test -p dosker-build --test cache_wiring`

- [x] **Task 13: Build orchestration and OCI commit**
    - **Implements:** FR-001, FR-002, FR-003, FR-013, FR-017; design §Build And Runtime Semantics, §Data Flow, §Error Handling And Edge Cases; contract §FROM, §Normalization And Cache Keys.
    - **Create `crates/dosker-build/src/build.rs`**
        - Use the `BuildReporter` trait defined in `reporter.rs`.
        - `pub struct BuildOptions { pub tag: Option<ImageReference>, pub update_lock: bool, pub no_cache: bool, pub timeout: Duration, pub now: DateTime<Utc> }`.
        - `pub struct BuildOutcome { pub reference: Option<ImageReference>, pub manifest_digest: String, pub config_digest: String, pub steps: usize, pub cached_steps: usize }`.
        - `pub struct Builder { root: PathBuf, store: OciStore, cache: CacheStore, resolver: Box<dyn EngineResolver> }` — `new(root: PathBuf) -> Self` uses `HostEngineResolver`; `with_resolver(root: PathBuf, resolver: Box<dyn EngineResolver>) -> Self`; `build(&self, context_dir: &Path, options: &BuildOptions, reporter: &mut dyn BuildReporter) -> Result<BuildOutcome, BuildError>`.
        - `build` steps: open the context, parse the Doskerfile, resolve and load the base, select the last `ENGINE` or the base config engine pin or `qemu`, resolve the engine, call `resolve_lock` with `base.config.agent_version.as_deref()`, print `FROM <source>`, apply the instruction list, commit the image, and write the lock.
        - Base resolution: a stored tag or `name@sha256` loads the manifest, config, and tree; a non-booter keeps `base.layers` and the raw-disk annotation; a stored booter tag loads the booter config and one booter layer. A relative local `.img` or `booter:<path>` resolves against the context root; an absolute path is used as-is. The source is hashed into `source_digest`, and the deterministic reference is `dosker-build:<source_digest hex>`. Load the reference if it exists; otherwise call `OciStore::import` with `fixed_timestamp()`. If `import` returns `AlreadyExists`, load the existing reference. For a disk import, verify `manifest.raw_disk_digest() == Some(source_digest)`; a mismatch is an error. A booter import has no raw-disk annotation, so verify only that it is a booter image. The imported image is used as-is. `freedos:<version>` returns `UnsupportedBase`; a missing local path returns an I/O error that names the path.
        - Instruction loop: print `Step <i>/<n>: <normalized instruction>`; compute `parent_tree_digest` and `config_digest`; compute the cache key; look it up unless `--no-cache`. A `COPY` computes its `CopyPlan` first and uses the plan's normalized instruction and input hashes. A `RUN --host` uses the rendered command and the context plus staged tree hashes. A config-only instruction uses its normalized instruction and empty input hashes. On a filesystem hit, call `BuildState::apply_layer` with the cached layer. On a config hit, call `BuildState::apply_config`. On a miss, run the instruction in this order: compute the delta, construct the `Layer`, call `BuildState::apply_layer`/`apply_config`, and only after that succeeds store the cache record. Print `Step <i>/<n>: cached|built`.
        - `COPY`: call `plan_copy` with the instruction line before computing the cache key; on a hit apply the cached layer; on a miss create `Layer::from_delta` from the plan delta with the previous layer digest, `put_layer`, call `BuildState::apply_layer`, then store the cache record.
        - `RUN --host`: build the rendered command, use `context:<digest>` and `staged:<tree digest>` as input hashes; on a hit apply the cached layer; on a miss run `run_host` with `<root>/tmp` as the work root (created on demand), create the layer, `put_layer`, call `BuildState::apply_layer`, then store the cache record.
        - `RUN` without `--host`: return an instruction error that names `RUN --host`.
        - On a booter base, `COPY` and `RUN --host` return an instruction error before planning or execution.
        - Config-only: use empty input hashes; on a hit call `BuildState::apply_config`; on a miss call `BuildState::apply_config`, then store the cache record. Config-only creates no layer and changes no head.
        - History: the base history is preserved; every new instruction adds one entry; filesystem entries have `empty_layer: false`; config-only entries have `empty_layer: true`; `config.created_by` and the history `created_by` are the normalized instruction, so `created_by` ends as the last normalized instruction.
        - Commit: `layers = base.layers + state.layers`; carry `ANNOTATION_RAW_DISK_DIGEST` from the base; call `OciStore::commit_image`; write the lock only after a successful commit. The lock agent pin is `base.config.agent_version.as_deref()`.
        - A failed `plan_copy`, `run_host`, layer construction, `apply_layer`, or `apply_config` writes no cache record for that instruction and no layer; earlier cache entries remain. No image or lock is written on any failure.
    - **Create `crates/dosker-build/tests/support/mod.rs`**
        - `pub fn sample_boot() -> BootMetadata` — a valid MBR-partitioned boot with dummy geometry, MBR, boot sector, FAT16, and label.
        - `pub fn sample_config(boot: BootMetadata, agent_version: Option<&str>) -> ImageConfig` — a valid tree or booter config with default machine values, one history entry, the given agent version, and an empty `engine_config`.
        - `pub fn store_tree_base(root: &Path, reference: &str, tree: &Tree, raw_disk: Option<&[u8]>, agent_version: Option<&str>) -> String` — creates an `OciStore`, puts one base layer from `tree.to_delta()`, puts the raw disk blob when given, writes the `org.dosker.rawdisk.digest` annotation, commits the base image, and returns the manifest digest.
        - `pub fn store_booter_base(root: &Path, reference: &str, bytes: &[u8]) -> String` — puts the booter blob, commits a booter config with one booter layer, and returns the manifest digest.
        - `pub fn write_fat_image(path: &Path)` — formats a 4 MiB superfloppy FAT16 image with an `AUTOEXEC.BAT` for a local disk import test.
        - `pub fn empty_tree() -> Tree`.
    - **Create `crates/dosker-build/tests/build.rs`**
        - `build_from_a_stored_tag_applies_copy_run_host_and_config`
        - `build_from_a_digest_reference_pins_the_digest`
        - `build_from_a_booter_tag_changes_only_config`
        - `build_imports_a_local_disk_image`
        - `build_second_run_from_a_local_import_reuses_the_image`
        - `build_imports_a_local_booter_image`
        - `build_rejects_a_missing_local_image`
        - `build_rejects_a_freedos_base`
        - `build_rejects_an_in_image_run`
        - `build_rejects_a_booter_copy_and_run_host`
        - `build_writes_the_lock_file`
        - `build_writes_the_image_manifest_and_layers`
        - `build_keeps_the_base_raw_disk_annotation`
        - `build_records_a_history_entry_for_each_instruction`
        - `config_only_history_entries_are_empty_layers`
        - `build_sets_created_by_to_the_last_instruction`
        - `workdir_directory_appears_after_a_later_copy`
        - `a_build_ending_after_workdir_keeps_the_config_only`
        - `workdir_then_copy_second_build_reports_a_cache_hit`
        - `build_second_run_reports_cached_for_every_step`
        - `copy_dot_second_build_reports_a_cache_hit`
        - `no_cache_rebuilds_every_step`
        - `env_change_invalidates_later_steps`
        - `a_failed_instruction_writes_no_image_or_lock`
        - `build_without_the_engine_fails`
        - `build_reports_progress_lines`
    - **Modify `crates/dosker-build/src/lib.rs`**
        - Declare `pub mod build;` and re-export `BuildOptions`, `BuildOutcome`, and `Builder`.
    - **Verify:** `cargo test -p dosker-build --test build`

- [x] **Task 14: CLI `dosker build`**
    - **Implements:** FR-001, FR-002, FR-013; design §CLI, §Build And Runtime Semantics.
    - **Modify `crates/dosker-cli/src/cli.rs`**
        - Add `Build(BuildArgs)` to `Command`.
        - `pub struct BuildArgs { #[arg(short = 't', long = "tag")] pub tag: Option<String>, #[arg(long = "update")] pub update: bool, #[arg(long = "no-cache")] pub no_cache: bool, #[arg(long = "timeout", default_value_t = 300)] pub timeout: u64, #[arg(default_value = ".")] pub context: PathBuf }`.
        - Dispatch `Build` through `Context::load()` and `commands::build::execute`.
    - **Create `crates/dosker-cli/src/commands/build.rs`**
        - `pub fn execute(args: BuildArgs, context: &mut Context, now: DateTime<Utc>) -> Result<u8, CliError>` — parses the tag with `ImageReference::parse`, builds `BuildOptions` with `Duration::from_secs(args.timeout)`, runs `Builder::new(context.paths.root().to_path_buf())` with a `ConsoleReporter`, and prints `built <tag> <manifest digest>` or `built <manifest digest>`. The command has no `--allow-engine-mismatch`; only `--update` is available.
        - `struct ConsoleReporter<'a>` implements `BuildReporter` over `Console::line` and `Console::bytes`.
    - **Modify `crates/dosker-cli/src/commands/mod.rs`**
        - Declare `pub mod build;`.
    - **Modify `crates/dosker-cli/src/error.rs`**
        - Add `Build(#[from] BuildError)`; it uses the default exit code 1.
    - **Modify `crates/dosker-cli/Cargo.toml`**
        - Add `dosker-build.workspace = true`.
    - **Create `crates/dosker-cli/tests/cli_build.rs`**
        - `build_prints_a_tag_and_the_manifest_digest`
        - `build_without_a_tag_prints_the_manifest_digest`
        - `build_timeout_overrides_the_default`
        - `build_lists_the_tag_in_images`
        - `build_writes_a_version_one_lock_file`
        - `build_reports_cached_steps_on_a_second_run`
        - `build_env_change_reports_later_steps_built`
        - `build_rejects_a_bad_8_3_name_with_the_path` — asserts the rejected source or destination path in stderr.
        - `build_rejects_a_symlink_with_the_path` — asserts the rejected source path in stderr.
        - `build_rejects_a_traversal_path_with_the_path` — asserts the rejected path in stderr.
        - `build_rejects_a_bad_instruction_with_the_line`
        - `build_lock_mismatch_stops_and_update_refreshes`
        - `build_no_cache_rebuilds`
        - `build_without_qemu_fails`
        - `build_prints_run_host_output`
    - **Verify:** `cargo test -p dosker-cli --test cli_build`

- [x] **Task 15: Built-image boot integration**
    - **Implements:** FR-002, FR-013; design §Success Criteria (SC-002), §Testing Strategy.
    - **Create `crates/dosker-cli/tests/qemu_build.rs`**
        - `built_image_boots_and_answers_exec` — gated on `DOSKER_INTEGRATION=1`; imports the prepared image as `base:latest`, writes a Doskerfile with `FROM`, `COPY`, `RUN --host`, `ENV`, `WORKDIR`, `CMD`, and `MACHINE`, runs `dosker build -t built:latest`, then:
            - loads the built image's tree and asserts it contains the `COPY` destination and the `RUN --host` created path;
            - loads the built image's config and asserts the expected `env`, `workdir`, `machine`, and `cmd` values;
            - loads the built manifest and asserts the raw-disk annotation equals the base image's annotation;
            - starts the built image with `dosker run -d built:latest`, verifies `dosker exec` returns the expected output and status, and stops it.
        - The guest sees only the retained base disk; the `COPY` and `RUN --host` paths are proven in the built tree, not inside the guest.
    - **Verify:** `DOSKER_INTEGRATION=1 DOSKER_QEMU_IMAGE=<prepared image> cargo test -p dosker-cli --test qemu_build`

- [x] **Task 16: Document `engine_config` in the data model**
    - **Implements:** design §Data Model; data-model §Image Config.
    - **Modify `.sdd/specs/2026-11-01-dosker/data-model.md`**
        - Add `engine_config` to the Image Config table with type `map of string to list of string` and notes `Ordered raw ENGINE-CONFIG lines by engine; omitted when empty.`
        - Update the Lock File Rules to state that only `--update` is in scope for refreshing pins and `--allow-engine-mismatch` is future work.
    - **Verify:** `rg -n "engine_config|allow-engine-mismatch" .sdd/specs/2026-11-01-dosker/data-model.md`

- [x] **Task 17: Workspace verification**
    - **Implements:** design §Testing Strategy, §Success Criteria.
    - **Modify** no files; run the full gate.
    - **Verify:** `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --features dosker-cli/mock-engine,dosker-cli/qemu-engine -- -D warnings` and `cargo test --workspace --features dosker-cli/mock-engine` and `cargo test -p dosker-engine-qemu`

## Acceptance Evidence

| Roadmap acceptance clause | Proved by |
| --- | --- |
| Parser covers every instruction and placeholder rule | Task 3 and Task 4 parser tests |
| Normalized instruction form | Task 7 normalization tests; Task 8 `COPY` normalization |
| `FROM` a stored imported image, `COPY`, `RUN --host`, `ENV`, `WORKDIR`, `CMD`, and `MACHINE` build an image | Task 13 `build_from_a_stored_tag_applies_copy_run_host_and_config`; Task 14 CLI build tests |
| A local `.img` and `booter:` path import and build as-is | Task 13 `build_imports_a_local_disk_image`, `build_imports_a_local_booter_image`, `build_second_run_from_a_local_import_reuses_the_image`; Task 3 `from_accepts_an_absolute_img_path`, `from_classifies_a_tag_a_digest_a_path_and_a_booter` |
| The built image boots with Slice 3 | Task 15 `built_image_boots_and_answers_exec`; the boot uses the retained base raw disk while the built tree holds the changes, so the guest sees only the base disk |
| A second build reports a cache hit for every step and boots no VM | Task 13 `build_second_run_reports_cached_for_every_step`; Task 14 `build_reports_cached_steps_on_a_second_run`; no VM exists in this slice |
| An `ENV` change invalidates later steps | Task 13 `env_change_invalidates_later_steps`; Task 14 `build_env_change_reports_later_steps_built` |
| A bad 8.3 name, a symlink, or a `..` path fails and names the path | Task 8 `copy_rejects_an_invalid_source_name`, `copy_rejects_an_invalid_destination_name`, `copy_rejects_a_symlink`, `copy_rejects_a_traversal_path`; Task 9 `run_host_rejects_a_symlink`, `run_host_rejects_an_invalid_8_3_name`; Task 14 `build_rejects_a_bad_8_3_name_with_the_path`, `build_rejects_a_symlink_with_the_path`, `build_rejects_a_traversal_path_with_the_path` |
| A build writes a version 1 lock file | Task 5 lock tests; Task 11 lock resolution tests; Task 13 `build_writes_the_lock_file`; Task 14 `build_writes_a_version_one_lock_file` |
| A changed lock pin stops the build and `--update` refreshes it | Task 11 `base_tag_movement_fails_without_update`, `base_tag_movement_updates_with_update`, `engine_version_mismatch_fails_without_update`, `engine_version_mismatch_updates_with_update`; Task 14 `build_lock_mismatch_stops_and_update_refreshes` |
| `COPY` merges directories, replaces files, preserves empty directories, sets archive and fixed timestamps, and creates a layer | Task 8 copy tests; Task 13 `build_writes_the_image_manifest_and_layers` |
| `RUN --host` uses the staged tree, controlled environment, delta/whiteouts, fixed timestamps, reserved path, and cache input | Task 9 run-host tests; Task 13 `build_from_a_stored_tag_applies_copy_run_host_and_config` |
| Config-only instructions update `ImageConfig` and history with no layer | Task 10 config-instruction tests; Task 13 `build_records_a_history_entry_for_each_instruction`, `config_only_history_entries_are_empty_layers` |
| `WORKDIR` creates no layer and only materializes through a later filesystem layer | Task 10 `workdir_adds_history_but_no_layer`, `workdir_does_not_change_the_head_layer`; Task 13 `workdir_directory_appears_after_a_later_copy`, `a_build_ending_after_workdir_keeps_the_config_only`, `workdir_then_copy_second_build_reports_a_cache_hit` |
| A `COPY .` build stays cacheable after the lock file is written | Task 6 `source_hashes_skip_root_context_metadata`; Task 13 `copy_dot_second_build_reports_a_cache_hit` |
| The last instruction sets `created_by` | Task 13 `build_sets_created_by_to_the_last_instruction` |
| `ENGINE-CONFIG` is persisted | Task 2 config tests; Task 10 `engine_config_appends_lines_in_order` |
| Cache key uses parent tree digest, config digest, normalized instruction, and input hashes | Task 7 `build_config_digest_*`; Task 12 cache tests; Task 13 cache tests |
| A built image is persisted through the OCI store | Task 1 OCI tests; Task 13 `build_writes_the_image_manifest_and_layers` |
| `FROM freedos` and in-image `RUN` fail clearly | Task 13 `build_rejects_a_freedos_base`, `build_rejects_an_in_image_run` |
| An imported image, including a local path import, is used as-is with no agent injection | Task 13 preserves the base `agent_version` and layers; stated in Out Of Scope |
| In-scope requirements are covered | FR-001: Tasks 3, 4, 10, 13, 14; FR-002: Tasks 1, 7, 8, 9, 13; FR-003: Tasks 6, 7, 11, 12, 13; FR-013: Tasks 5, 11, 13, 14; FR-017: Tasks 6, 7, 8, 9 |

## Out Of Scope

- In-DOS `RUN` and the tree-to-FAT projection. The later in-DOS `RUN` slice owns both.
- The FreeDOS base builder and `FROM freedos:<version>` execution.
- The import layer and agent injection for an arbitrary imported or locally imported image. Both are used as-is.
- `--allow-engine-mismatch`. Only `--update` is in scope.
- Runtime application of image `ENV`, `WORKDIR`, and `MACHINE`. The image config records them; the runtime carries `CMD` only.
- `EXPOSE` publishing, `VOLUME` creation, NAT, bind mounts, and development mode.
- DOSBox-X and Bochs adapters. `ENGINE` accepts the names; the build rejects an engine without a resolver until its adapter slice.
- Multi-stage builds, `ARG`, `ENTRYPOINT`, `HEALTHCHECK`, `commit`, long file names, a registry, `save`, `load`, and `system prune`.
- Removing the raw-disk bridge. The bridge remains until the tree-to-FAT projection.

## Risks

- **The booted guest sees the base disk.** The built image's tree is correct and loadable, but the runtime boots the retained base raw disk until the projection lands. The integration test proves boot and `exec`; it asserts the tree paths and does not prove the `COPY` file inside the guest. The projection slice must replace the bridge and add guest-visible evidence.
- **A local import is used as-is.** A local `.img` import does not add an agent. The built image boots and answers `exec` only when the source image already contains a compatible agent. Agent injection remains later work.
- **The lock has no agent section for a base without an agent version.** Slice 5 imports leave `agent_version` empty. The lock records what the base config provides and omits the agent section when it is absent; a later base with a version pins and checks it.
- **`COPY .` includes every other non-ignored root entry.** The root `Doskerfile`, `Doskerfile.lock`, and `.doskerignore` are skipped by both the context digest and the source walk, so the second build stays cacheable. Every other non-ignored root entry is copied and must still have a valid 8.3 name; keep the remaining root entries 8.3 or add `.doskerignore` patterns.
- **Engine resolution is required for a host-only build.** The lock pins the selected engine version, so a missing QEMU fails the build even when no VM would run. Unit tests inject a resolver; the CLI requires the engine binary.
- **Host shell differences.** `RUN --host` uses `/bin/sh` and `cmd.exe`; quoting, newline handling, and available commands differ. Tests gate shell-specific cases and pin the environment contract.
- **Captured rather than live host output.** `RUN --host` output is written to the reporter after exit. A long command shows output late; the design does not require streaming for host commands.
- **Config-only cache records use the head layer digest.** The record is a progress marker, not a new layer. A cache reader must not treat a config-only record as a filesystem layer; the instruction and key identify it.
- **`ENGINE-CONFIG` is recorded but not applied by the runtime.** The adapter consumes it in a later slice; this slice proves persistence and ordering only.
- **Raw disk and tree double storage.** A built image keeps the base raw disk and its base tree layers plus the new tree layers, so it uses more space than the tree alone until the bridge is removed.
- **Cache invalidation breadth.** The context digest covers every non-ignored context file, so an unrelated file change invalidates `COPY` and `RUN --host` steps. This follows the contract but may be coarse.

## Deviations

- **`ImageConfig.engine_config`.** The data model's config table does not list the field. It is the only image-level place to persist the config-only `ENGINE-CONFIG` lines. It is additive, defaults to empty, is omitted from canonical JSON when empty, and keeps the image config schema version at 1. Existing configs parse unchanged. Task 16 records the field in the data model.
- **Raw RUN/CMD command text.** `RUN`, `RUN --host`, and `CMD` preserve the expanded argument text, including quotes and backslashes, instead of re-rendering tokens. This keeps host globs and Windows DOS paths intact and makes the normalized command and cache key match the command that runs.
- **COPY empty directories.** Directory sources preserve empty subdirectories; COPY input hashes include directory markers so an empty-directory-only change invalidates the cache. Directory-only deltas add their ancestor directories.
- **Booter cache records.** Config-only cache records validate an opaque booter head blob by blob digest rather than decoding it as a layer, so a second booter build can report cached steps.
- **Local base imports.** A local `.img` or `booter:` path is imported under a deterministic content-derived reference and used as-is, without agent injection.
- No other deviation from the design's image shape or instruction semantics is introduced.
