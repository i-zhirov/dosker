# DOSkerfile License Core Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-23
- **Status**: Implemented
- **Design**: /Users/iliazhirov/.local/share/opencode/worktree/870331d51b6f42d36c9762bfa5157767195393e1/doskerfile-registry/.sdd/specs/2026-11-22-doskerfile-registry/design.md
- **Roadmap**: /Users/iliazhirov/.local/share/opencode/worktree/870331d51b6f42d36c9762bfa5157767195393e1/doskerfile-registry/.sdd/specs/2026-11-22-doskerfile-registry/roadmap.md — slice: license-core
- **Depends on**: —
- **Manual prerequisites**: —
- **Goal**: Add the license entry model, the repeatable `LICENSE` Doskerfile instruction, the additive image-config field, build inheritance, the built-in and imported entries, and the local-build warning, with no registry or network work.

## Constraints

- `LICENSE` is repeatable, config-only, allowed after `FROM`, and allowed for booter images.
- Keys are `NAME`, `SPDX`, `STATUS`, `HOLDER`, `BASIS`, `EVIDENCE`, `LICENSE_URL`, `COVERS`, `SOURCE_URL`, and `NOTICE`. `NAME`, `SPDX`, and `STATUS` are required.
- `status` is `redistributable` or `abandonware`.
- `basis` is `open-source-license`, `freeware-release`, `public-domain`, `rights-holder-statement`, or `good-faith`.
- `abandonware` requires `SPDX=NONE`, `basis=good-faith`, and `evidence`.
- `open-source-license` requires a real SPDX expression.
- `good-faith` is valid only with `abandonware`.
- `redistributable` with `SPDX=NONE` requires a freeware, public-domain, or rights-holder basis and evidence.
- A copyleft SPDX expression requires `source_url`.
- SPDX IDs are checked against the SPDX license list.
- `LicenseRef-*` requires `license_url`.
- The summary is `abandonware` if every entry is abandonware, `redistributable` if every entry is redistributable, and `mixed` otherwise.
- `ImageConfig.licenses: Vec<LicenseEntry>` is additive and has `#[serde(default, skip_serializing_if = "Vec::is_empty")]`.
- An image without entries serializes to its old canonical JSON and keeps its old config digest. `schema_version` stays 1.
- Inheritance starts from the resolved base entries and appends the Doskerfile entries. An identical duplicate `name` is deduplicated. A conflicting duplicate is an error.
- The built-in `freedos:1.3` base carries a FreeDOS entry and the DOSKER agent entry. An imported disk image carries the DOSKER agent entry.
- A local build without any `LICENSE` succeeds but prints a warning. Registry enforcement is a later slice.
- No registry, reference grammar, validator, HTTP client, or site work is in this plan.

### Plan Decisions

- `COVERS` is one unquoted `key=value` token whose value is a comma-separated list of DOS path globs, for example `COVERS=C:\WOLF3D\*,C:\WOLF3D.EXE`. Do not quote it: the tokenizer treats `\` inside quotes as an escape and rejects `\W`. Unquoted values keep backslashes literally. A glob cannot contain a comma.
- SPDX validation uses the `spdx` crate version `0.13` with `ParseMode::STRICT`. `NONE` is handled before the parser. `LicenseRef-*` is detected in the expression text.
- URI syntax is not parsed in this slice. The rules check presence only. The registry schema validates the URI format later.
- `build_config_digest` does not include `licenses`. A license change affects the final config digest and the manifest digest, not the cache key of a later filesystem step.
- The local-build warning text is exactly `warning: the image has no license entries`.
- The built-in FreeDOS entry uses `NAME=FreeDOS`, `SPDX=GPL-2.0-only`, `STATUS=redistributable`, `BASIS=open-source-license`, `HOLDER=FreeDOS contributors`, `LICENSE_URL=https://www.gnu.org/licenses/old-licenses/gpl-2.0.html`, `SOURCE_URL=https://github.com/FDOS`, and a short GPL notice.
- Keep the FreeDOS SPDX identifier in one `FREEDOS_SPDX` constant. Confirm `GPL-2.0-only` against the pinned FreeDOS distribution's `COPYING` before the first publish. Change only that constant if the upstream states `or-later`.
- The built-in DOSKER agent entry uses `NAME=DOSKER`, `SPDX=MIT OR Apache-2.0`, `STATUS=redistributable`, `BASIS=open-source-license`, `HOLDER=DOSker contributors`, `LICENSE_URL=https://github.com/i-zhirov/dosker#license`, and a short dual-license notice.

## Tasks

- [x] **Task 1: License model and semantic validation in `dosker-core`**
    - **Implements:** FR-001, FR-002, FR-003, design §Data Model, design §Requirements
    - **Modify `Cargo.toml`**
        - Add workspace dependency `spdx = { version = "0.13", default-features = false }`.
    - **Modify `crates/dosker-core/Cargo.toml`**
        - Add `spdx.workspace = true`.
    - **Create `crates/dosker-core/src/license.rs`**
        - `pub struct LicenseEntry` — fields `pub name: String`, `pub spdx: String`, `pub holder: Option<String>`, `pub status: LicenseStatus`, `pub basis: Option<LicenseBasis>`, `pub evidence: Option<String>`, `pub license_url: Option<String>`, `pub covers: Vec<String>`, `pub source_url: Option<String>`, `pub notice: Option<String>`.
        - Derive `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`. Add `#[serde(default, skip_serializing_if = "Vec::is_empty")]` to `covers`. Add `#[serde(default, skip_serializing_if = "Option::is_none")]` to each optional field.
        - `pub enum LicenseStatus { Redistributable, Abandonware }` — derive `Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize` with `#[serde(rename_all = "snake_case")]`.
        - `pub enum LicenseBasis { OpenSourceLicense, FreewareRelease, PublicDomain, RightsHolderStatement, GoodFaith }` — derive the same traits with `#[serde(rename_all = "kebab-case")]`.
        - `pub enum LicenseSummary { Redistributable, Abandonware, Mixed }` — derive the same traits with `#[serde(rename_all = "snake_case")]`.
        - `LicenseStatus::parse(&str) -> Option<Self>` and `LicenseStatus::as_str(self) -> &'static str`.
        - `LicenseBasis::parse(&str) -> Option<Self>` and `LicenseBasis::as_str(self) -> &'static str`.
        - `LicenseSummary::as_str(self) -> &'static str`.
        - `LicenseEntry::validate(&self) -> Result<(), LicenseError>` — checks the name token `^[A-Za-z0-9][A-Za-z0-9_.+-]*$` with length 1 to 64, a non-empty SPDX value, holder length 1 to 200, notice length up to 2000, each cover non-empty and up to 128 characters, optional URIs non-empty, and the conditional rules from Constraints. Parse every non-`NONE` SPDX expression with `spdx::Expression::parse_mode(..., spdx::ParseMode::STRICT)`. Detect a `LicenseRef-` term and require `license_url`. Require `source_url` when any parsed license ID reports `is_copyleft()`. Iterate with `expression.requirements()` and match `req.license` as `spdx::LicenseItem::Spdx { id, .. }` to call `id.is_copyleft()`. Return a reason that names the failed key.
        - `pub fn license_summary(entries: &[LicenseEntry]) -> Option<LicenseSummary>` — `None` for an empty slice, otherwise the rule from Constraints.
        - `pub fn validate_licenses(entries: &[LicenseEntry]) -> Result<(), LicenseError>` — validates every entry and rejects a duplicate `name`.
        - `pub fn merge_license_entry(entries: &mut Vec<LicenseEntry>, entry: LicenseEntry) -> Result<(), LicenseError>` — appends a new name, keeps one identical entry, and returns an error that names the duplicate when the existing entry differs.
        - `pub enum LicenseError` — `Invalid { reason: String }` with `thiserror`, message `{reason}`.
    - **Modify `crates/dosker-core/src/lib.rs`**
        - Add `pub mod license;` and re-export `LicenseBasis`, `LicenseEntry`, `LicenseError`, `LicenseStatus`, `LicenseSummary`, `license_summary`, `merge_license_entry`, `validate_licenses`.
    - **Create `crates/dosker-core/tests/license.rs`**
        - `an_open_source_entry_validates` — `MIT` with `open-source-license` and no source URL is accepted.
        - `a_copyleft_expression_requires_a_source_url` — `GPL-2.0-or-later` fails without `source_url` and passes with it.
        - `an_unknown_spdx_id_is_rejected` — `BOGUS-1.0` fails.
        - `a_license_reference_requires_a_license_url` — `LicenseRef-Custom` fails without `license_url` and passes with it.
        - `abandonware_requires_spdx_none_good_faith_and_evidence` — each missing or contradictory part fails.
        - `an_open_source_basis_requires_a_real_expression` — `open-source-license` with `SPDX=NONE` fails.
        - `a_good_faith_basis_requires_abandonware` — `good-faith` with `redistributable` fails.
        - `a_redistributable_none_entry_requires_a_free_basis_and_evidence` — `public-domain` with evidence passes, `open-source-license` fails, missing evidence fails.
        - `license_names_must_be_tokens` — a name with a space or a leading punctuation character fails.
        - `license_summary_reports_the_mixed_and_uniform_cases` — all abandonware, all redistributable, mixed, and empty.
        - `identical_license_entries_are_deduplicated` — `merge_license_entry` keeps one entry.
        - `conflicting_license_entries_are_rejected` — `merge_license_entry` fails with the name in the reason.
        - `license_entries_use_the_contract_json_shape` — serialization of a required-only entry has exactly `name`, `spdx`, and `status`, and a full entry round-trips.
        - `license_status_and_basis_use_the_contract_values` — enum serialization matches the lowercase and kebab-case values.
    - **Verify:** `cargo test -p dosker-core --test license`

- [x] **Task 2: `LICENSE` parsing in `doskerfile.rs`**
    - **Implements:** FR-001, FR-002, design §Data Model, design §Requirements
    - **Modify `crates/dosker-core/src/doskerfile.rs`**
        - Add `InstructionKind::License(LicenseEntry)`.
        - Add `Self::License(_) => "LICENSE"` to `InstructionKind::name`.
        - Add `license_names: BTreeSet<String>` to `Parser`.
        - Add `const LICENSE_KEYS: [&str; 10]` with the ten uppercase keys from Constraints.
        - Add `Parser::parse_license(&mut self, raw_args: &str, line: usize) -> Result<InstructionKind, DoskerfileError>` — expand placeholders with `self.expand`, tokenize with `tokenize`, split each token once at `=`, reject an unknown key, reject a repeated key, build a `LicenseEntry`, split `COVERS` on commas, parse `STATUS` and `BASIS` with their `parse` methods, call `LicenseEntry::validate`, and reject a repeated `NAME` in `license_names`. Report every failure with `line_error` and the line.
        - Dispatch `"LICENSE"` to `parse_license` in `parse_instruction`.
    - **Modify `crates/dosker-core/tests/doskerfile.rs`**
        - `license_parses_every_key` — a full line yields the expected entry, including an unquoted `COVERS=C:\WOLF3D\*,C:\WOLF3D.EXE` as a two-item list.
        - `license_keeps_unquoted_dos_backslashes_in_covers` — the parsed covers equal `["C:\\WOLF3D\\*", "C:\\WOLF3D.EXE"]`.
        - `license_rejects_a_quoted_cover_glob` — `COVERS="C:\WOLF3D\*"` fails at the line because the tokenizer rejects the `\W` escape.
        - `license_accepts_only_the_required_keys` — `NAME`, `SPDX`, and `STATUS` alone yield an entry with empty optional fields.
        - `license_requires_name_spdx_and_status` — each missing required key fails at its line.
        - `license_rejects_an_unknown_key` — an extra key fails.
        - `license_rejects_a_duplicate_key` — `SPDX` twice fails.
        - `license_rejects_a_duplicate_name_in_the_file` — two `LICENSE` lines with the same `NAME` fail on the second line.
        - `license_rejects_a_missing_conditional_field` — `abandonware` without `EVIDENCE` fails on its line.
        - `license_rejects_an_empty_key_value` — `NAME=` fails.
        - `license_rejects_an_unknown_status_or_basis` — each unknown enum value fails with the value in the message.
        - `license_expands_placeholders` — an `ENV` value is expanded into a license field.
        - Extend `parse_accepts_every_config_instruction` with a valid `LICENSE` line and expect `LICENSE` in the instruction-name list.
    - **Verify:** `cargo test -p dosker-core --test doskerfile`

- [x] **Task 3: Additive `ImageConfig.licenses` field and canonical stability**
    - **Implements:** FR-003, design §Data Model, design §Rollout And Compatibility
    - **Modify `crates/dosker-image/src/config.rs`**
        - Add `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub licenses: Vec<LicenseEntry>` to `ImageConfig`.
        - Call `validate_licenses(&self.licenses)` in `ImageConfig::validate` and map a failure to `ImageError::InvalidImageConfig` with the error reason.
        - Keep `IMAGE_CONFIG_SCHEMA_VERSION` at `1`.
    - **Modify every `ImageConfig` literal to add `licenses: Vec::new()`**
        - `crates/dosker-image/src/freedos/store.rs`
        - `crates/dosker-image/src/oci.rs` in `import_config`
        - `crates/dosker-image/tests/config.rs`
        - `crates/dosker-image/tests/layer.rs`
        - `crates/dosker-image/tests/support/mod.rs` in `sample_config`
        - `crates/dosker-build/tests/config_instructions.rs` in `tree_config` and `booter_config`
        - `crates/dosker-build/tests/normalize.rs` in `sample_config`
        - `crates/dosker-build/tests/support/mod.rs` in `sample_config`
    - **Modify `crates/dosker-image/tests/config.rs`**
        - `licenses_round_trip_through_json` — a valid entry survives a JSON round-trip.
        - `empty_licenses_are_omitted_from_canonical_json` — the canonical JSON text does not contain `licenses`.
        - `config_without_licenses_keeps_its_previous_top_level_keys` — for the sample config, whose `engine_config` is empty, the canonical object keys are exactly `agent_version`, `boot`, `booter`, `cmd`, `created_by`, `dos_flavour`, `engine_pin`, `env`, `exposed_ports`, `history`, `machine`, `requires`, `schema_version`, `volumes`, and `workdir`, and `schema_version` is `1`.
        - `config_validate_rejects_an_invalid_license_entry` — a bad SPDX value fails.
        - `config_validate_rejects_a_duplicate_license_name` — two entries with the same name fail.
        - `config_validate_accepts_a_valid_license_entry` — the entry from `licenses_round_trip_through_json` passes.
    - **Verify:** `cargo test -p dosker-image --test config --test layer`

- [x] **Task 4: `BuildState` handling, booter allowance, and normalization**
    - **Implements:** FR-001, FR-002, FR-024, design §Data Model, design §Requirements
    - **Modify `crates/dosker-build/src/state.rs`**
        - Add `InstructionKind::License { .. }` to the booter allowlist.
        - Add `InstructionKind::License(entry)` to the `apply_config` match — call `merge_license_entry(&mut self.config.licenses, entry.clone())` and map a failure to `instruction_error(instruction, error.to_string())`.
        - Leave the existing `record(normalized, now, true)` call as the history entry for the instruction.
    - **Modify `crates/dosker-build/src/normalize.rs`**
        - Add an `InstructionKind::License(entry)` arm that returns `LICENSE ` followed by `render_license(entry)`.
        - Add `fn render_license(entry: &LicenseEntry) -> String` — emits one token per present field in the order `NAME`, `SPDX`, `STATUS`, `HOLDER`, `BASIS`, `EVIDENCE`, `LICENSE_URL`, `COVERS`, `SOURCE_URL`, `NOTICE`, joins the tokens through `render_tokens`, and joins `covers` with commas.
        - Keep `build_config_digest` unchanged.
    - **Modify `crates/dosker-build/tests/config_instructions.rs`**
        - `license_appends_to_the_config_and_records_an_empty_layer` — the entry is stored and the history entry has `empty_layer = true`.
        - `an_identical_license_is_deduplicated` — a config that already holds the entry keeps one copy.
        - `a_conflicting_license_fails_without_changing_the_config` — the config is unchanged and the error is a `LICENSE` instruction error.
        - `a_booter_accepts_license` — a booter state accepts the instruction and records history.
        - Extend `config_only_instructions_do_not_change_the_head_layer` with a `LICENSE` step and expect one more history entry.
    - **Modify `crates/dosker-build/tests/normalize.rs`**
        - `normalize_license_uses_a_fixed_key_order` — a full entry renders the exact token order and quotes a value with spaces.
        - `build_config_digest_ignores_history_boot_and_licenses` — add a license entry to the changed config and keep the digest equal.
    - **Verify:** `cargo test -p dosker-build --test config_instructions --test normalize`

- [x] **Task 5: Base inheritance, identical dedupe, and conflict error**
    - **Implements:** FR-024, design §Data Model, design §Error Handling And Edge Cases
    - **No production change is needed for inheritance.** `BuildState::with_boot` receives the resolved base `ImageConfig` by value, so its `licenses` are already present when the instruction loop starts. `apply_config` merges each Doskerfile entry through `merge_license_entry`. This task adds the fixture and the tests that prove it.
    - **Modify `crates/dosker-build/tests/support/mod.rs`**
        - Add `pub fn sample_license(name: &str) -> LicenseEntry` — a valid redistributable `MIT` entry with `BASIS=open-source-license`.
        - Add `pub fn store_licensed_tree_base(root: &Path, reference: &str, licenses: Vec<LicenseEntry>) -> String` — stores an empty-tree base whose `ImageConfig` carries the supplied licenses and returns the manifest digest.
    - **Create `crates/dosker-build/tests/license_inheritance.rs`**
        - `a_child_appends_its_entries_to_the_base_entries` — the committed config lists the base names first and the Doskerfile name last.
        - `an_identical_base_entry_is_deduplicated` — a Doskerfile entry equal to a base entry leaves one entry.
        - `a_conflicting_base_entry_fails_the_build` — an entry with the same name and a different SPDX value returns a `LICENSE` instruction error and stores no tag.
    - **Verify:** `cargo test -p dosker-build --test license_inheritance`

- [x] **Task 6: Built-in `freedos:1.3` entries and the imported agent entry**
    - **Implements:** FR-025, design §Data Model, design §Requirements
    - **Create `crates/dosker-image/src/licenses.rs`**
        - `pub const FREEDOS_SPDX: &str = "GPL-2.0-only";` — the single place to correct the FreeDOS variant.
        - `pub const FREEDOS_SOURCE_URL: &str = "https://github.com/FDOS";`
        - `pub fn freedos_license() -> LicenseEntry` — the exact FreeDOS values from Plan Decisions, with `FREEDOS_SPDX`, `FREEDOS_SOURCE_URL`, a short notice, and no `covers`.
        - `pub fn agent_license() -> LicenseEntry` — the exact DOSKER values from Plan Decisions, with a short notice and no `covers`.
        - `pub fn freedos_base_licenses() -> Vec<LicenseEntry>` — `[freedos_license(), agent_license()]`.
    - **Modify `crates/dosker-image/src/lib.rs`**
        - Add `pub mod licenses;` and re-export `agent_license`, `freedos_base_licenses`, `freedos_license`.
    - **Modify `crates/dosker-image/src/freedos/store.rs`**
        - Set `licenses: freedos_base_licenses()` in `expected_base_config`. This is the only constructor; `BaseImage::base_config` delegates to it, so one change covers both.
    - **Modify `crates/dosker-image/src/oci.rs`**
        - In `import_config`, set `licenses: vec![agent_license()]` for `ImageMedia::Disk` and `Vec::new()` for `ImageMedia::Booter`.
    - **Modify `crates/dosker-build/src/build.rs`**
        - In `Builder::is_reusable_disk_import`, also require the stored config's `licenses` to equal `vec![dosker_image::agent_license()]`.
    - **Modify `crates/dosker-image/tests/freedos_store.rs`**
        - Extend `store_commits_a_freedos_config` to assert `config.licenses == freedos_base_licenses()` and that every entry validates.
        - `freedos_base_entries_are_valid_and_redistributable` — names are `FreeDOS` and `DOSKER`, and the summary is `Redistributable`.
    - **Modify `crates/dosker-image/tests/import_agent.rs`**
        - Extend `import_injects_the_agent_into_a_dos_image` to assert `config.licenses == vec![agent_license()]`.
        - `import_booter_has_no_agent_entry` — a booter import has empty `licenses`.
    - **Verify:** `cargo test -p dosker-image --test freedos_store --test import_agent && cargo test -p dosker-build --test build`

- [x] **Task 7: Local-build warning and end-to-end build test**
    - **Implements:** FR-004 (local-warning part), FR-024, design §Requirements
    - **Modify `crates/dosker-build/src/build.rs`**
        - Add `pub const LICENSE_MISSING_WARNING: &str = "warning: the image has no license entries";`.
        - After the instruction loop and before `commit_image`, when `state.config.licenses.is_empty()`, write `LICENSE_MISSING_WARNING` through the reporter.
    - **Modify `crates/dosker-build/src/lib.rs`**
        - Re-export `LICENSE_MISSING_WARNING`.
    - **Create `crates/dosker-build/tests/license_warning.rs`**
        - `a_build_without_entries_succeeds_and_warns` — the build succeeds, the reporter line equals `LICENSE_MISSING_WARNING`, and the committed config has empty `licenses`.
        - `a_build_with_an_entry_does_not_warn` — a `LICENSE` step stores the entry and prints no warning.
        - `a_build_from_a_licensed_base_does_not_warn` — the inherited entries prevent the warning.
        - `a_build_stores_the_declared_entry_in_the_config` — `dosker build` commits the entry and the config validates.
    - **Verify:** `cargo test -p dosker-build --test license_warning`

- [x] **Task 8: Documentation for the `LICENSE` instruction**
    - **Implements:** FR-001, FR-004 (documentation)
    - **Modify `.sdd/specs/2026-11-01-dosker/contracts/doskerfile.md`**
        - Add `license` to the grammar alternatives.
        - Add a `### LICENSE` section with the ten keys, the required keys, the `status` and `basis` values, the conditional rules, the `COVERS` comma-separated list, the summary values, repeatability, config-only behavior, and the booter allowance.
        - Add `LICENSE` to the config-only list and describe its normalized key order.
        - Document that a local build without entries succeeds and prints `warning: the image has no license entries`.
    - **Modify `README.md`**
        - After the example description, add a short paragraph that names `LICENSE`, its required keys, the two status values, the abandonware rule, and the local warning.
    - **Verify:** `rg -n "^### LICENSE$" .sdd/specs/2026-11-01-dosker/contracts/doskerfile.md && rg -n "LICENSE" README.md`

## Definition Of Done

The slice is complete when every task above is checked and
`cargo test -p dosker-core -p dosker-image -p dosker-build` passes. The
suite must prove that a Doskerfile with `LICENSE` builds and stores the
entries, a child inherits the base entries, an imported disk carries the
agent entry, and an image without entries keeps its previous canonical
config JSON.
