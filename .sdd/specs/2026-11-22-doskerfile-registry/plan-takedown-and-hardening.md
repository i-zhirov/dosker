# Takedown And Hardening Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> `Verify` command passes.

- **Date**: 2026-11-23
- **Status**: Implemented
- **Design**: `.sdd/specs/2026-11-22-doskerfile-registry/design.md`
- **Roadmap**: `.sdd/specs/2026-11-22-doskerfile-registry/roadmap.md` — slice: takedown-and-hardening
- **Depends on**: static-registry-repository, client-distribution
- **Manual prerequisites**: the two decisions are resolved. The takedown contact is `https://github.com/i-zhirov/dosker-registry/issues`; the response window is 7 days; signing is deferred and must be recorded. The real takedown still needs the Slice 3 deployment (the repository and Pages).
- **Repositories**: Tasks 1, 2, 3, and 5 run in the `dosker-registry` checkout (`../dosker-registry`). Task 4 runs in the `doskerfile-registry` workspace. Task 6 spans both. Commit each repository separately.
- **Goal**: Add the takedown workflow, harden the site generator and the client against revoked digests, keep local copies, record the takedown contact and response window, and record the deferred-signing decision.

## Constraints

- The takedown contact is the public issue tracker `https://github.com/i-zhirov/dosker-registry/issues`. The policy promises a response within 7 days.
- Signing is deferred for the first version. Release assets stay unsigned. The SHA-256 digests are the integrity check. The decision and its reason go into `POLICY.md`.
- A revocation is recorded in `revoked.toml` first. The merge triggers the site deploy because `publish.yml` already watches `revoked.toml`.
- The takedown deletes the image tar and keeps the sidecar. The sidecar maps a version to its digest, so the generator can skip a revoked version before it reads the missing tar. A missing sidecar for an inventory-listed version is an error; the takedown never deletes a sidecar.
- The site generator omits every revoked digest from every tag and version list. An image whose **current** version is revoked is omitted even when older live versions exist. The tags map only to the current digest, and `image-index.schema.json` requires at least one tag, so the image has no valid tag map. FR-029 requires omission when no version is live and does not require retention otherwise. This is a plan decision, not a design change.
- The client never deletes a local copy. A local tag stays after a takedown.
- A later publish must not restore a deleted tar. `publish.py` skips a current digest that `revoked.toml` records, and the publish workflow's upload step skips an empty release directory instead of failing on an unmatched glob.
- The site deploy is one shared reusable workflow. `publish.yml` and `takedown.yml` both call it. One concurrency group named `publish` serializes them. The shared workflow defines no concurrency of its own.
- The takedown workflow runs only on `main`, only from a maintainer dispatch, and only when the digest is already in `revoked.toml`. The takedown job carries an explicit `if: github.ref == 'refs/heads/main'`. A retry is a no-op.
- The contact repository name and the Pages host may differ until the Slice 3 final-domain prerequisite is met. This plan keeps the existing `REGISTRY_URL` value and uses the contact URL only for policy and workflow messages.
- Tests use the fixture site and a temporary release directory. The real workflow needs the Slice 3 deployment.

## Plan Decisions

- Extract the Pages deploy from `publish.yml` into `.github/workflows/site.yml` with `on: workflow_call`. Move `runs-on`, `timeout-minutes`, `environment`, and the `REGISTRY_URL` constant into `site.yml`. The caller job keeps `needs`, `if`, and an explicit `permissions` block. A `uses` job cannot have `steps`, `runs-on`, or `environment`.
- Add `.github/workflows/site.yml` to the `publish.yml` `paths` filter.
- `scripts/takedown.py` has a `--check` mode and a `--names` mode. It never deletes and has no dry-run flag. The workflow deletes with `gh release delete-asset` after checking the asset exists.
- The sidecar stays after a takedown. Only `<asset_digest>.tar` is deleted.
- `publish.py` takes an explicit `--repo` (defaulting to the checkout root) and reads `revoked.toml` from it. If the current digest is revoked for the same namespace and name, it prints a skip message and returns 0 without copying the tar. Matching on the digest alone would over-skip another image that shares a manifest.
- The client takedown tests serve a rebuilt site with the image omitted. The local store keeps the tag from the earlier pull. A pull of the omitted image fails with `ImageNotFound`, not `TagNotFound`.
- The policy test asserts the contact, the literal `7 days`, and the signing sentence so they cannot drift silently.
- The signing decision is "deferred". No signing key and no attestation command are added.
- Only this plan and roadmap Slice 5 become `Implemented`. The design and the roadmap overall keep their status because Slice 3 is still `In Progress` and the design's open questions remain.

## Registry Contract For A Takedown

- `revoked.toml` records `namespace`, `name`, `digest`, `reason`, `decided_at`, and optionally `issue`.
- The image index omits the revoked digest from `tags` and `versions`.
- An image with no live current version is absent from `/index.json`.
- The version documents for a revoked digest are not published.
- The tar is deleted from the Release. The sidecar remains.
- A later publish skips the revoked current digest, so the tar is not restored.
- The client sees a missing image or tag and fails closed. It keeps any local copy.

## Tasks

- [x] **Task 1: Shared Pages deploy workflow** (registry checkout)
    - **Implements:** FR-016, design §`publish` workflow
    - **Create** `.github/workflows/site.yml` with `on: workflow_call`. Move the current site steps from `publish.yml`: checkout, Python, `check-jsonschema`, release inventory and asset download, `scripts/generate_site.py`, `scripts/check_site.sh`, optional `CNAME`, `actions/upload-pages-artifact`, and `actions/deploy-pages`.
        - Workflow permissions: `contents: read`, `pages: write`, `id-token: write`.
        - Workflow `env`: `REGISTRY_URL` with the current value from `publish.yml`.
        - Job: `runs-on`, `timeout-minutes`, and `environment: github-pages` with `url: ${{ steps.deployment.outputs.page_url }}`.
        - No `concurrency` block.
    - **Modify** `.github/workflows/publish.yml`: replace the `site` job body with `uses: ./.github/workflows/site.yml`; keep `needs`, the `if`, and the explicit `permissions` block; remove the moved `environment`, `step`, and `REGISTRY_URL` entries; add `".github/workflows/site.yml"` to the `paths` filter.
    - **Create** `tests/test_workflows.py`: `site.yml` exists with `workflow_call`; `publish.yml` calls it and no longer contains the Pages upload or deploy steps; `site.yml` has no `concurrency`; `publish.yml` lists `site.yml` in its paths.
    - **Verify:** `actionlint .github/workflows/*.yml && python3 -m unittest tests.test_workflows`

- [x] **Task 2: Takedown helper, workflow, and publish guard** (registry checkout)
    - **Implements:** FR-016, FR-029, design §`Takedown`
    - **Create** `scripts/takedown.py`:
        - `--check --repo <repo> --namespace --name --digest` returns 0 when `revoked.toml` has the record, and 1 with a clear message when it does not.
        - `--names --assets <dir> --digest` reads the downloaded sidecars and prints the tar name and the sidecar name, tab separated. It errors when no sidecar matches.
        - The sidecar is never deleted; the helper reports it for the operator but the workflow deletes only the tar.
    - **Create** `.github/workflows/takedown.yml`: `workflow_dispatch` inputs `namespace`, `name`, `digest`; `concurrency: group: publish`; a `takedown` job with `if: github.ref == 'refs/heads/main'`, `permissions: contents: write`, `actions/checkout`, `GH_TOKEN: ${{ github.token }}`, a `--check` run, `gh release download <tag> --pattern '*.json'`, a `--names` run, and a delete loop that checks `gh release view <tag> --json assets` before `gh release delete-asset <tag> <tar> --yes`; a `deploy` job with `needs: takedown`, `permissions: {contents: read, pages: write, id-token: write}`, and `uses: ./.github/workflows/site.yml`.
    - **Modify** `scripts/publish.py`: add `--repo` (default: the checkout root), read `revoked.toml` from it, and skip the current digest when the same namespace and name are revoked. Print a skip message and return 0 without copying the tar.
    - **Modify** `.github/workflows/publish.yml`'s build upload step: use `shopt -s nullglob` or `compgen -G` so an empty release directory uploads nothing and succeeds.
    - **Create** `tests/test_takedown.py`: the record check passes and fails; the names are found; a missing sidecar errors; a retried delete plan still names the sidecar.
    - **Modify** `tests/test_publish.py`: `a_revoked_current_digest_is_not_republished` builds a temporary repository root with a `revoked.toml` and passes `--repo`.
    - **Modify** `tests/test_workflows.py`: `takedown.yml` calls the shared site workflow, uses the `publish` concurrency group, and carries the `main` guard; parse the workflow as YAML when PyYAML is available, otherwise assert the exact quoted `site.yml` entry under `on.push.paths` in `publish.yml`.
    - **Verify:** `python3 -m unittest tests.test_takedown tests.test_publish tests.test_workflows && actionlint .github/workflows/*.yml`

- [x] **Task 3: Generator revocation hardening** (registry checkout)
    - **Implements:** FR-016, FR-029, SC-009, SC-015
    - **Modify** `tests/test_generate_site_errors.py` only unless a gap appears:
        - `a_revoked_current_version_with_older_live_versions_omits_the_image`: revoke the current digest, keep an older live version, and assert the image and its tags are absent.
        - Confirm the existing `a_revoked_version_and_its_tags_are_omitted` already covers the deleted tar and the successful generation.
    - **Rule:** the generator checks the revoked set before it reads the tar, so keeping the sidecar and deleting only the tar is safe. Do not add a skip for a missing sidecar; that is an error.
    - **Verify:** `python3 -m unittest tests.test_generate_site_errors`

- [x] **Task 4: Client takedown behavior** (DOSker workspace)
    - **Implements:** FR-016, FR-028, FR-029
    - **Create** `crates/dosker-cli/tests/cli_takedown.rs` using the disk fixture `registry_site_image_disk`:
        - `pull_after_a_takedown_fails_and_keeps_the_local_copy`: pull a fixture tag; build the site **into a fresh directory** and start a **new** `StaticServer` with the image omitted (`build_site` does not remove stale files); the pull exits 1 with `image not found`; the local tag remains; `run -d` starts it offline.
        - `pull_a_revoked_digest_fails`: the same fresh site and server; a pull by digest exits 1 with `image not found`.
    - **Create** fixtures in `crates/dosker-cli/tests/common/mod.rs` only if the site helper cannot express the empty catalog.
    - **Verify:** `cargo build -p dosker-test-support --bin mock-engine && CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/usr/bin/cc CC=/usr/bin/cc cargo test -p dosker-cli --features mock-engine --test cli_takedown`

- [x] **Task 5: Policy and docs** (registry and DOSker checkouts)
    - **Implements:** FR-029, design §`Takedown`, roadmap slice
    - **Modify** `POLICY.md`:
        - Record the takedown contact `https://github.com/i-zhirov/dosker-registry/issues` and the literal `7 days` response window.
        - Record the process: issue with the image name, digest, and evidence; maintainer PR to `revoked.toml`; merge deploys the filtered site; maintainer dispatches the takedown workflow; the tar is deleted; the sidecar stays; local copies stay.
        - Replace the "asset deletion is out of scope for the first release" sentence with the takedown behavior.
        - Record the signing decision: assets are unsigned in the first version; digests are the integrity check; signing and attestation are deferred.
    - **Create** `tests/test_policy.py`: the policy contains the contact URL, `7 days`, `unsigned`, and `digest`.
    - **Modify** `docs/registry.md` (takedown, local copy, unsigned), `docs/images.md` (local copy retention), and the registry `README.md` if its policy link or summary needs the new behavior.
    - **Verify:** `python3 -m unittest tests.test_policy`

- [x] **Task 6: Full verification and statuses**
    - **Registry:** `python3 -m unittest discover -s tests && actionlint .github/workflows/*.yml && DOSKER_BIN=<path-to-dosker> scripts/validate.sh images`. Install `check-jsonschema` or let `scripts/check_site.sh` use `uvx`.
    - **DOSker:** `cargo build -p dosker-test-support --bin mock-engine && CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/usr/bin/cc CC=/usr/bin/cc cargo test -p dosker-image -p dosker-build && CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/usr/bin/cc CC=/usr/bin/cc cargo test -p dosker-cli --features mock-engine && CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/usr/bin/cc CC=/usr/bin/cc cargo clippy -p dosker-image -p dosker-build -p dosker-cli --all-targets --features mock-engine -- -D warnings && cargo fmt --all -- --check`.
    - **Update** this plan's status to `Implemented`, the roadmap Slice 5 to `Implemented`, and the roadmap Slice 5 `Plan` field to this plan's path. Leave the design and the roadmap overall status unchanged: Slice 3 is still `In Progress` and the design's open questions remain.

## Definition Of Done

- A takedown removes the version and its tags from the catalog.
- A pull through a revoked tag fails.
- A local copy is unchanged and still starts.
- The signing decision is recorded.
- The docs match the behavior.
- The registry and DOSker test suites, clippy, fmt, and actionlint are clean.
- The design success criteria for the slice are met, with the FR-029 current-version interpretation recorded above: SC-009 and SC-015 hold; the design's overall success criteria and the roadmap Definition Of Done wait on the Slice 3 deployment and the design's open questions.

## Progress

- 2026-11-23: Approved.
- 2026-11-23: All tasks implemented; registry 57 tests, DOSker suites, clippy, fmt, actionlint, and `validate.sh images` are green.
- 2026-11-23: Slice 3 is deployed and verified, so its status is `Implemented`. The design and roadmap overall stay `Approved` for the design's remaining OpenWatcom license review; the resume and signing questions are deferred.
- 2026-11-23: The takedown workflow was exercised against `i-zhirov/takedown-test`. The takedown deleted the tar, omitted the image, and a fresh pull failed while the local copy stayed. The exercise also found that an empty CI release directory skipped the version-immutability check; the publish workflow now downloads the existing sidecars before publishing.
- 2026-11-23: The content-immutability extension was exercised. The sidecar records the manifest-layer content digest; the takedown stamps it for older versions; a bumped version of the revoked test image was rejected by the publish build with `content is revoked`, while the release and site stayed unchanged.

## Deviations

- The `publish.yml` build step skips the release creation as well as the uploads when the release directory is empty. Without that, a revoked image could create an empty release.
- The client takedown tests build the site into a fresh directory and start a new server, because `build_site` does not delete stale files.
- `tests/test_workflows.py` checks `site.yml` with plain text rather than a YAML parser; the repository has no PyYAML dependency and the check is deliberately narrow.
- `takedown.py` prints the sidecar name for the operator but the workflow deletes only the tar. The sidecar stays for the generator's version-to-digest map.
- `scripts/validate.sh images` passed with the locally built `target/debug/dosker` and the local FreeDOS base.
