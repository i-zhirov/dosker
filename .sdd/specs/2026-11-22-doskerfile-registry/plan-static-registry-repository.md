# Static Registry Repository Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> `Verify` command passes.

- **Date**: 2026-11-23
- **Status**: Implemented
- **Manual prerequisites status (2026-11-23):** local implementation proceeds for Tasks 1–8. The domain/`$id`, the GitHub repository and Pages, the license wording, the agent binaries, the `0.1.3` release, and a host `check-jsonschema` install are still open. Task 9's real publish and the CI runs wait for them. Local checks use `uvx check-jsonschema` when the binary is absent.
- **Design**: `.sdd/specs/2026-11-22-doskerfile-registry/design.md`
- **Roadmap**: `.sdd/specs/2026-11-22-doskerfile-registry/roadmap.md` — slice: static-registry-repository
- **Depends on**: registry-references-and-validation
- **Manual prerequisites**:
    - Choose the final registry domain and update the schema `$id`.
    - Create the `dosker-registry` repository and enable Pages through a custom Actions workflow.
    - Settle the DOS agent/OpenWatcom license terms and the GPL source-offer wording.
    - Decide how registry CI obtains `DOSKER.EXE`.
    - Publish the `dosker` release `0.1.3` that ships `registry validate` and `registry check-final`.
    - Install `check-jsonschema` and `actionlint` on the test host.
- **Goal**: Create the separate `dosker-registry` repository with the registry layout, the six schemas, the deterministic site generator, the read-only `validate` workflow, the idempotent `publish` workflow, `POLICY.md`, and the first published example image. Add `registry check-final` to the `dosker` CLI so publish can repeat coverage on the built tree.

## Constraints

- Tasks 1, 2, and 4–9 run in the `dosker-registry` checkout. Task 3 runs in the `dosker` checkout.
- The registry repository has `namespaces.toml`, `revoked.toml`, `images/<owner>/<name>/`, `schemas/`, `site/`, `scripts/`, `.github/workflows/`, `tests/`, `README.md`, and `POLICY.md`.
- The six schemas are copies of `contracts/{catalog,image-index,licenses,namespaces,registry,revoked}.schema.json` from the design. The `$id` host is the placeholder `https://dosker.dev/schemas/...` until the manual prerequisite changes it.
- Pages paths are `/index.json`, `/v1/<owner>/<name>/index.json`, and `/v1/<owner>/<name>/versions/sha256-<hex>/{licenses.json,manifest.json,config.json}`, plus the HTML.
- The catalog, image index, and license record match their schemas exactly. The image index uses a `tags` map and a `versions` array. A version is immutable; a tag is movable.
- The design uploads each tar as `<asset_digest>.tar` to one Release per image. This plan adds a `<version>.json` sidecar with `version`, `digest`, `config_digest`, `license_summary`, `published_at`, `asset_digest`, and `asset_size`. The site generator reads the tars and the sidecars.
- Publish uploads the tar before the site entry exists. A second publish of the same version and digest is a no-op. A different digest for an existing version fails. A license correction is a new `version`.
- A revoked digest is omitted from every tag and version list. An image with no live version is omitted from the catalog.
- `publish` repeats the coverage check on the final built tree with `dosker registry check-final <ref>`.
- The site generator is deterministic. It uses `SOURCE_DATE_EPOCH` for `generated_at`, sorts every key and list, and writes compact JSON with a trailing newline.
- The generator fails if an index entry points to a missing asset, if two images declare the same tag, whether a version is bound to a different digest, or if an owner or name contains `..` or a path separator.
- `validate.yml` is read-only, never runs `RUN`, and runs `dosker registry validate` for every image plus `check-jsonschema`. `publish.yml` has `contents: write`, `pages: write`, and `id-token: write`, uses one concurrency group, and deploys Pages only after every asset and site document exists.
- The first image is `i-zhirov/hello-dos` from the `hello-dos` example plus a `COPY NOTICE.txt C:\DOSKER\NOTICE.TXT`. It inherits the FreeDOS and agent entries and needs the notice.
- A local `dosker` binary that contains `registry validate` and `registry check-final` is required for the registry tests.

## Plan Decisions

- The Python generator is Python 3.11 with the standard library (`tomllib`, `json`, `tarfile`, `pathlib`, `html`). Tests use `unittest`. CI adds `check-jsonschema`; `actionlint` checks the workflows.
- The Release for an image is tagged `<owner>-<name>`. The assets are `<asset_digest>.tar` and `<version>.json`. The sidecar is the version record; the tar is the transport.
- The generator takes `--repo`, `--releases` (a JSON inventory), `--out`, and `--epoch`. The inventory is the GitHub Releases API shape `{ "tag_name": ..., "assets": [{ "name", "browser_download_url", "size", "digest" }] }` filtered to the registry.
- The current `registry.toml` supplies the description, homepage, deprecated, tags, and version. The current version must carry the `latest` tag. The current tag set maps to the current digest. Older versions come from the inventory.
- The first image uses the `freedos:1.3` base. The `COPY NOTICE.txt C:\DOSKER\NOTICE.TXT` path exercises the Slice 2 notice exception.
- The registry `validate.yml` is the design asset with the schema steps intact and the `DOSKER_VERSION` pin `0.1.3`. The `publish.yml` is new.
- `registry check-final` loads a tagged image from the local store, reads its config licenses, and repeats the coverage and notice checks on its final tree. It does not inspect the Doskerfile.
- The site templates are plain HTML and CSS with a small inline search filter. No framework and no build step beyond the generator.
- The `takedown.yml` workflow is out of scope for this slice; `revoked.toml` and the generator omission are in scope.

## Tasks

- [x] **Task 1: Registry repository skeleton, schemas, and policy**
    - **Implements:** FR-010, FR-011, FR-012, FR-013, design §Component Interfaces, design §API Contracts
    - **Create** `README.md`, `POLICY.md`, `.gitignore`, `namespaces.toml`, `revoked.toml`, `schemas/`, `site/`, `scripts/`, `tests/`, `images/`.
    - `namespaces.toml`: `schema = 1` and the `i-zhirov` owner with `github = "i-zhirov"` and a proof URL.
    - `revoked.toml`: `schema = 1` and `revoked = []`.
    - `.gitignore`: `__pycache__/`, `*.pyc`, `_site/`, `tests/tmp/`.
    - Copy the six schemas from the design `contracts/` into `schemas/`.
    - `POLICY.md`: namespace proof, abandonware claim and takedown, GPL source offer, agent license, and the first-image license statement.
    - `README.md`: the layout, the local validation command, the publish flow, the contribution rules, and the schema-`$id` note.
    - `site/style.css` and `site/templates/` with the base layout and the three views as placeholders for Task 5.
    - **Create** `tests/test_schemas.py`: every schema parses; `namespaces.toml` and `revoked.toml` convert to JSON and pass `check-jsonschema`.
    - **Verify:** `python3 -m unittest tests.test_schemas`

- [x] **Task 2: First image and fixture registries**
    - **Implements:** FR-004, FR-013, FR-026, FR-030, design §Rollout
    - **Create** `images/i-zhirov/hello-dos/Doskerfile` from the `hello-dos` example plus `COPY NOTICE.txt C:\DOSKER\NOTICE.TXT`.
    - **Create** `images/i-zhirov/hello-dos/NOTICE.txt` with the FreeDOS and agent notice text.
    - **Create** `images/i-zhirov/hello-dos/registry.toml`: `schema = 1`, `version = "1.0.0"`, `tags = ["latest"]`, a description, and a homepage.
    - **Create** `tests/fixtures/registry-good/` with `namespaces.toml`, `revoked.toml`, and one `freedos:1.3` image that copies a notice and covers its own files.
    - **Create** the bad fixtures: a contradictory license, an uncovered COPY, a missing notice, an unknown owner, and a duplicate tag. The no-license case stays covered by the `dosker` registry tests.
    - **Create** `scripts/validate.sh <root>`: converts TOML, runs `check-jsonschema`, and runs `dosker registry validate` for every `images/*/*/` under the given root.
    - **Verify:** `scripts/validate.sh images && scripts/validate.sh tests/fixtures/registry-good && for d in tests/fixtures/registry-bad-*; do ! scripts/validate.sh "$d"; done`

- [x] **Task 3: `dosker registry check-final`**
    - **Implements:** FR-026 (publish part), design §`publish` workflow
    - **In the `dosker` checkout**, extend `crates/dosker-build/src/registry.rs`:
        - `pub fn validate_final(reference: &ImageRef, store: &OciStore) -> Result<ImageValidation, RegistryError>` loads the config and tree, reads `config.licenses`, requires at least one, validates the entries, and calls `coverage_misses` and the notice rule on the final tree.
        - A booter image uses the booter rule and returns `BooterNoticeUnverifiable` when a notice is required.
    - Extend `crates/dosker-cli/src/cli.rs` and `crates/dosker-cli/src/commands/registry.rs`:
        - `RegistryCommand::CheckFinal(CheckFinalArgs { image: String })`.
        - `execute` prints `checked <ref>` on success and `error: ...` with exit 1 on failure.
    - Tests:
        - `crates/dosker-build/tests/registry_validation.rs`: `a_final_tree_outside_coverage_fails`, `a_final_tree_with_the_notice_passes`, `a_final_booter_without_a_wildcard_fails`.
        - `crates/dosker-cli/tests/cli_registry.rs`: `registry_check_final_accepts_a_covered_image`, `registry_check_final_rejects_an_uncovered_executable`.
    - **Verify (in the `dosker` checkout):** `cargo test -p dosker-build --test registry_validation && cargo test -p dosker-cli --features mock-engine --test cli_registry && cargo clippy -p dosker-build -p dosker-cli --all-targets --features mock-engine -- -D warnings`

- [x] **Task 4: Site generator registry documents**
    - **Implements:** FR-010, FR-011, FR-012, FR-022, design §Site generator, design §API Contracts
    - **Create** `scripts/generate_site.py`:
        - Arguments: `--repo`, `--releases`, `--assets`, `--out`, `--epoch`, `--registry-url`.
        - Inputs: `namespaces.toml`, `revoked.toml`, every `images/*/*/registry.toml`, the Releases API JSON, and the downloaded asset directory.
        - Outputs: `index.json`, `v1/<owner>/<name>/index.json`, and `v1/<owner>/<name>/versions/sha256-<hex>/{licenses.json,manifest.json,config.json}`.
        - The catalog carries `schema=1`, `generated_at` from the epoch, `registry` from `--registry-url`, and the sorted image entries with `description`, `homepage`/`deprecated` when present, `tags`, `license_summary`, `requires_acceptance`, and `latest`.
        - The image index carries `schema=1`, `image`, `namespace`, `name`, `description`, optional `homepage`/`deprecated`, `tags`, and the sorted `versions`. Each version carries `version`, `digest`, `config_digest`, `manifest_url`, `config_url`, `asset_url`, `asset_digest`, `asset_size`, `published_at`, `license_summary`, and `licenses_url`.
        - The license record carries `schema=1`, `image`, `digest`, `config_digest`, `summary`, and the entries from the verified config.
        - The tar reader opens `<asset_digest>.tar`, checks the asset digest and size, reads the OCI layout, the manifest, and the config, and refuses a mismatch.
        - The version sidecar `<version>.json` supplies the version, digest, config digest, summary, publish time, asset digest, and asset size.
        - JSON is compact with sorted keys, sorted arrays, and a trailing newline. The page paths use `sha256-<hex>`.
    - **Create** `tests/fixtures/oci.py`: a deterministic OCI-layout tar builder for the generator fixtures.
    - **Create** `tests/fixtures/site-good/` and `tests/fixtures/releases-good.json`; the golden outputs under `tests/golden/site/`.
    - **Create** `tests/test_generate_site.py`:
        - `the_catalog_matches_the_golden_and_schema`.
        - `the_image_index_matches_the_schema`.
        - `the_license_record_matches_the_schema`.
        - `the_manifest_and_config_are_copied_byte_for_byte`.
        - `the_current_tags_point_to_the_current_digest`.
        - `the_latest_version_is_the_current_version`.
        - `two_runs_are_byte_identical`.
        - `check_jsonschema_accepts_every_generated_document`.
    - **Verify:** `python3 -m unittest tests.test_generate_site`

- [x] **Task 5: Site generator HTML and CSS**
    - **Implements:** design §UI Design, design §Site generator
    - **Create** `site/style.css`, `site/app.js`, and `site/templates/` for the base layout, the home list with search, the image page, and the version page.
    - The home shows `owner/name`, description, tags, and the license badge. The image page shows the description, homepage, tags, and the version table. The version page shows the license entries, any abandonware notice, the asset link, and its digest.
    - **Extend** `scripts/generate_site.py` to render `index.html`, `v1/<owner>/<name>/index.html`, and the per-version page, copy the CSS and JS, and escape all user text.
    - The HTML uses semantic headings, a visible focus outline, and text that does not rely on color alone for the license status.
    - **Create** `tests/test_site_html.py`: the home lists the image; the image page lists the version; the version page shows the entries and the notice; the abandonware notice is present; the search script and CSS are copied; the output is deterministic.
    - **Verify:** `python3 -m unittest tests.test_site_html`

- [x] **Task 6: Site generator negative and revoked cases**
    - **Implements:** FR-014, FR-022, FR-029, design §Error Handling And Edge Cases
    - **Extend** `scripts/generate_site.py` and tests for these failures:
        - `two images declare the same tag`.
        - `a version record points to a missing asset`.
        - `the asset digest or size does not match the sidecar`.
        - `a version is bound to a different digest`.
        - the current version has no `latest` tag.
        - the current version has no sidecar.
        - an owner or name contains `..` or a path separator.
    - **Add** `tests/test_generate_site_errors.py`:
        - `a_duplicate_tag_fails`.
        - `a_missing_asset_fails`.
        - `a_tampered_asset_fails`.
        - `a_version_conflict_fails`.
        - `a_missing_latest_fails`.
        - `a_missing_current_sidecar_fails`.
        - `a_path_traversal_is_rejected`.
        - `a_revoked_version_and_its_tags_are_omitted`.
        - `an_image_with_no_live_version_is_omitted`.
        - `a_revoked_digest_does_not_remove_a_local_copy`.
    - **Verify:** `python3 -m unittest tests.test_generate_site_errors`

- [x] **Task 7: Read-only `validate` workflow**
    - **Implements:** FR-015, design §`validate` workflow
    - **Copy** the design asset `.sdd/specs/2026-11-22-doskerfile-registry/assets/workflows/validate.yml` to `.github/workflows/validate.yml` unchanged except for a header comment that names this repository.
    - Confirm the workflow keeps `contents: read`, the protected-file guard, the TOML-to-JSON step, the five schema checks, the pinned `0.1.3` release with `SHA256SUMS` verification, and one `dosker registry validate` call per image.
    - Add a `scripts/validate_schemas.sh <root>` helper that runs the same TOML conversion and schema checks locally, so the fixture tests and the workflow agree.
    - **Extend** `tests/test_schemas.py`: the workflow exists, parses, has no `write` permission, has no `RUN` step, and pins `DOSKER_VERSION`.
    - **Verify:** `actionlint .github/workflows/validate.yml && python3 -m unittest tests.test_schemas`

- [x] **Task 8: `publish` workflow and publisher**
    - **Implements:** FR-026, design §`publish` workflow
    - **Create** `scripts/publish.py`: takes `--image`, `--version`, `--digest`, `--config-digest`, `--summary`, `--asset`, `--release-dir`, `--published-at`, and `--dry-run`. It computes the asset digest and size, writes `<asset_digest>.tar` and `<version>.json`, and enforces the idempotency and immutability rules.
    - **Create** `.github/workflows/publish.yml`:
        - Trigger on `push` to `main` for `images/**`, `namespaces.toml`, `revoked.toml`, and the workflow, plus `workflow_dispatch`.
        - One `concurrency` group; `contents: write` and `pages: write` only in the jobs that need them.
        - A `changes` job that lists the changed `images/<owner>/<name>` directories with `scripts/changed_images.py`.
        - A `build` matrix that installs the pinned `0.1.3` release, runs `dosker build`, `dosker save`, `dosker registry check-final`, `scripts/publish.py`, and `gh release upload`. The tar is uploaded before the sidecar.
        - A `site` job that downloads every Release asset, runs `scripts/generate_site.py`, runs `check-jsonschema` on the generated documents, and deploys Pages only after all assets and documents exist.
    - **Create** `scripts/changed_images.py` and tests.
    - **Create** `tests/test_publish.py`: first publish writes the tar and the sidecar; a repeat is a no-op; a different digest fails; a license correction uses a new version; the tar is uploaded before the site entry.
    - **Verify:** `python3 -m unittest tests.test_publish && actionlint .github/workflows/publish.yml`

- [x] **Task 9: First image, end-to-end, and documentation**
    - **Implements:** FR-030, design §Rollout, roadmap slice `Done when`
    - Build `images/i-zhirov/hello-dos` locally, save it, publish it into a temporary release directory, generate the site, and run the schema checks.
    - **Create** `scripts/e2e.sh`: build, save, publish, generate, validate, a second idempotent publish, and a rebuild that reproduces the catalog.
    - **Finish** `README.md` and `POLICY.md` with the first-image license statement, the publish flow, and the manual prerequisites that remain.
    - The real GitHub Release and Pages deployment stay manual until the plan's prerequisites are met.
    - **Verify:** `scripts/e2e.sh && python3 -m unittest discover -s tests && actionlint .github/workflows/*.yml`

## Definition Of Done

- `validate.yml` passes on `tests/fixtures/registry-good` and fails on every `tests/fixtures/registry-bad-*` directory.
- `publish.yml` and `scripts/publish.py` publish the example image idempotently and refuse a digest or version conflict.
- The generated site passes `check-jsonschema` for the catalog, the image index, and every license record.
- A rebuild from the repository and the Release inventory reproduces the catalog byte for byte.
- `POLICY.md` covers namespace proof, abandonware claim and takedown, the GPL source offer, the agent license, and the first-image license statement.
- Every local check in the task `Verify` lines passes. The manual prerequisites stay listed in the plan and the README.

## Deviations

- **`latest` is not required per image.** FR-021 makes tags unique across the whole registry, so only one image can own `latest`. The catalog `latest` now names the `registry.toml` current version. Task 6's `a_missing_latest_fails` became `an_image_without_tags_fails`.
- **Assets are read from one directory per release.** The generator reads `<assets>/<owner>-<name>/<asset_digest>.tar` and `<assets>/<owner>-<name>/<version>.json`. A flat download directory lets two images that use the same version overwrite the same sidecar. `publish.yml` downloads each Release into its tag directory.
- **`scripts/check_site.sh` is shared.** The `publish` workflow and `scripts/e2e.sh` both call it. The Task 7 schema helper is `scripts/validate_schemas.sh`, and `scripts/validate.sh` calls it.
- **`check-jsonschema` falls back to `uvx`.** The test host has no `check-jsonschema` binary. The scripts use `uvx --quiet check-jsonschema` in that case. CI installs the binary with `pip`.
- **Task 8 ordering test changed.** The workflow uploads the tar before the sidecar. The unit test proves that a sidecar with a missing tar restores the tar and stays a no-op.
- **No static `tests/fixtures/releases-good.json`.** The fixture inventory is generated with the assets, so the asset digests and sizes stay in sync.
- **Local Python is 3.14.7.** The code needs 3.11 or newer. CI pins 3.12.
- **The registry checkout is a sibling directory.** The local checkout is `<worktree>/dosker-registry`, initialized with `git init`. Nothing is committed or pushed.
- **The plan is `Implemented`; the roadmap slice stays `In Progress`.** All tasks and local checks are complete, and `scripts/e2e.sh` passes locally. The roadmap `Done when` needs the manual prerequisites: the final domain and `$id`, the GitHub repository with Pages, the license wording, the agent-binary path, and the `0.1.3` DOSker release.
