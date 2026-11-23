# DOSkerfile Registry Roadmap

> Each slice gets its own plan, written when the slice starts. Execute a
> slice with the `executing-plans` skill.

- **Date**: 2026-11-23
- **Status**: Implemented
- **Design**: `.sdd/specs/2026-11-22-doskerfile-registry/design.md`
- **Strategy**: Five dependency-ordered slices across two repositories. Slice 1
  is the smallest end-to-end license tracer: the `LICENSE` instruction, the
  `LicenseEntry` model, the config field, the build-state handling, inheritance,
  and the built-in entries, with no network at all. Slice 2 adds the namespaced
  reference grammar and `dosker registry validate`, which together make the
  registry enforceable and give CI a read-only gate that never runs `RUN`.
  Slice 3 builds the separate registry repository: the layout, the metadata
  files, the six JSON schemas, the site generator, the two workflows, the
  policy, and one real published image. Slice 4 adds the client distribution
  path against that site: pull, search, remote inspect, digest-keyed
  acceptance, and auto-pull. Slice 5 closes takedown and hardening across both
  repositories. The manual prerequisites that gated Slices 3 to 5 are
  resolved: the registry host and Pages, the agent license terms, the
  source-offer wording, the agent binaries in CI, the takedown contact, and the
  signing decision.
  Each slice is verified on its own before the next starts. The design is too
  large for one plan, so each slice gets a plan just in time.

## Slices

### Slice 1 — license-core

Delivers the license model in the existing `dosker` workspace. It covers the
repeatable `LICENSE` Doskerfile instruction, `LicenseEntry` and its enums in
`dosker-core`, the parser and its semantic rules, the additive
`ImageConfig.licenses` field, `BuildState::apply_config` handling, the
booter-image allowance, base-license inheritance with identical-dedupe and
conflict-error, the built-in `freedos:1.3` base entries, the always-present
DOSKER agent entry for imported images, and the local-build warning when no
entry exists. Unit tests and build tests prove the slice.

In scope: the `NAME`, `SPDX`, `STATUS`, `HOLDER`, `BASIS`, `EVIDENCE`,
`LICENSE_URL`, `COVERS`, `SOURCE_URL`, and `NOTICE` fields; the conditional
field rules; the SPDX expression and `LicenseRef-*` checks; the
`redistributable`/`abandonware`/`mixed` summary; the skipped-when-empty
serialization that keeps old config digests stable; and inheritance from a
resolved base. Out of scope: any registry, any reference grammar, the
validator, and the network client. The slice needs no network, so a build with
`LICENSE` and an inherited base verifies it on its own.

- **Status:** Implemented
- **Depends on:** —
- **Requirements:** FR-001, FR-002, FR-003, FR-004 (local-warning part),
  FR-024, FR-025
- **Manual prerequisites:** —
- **Plan:** [`plan-license-core.md`](plan-license-core.md)
- **Done when:** `cargo test` for `dosker-core`, `dosker-image`, and
  `dosker-build` passes; a Doskerfile with `LICENSE` builds and the config
  carries the entries; a child of a licensed base inherits the base entries;
  an imported image carries the agent entry; an image without entries still
  serializes to its old config digest.

### Slice 2 — registry-references-and-validation

Delivers the enforceable registry contract in `dosker`. It covers the
namespaced reference grammar (`owner/name:tag`, `host/owner/name:tag`,
`owner/name@sha256:...`), the `DOSKER_REGISTRY` override, lock-file digest
recording for a remote base, the `dosker registry validate <dir>` command,
registry-wide tag uniqueness, namespace and `registry.toml` validation, the
license coverage check against the base and `COPY` inputs without running
`RUN`, the copyleft `SOURCE_URL` and notice rules, and the
registry-enforcement part of the local-build warning rule. It builds on the
parser and model from Slice 1 and stands alone against a local fixture
registry.

In scope: bare `name:tag` staying local; the namespaced forms; the default
registry host and its override; the base-digest lock entry; per-directory
validation of `registry.toml`, the Doskerfile, and the license entries;
registry-wide tag uniqueness; namespace proof and `registry.toml` schema
checks; the `COVERS` coverage rule for a non-system top-level directory and
executable; the `SOURCE_URL` requirement for a copyleft entry; the
`C:\DOSKER\NOTICE.TXT` requirement; and the booter `COVERS=*` allowance. Out of
scope: the registry repository, the site, the workflows, and the network
client. Tests use a local fixture registry, so the slice verifies on its own.

- **Status:** Implemented
- **Depends on:** Slice 1
- **Requirements:** FR-004 (registry part), FR-005, FR-017, FR-020, FR-021,
  FR-026 (validator part), FR-031 (validation part)
- **Manual prerequisites:** none. Tests use a local fixture registry.
- **Plan:** [`plan-registry-references-and-validation.md`](plan-registry-references-and-validation.md)
- **Done when:** `dosker registry validate` accepts the valid fixtures and
  rejects a missing license, a contradictory status/basis/SPDX combination, a
  duplicate tag, and an uncovered top-level executable; `owner/name:tag` and
  digest references parse; a build from a remote fixture base records the
  resolved digest in the lock file.

### Slice 3 — static-registry-repository

Delivers the new `dosker-registry` repository. It covers the repository
layout, `namespaces.toml`, `revoked.toml`, `registry.toml`, the six JSON
schemas under `schemas/`, the site generator that produces `index.json`,
per-image indexes, per-version `licenses.json`, `manifest.json`, `config.json`,
and the HTML UI, the `validate.yml` and `publish.yml` workflows, `POLICY.md`,
the immutable-version and idempotent-publish rules, full-tree coverage after
`RUN`, the license-correction rule, and the first published example image. It
consumes the parser, model, validator, and reference grammar from Slices 1 and
2, and stands alone once its manual prerequisites are met.

In scope: the registry directory conventions; the catalog, image-index, and
license-record documents matching their schemas; the browse UI; the read-only
`validate` workflow that never runs `RUN`; the `publish` workflow that rebuilds,
uploads the tar before the index entry exists, and deploys Pages atomically; the
namespace proof model; version immutability and idempotency; the full-tree
coverage check on publish; and the rule that a license correction becomes a new
version. Out of scope: the client commands and the takedown workflow. The slice
verifies on its own against fixtures and one real image.

- **Status:** Implemented
- **Depends on:** Slices 1 and 2
- **Requirements:** FR-010, FR-011, FR-012, FR-013, FR-014, FR-015, FR-022,
  FR-026 (publish part), FR-030
- **Manual prerequisites:** resolved. The final host is
  `https://i-zhirov.github.io/dosker-registry`; the repository and Pages are
  live; `POLICY.md` records the license wording; CI builds the agent and the
  FreeDOS base from the pinned DOSKer source; the `0.1.4` release is pinned.
- **Plan:** [`plan-static-registry-repository.md`](plan-static-registry-repository.md)
- **Done when:** `validate.yml` passes on a good fixture and fails on each bad
  fixture; `publish.yml` publishes the example image idempotently; the
  generated site passes the schemas; a rebuild from the repository and the
  Release inventory reproduces the catalog.

### Slice 4 — client-distribution

Delivers the client distribution path in `dosker`. It covers the small
blocking HTTP client with `rustls`, `dosker pull`, `dosker search`, remote
`dosker image inspect`, verified `manifest.json` and `config.json` before the
license gate, the digest-keyed acceptance store and stable exit code, caching
and re-verification, the HTTPS and 2 GiB rules, and auto-pull for `run`,
`create`, and `build` with `--no-pull`. It builds on the reference grammar from
Slice 2 and the published site from Slice 3, and stands alone against a local
static server.

In scope: tag resolution through the Pages API; asset and manifest and blob
digest verification; verified manifest and config before reading licenses; the
acceptance gate that refuses a non-redistributable image without the exact
digest; `search` filtering the catalog locally; remote `inspect`; HTTPS-only
downloads that refuse a redirect leaving HTTPS; the 2 GiB cap; re-verification
on every pull even when cached; and auto-pull with `--no-pull` for `build`. Out
of scope: `push`, accounts, private registries, and takedown. Tests use a local
static server, so the slice verifies on its own.

- **Status:** Implemented
- **Depends on:** Slices 2 and 3
- **Requirements:** FR-006, FR-007, FR-008, FR-009, FR-018, FR-019, FR-023,
  FR-027, FR-028
- **Manual prerequisites:** none beyond a published or fixture registry site.
  Tests use a local static server.
- **Plan:** [`plan-client-distribution.md`](plan-client-distribution.md)
- **Done when:** an end-to-end test builds, publishes to a temporary directory,
  serves, pulls, and runs; a tampered asset fails; an abandonware image refuses
  to pull without the acceptance flag and succeeds with it; a tampered catalog
  summary does not bypass the gate; `search` and remote `inspect` work; `run`
  works offline after a pull.

### Slice 5 — takedown-and-hardening

Delivers takedown and hardening across `dosker` and `dosker-registry`. It
covers the takedown workflow, revocation filtering in the site generator,
removal of tags that point at revoked digests, omission of images with no live
version, local-copy retention, the signing decision, and the policy and user
documentation updates. It builds on the registry from Slice 3 and the client
from Slice 4, and stands alone once its manual prerequisites are met.

In scope: recording a revocation in `revoked.toml`; the takedown workflow that
deletes the asset and redeploys Pages; filtering revoked digests from every tag
and version list; dropping a tag that points at a revoked digest; omitting an
image with no live version; keeping a local copy untouched; the recorded
signing decision; and updates to `POLICY.md` and the user docs to match the
behavior. Out of scope: automated signing when the decision defers it. The
slice verifies on its own through the catalog, a pull through a revoked tag, and
a local copy.

- **Status:** Implemented
- **Depends on:** Slices 3 and 4
- **Requirements:** FR-016, FR-029
- **Manual prerequisites:** resolved. The contact is the issue tracker
  `https://github.com/i-zhirov/dosker-registry/issues`, the response window is
  7 days, and signing is deferred and recorded in `POLICY.md`.
- **Plan:** [`plan-takedown-and-hardening.md`](plan-takedown-and-hardening.md)
- **Done when:** a takedown removes the version and its tags from the catalog; a
  pull through a revoked tag fails; a local copy is unchanged; the signing
  decision is recorded; docs match the behavior.

## Definition Of Done

All five slices are `Implemented`. Each slice plan is `Approved` and executed.
The design's success criteria SC-001 to SC-016 are met. The full test suite is
green. No open Critical or Important finding remains. The design's open
questions are resolved or explicitly deferred.
