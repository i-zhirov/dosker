# Client Distribution Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> `Verify` command passes.

- **Date**: 2026-11-23
- **Status**: Implemented
- **Design**: `.sdd/specs/2026-11-22-doskerfile-registry/design.md`
- **Roadmap**: `.sdd/specs/2026-11-22-doskerfile-registry/roadmap.md` — slice: client-distribution
- **Depends on**: static-registry-repository
- **Manual prerequisites**: none for the fixture tests. The real site needs the Slice 3 deployment and a public HTTPS URL.
- **Goal**: Add the blocking registry client, `pull`, `search`, remote `image inspect`, the digest-keyed acceptance gate, auto-pull for `run`/`create`/`build`, and asset caching to `dosker`, proven against a local fixture site.

## Constraints

- `pull` resolves through the Pages API: the per-image index, then `manifest.json`, then `config.json`. It never uses the catalog summary or `licenses.json` for the gate.
- Verification order: manifest digest, config digest and config validation, license gate, asset digest and size, the tar's manifest and config digest, then store and tag.
- The asset tar must carry the verified manifest digest and config digest. A tar with a different manifest or config fails before any index write.
- The gate uses the summary of the verified `config.licenses`. An empty list fails closed. A summary other than `redistributable` needs an acceptance keyed by the exact image digest. The exit code is `3`.
- `manifest.json` and `config.json` must equal the OCI blobs byte for byte. The registry generator already copies them.
- The client uses HTTPS. Plain HTTP is accepted only for `localhost` and `127.0.0.1`. A redirect target must satisfy the same rule on every hop.
- An asset is capped at 2 GiB. A site document is capped at 16 MiB.
- The asset is cached at `<data-root>/cache/registry/assets/<hex>.tar`. Every pull re-hashes the cached bytes. Site documents are fetched on every pull.
- Acceptance is stored at `<data-root>/accepted/<hex>.json` and records the image digest, the config digest, the image label, the summary, and the time.
- A tag pull tags `owner/name:<tag>`. A digest pull tags `owner/name:<version>`.
- The HTTP client is a small blocking HTTP/1.1 client over `rustls` with `ring` and `webpki-roots`. It is not `reqwest` and not `ureq`.
- Tests use the fixture site over loopback HTTP. The fixture site is built from the real OCI store and is served by a test-only static server.
- Re-verification runs on every pull, even when the asset is cached.

## Plan Decisions

- The transport, client, acceptance store, cache, and pull logic live in `dosker-image`. No new crate is added.
- The workspace gains `rustls` (default features off; `ring`, `std`, `tls12`), `webpki-roots`, and `url`. `rustls` uses an explicit ring provider, not the process default.
- `EXIT_LICENSE = 3` is a new stable exit code. `EXIT_ERROR` stays `1`.
- `CliError` gains a `Pull(PullError)` variant; `BuildError` gains `Pull(PullError)`. Both map `LicenseNotAccepted` to `EXIT_LICENSE`, so `pull`, `run`, `create`, and `build` share the hint.
- The acceptance hint is printed from `PullError::LicenseNotAccepted`'s `Display`: the summary, the image digest, the config digest, and `dosker pull <ref> --accept-license <image-digest>`.
- `run`, `create`, and `build` gain `--no-pull`. Auto-pull applies only when the reference is namespaced or host-qualified and the image is missing locally.
- `create_container` parses a bare name or digest with `ImageRef`; otherwise it parses `RegistryReference`, derives the local `ImageRef`, and pulls on `NotFound`.
- `build` auto-pulls a missing remote base. A non-redistributable base returns the gate error and the hint; the CLI maps it to `EXIT_LICENSE`.
- `search` filters `namespace/name`, `description`, and tag names with a case-insensitive substring. The table shows the tags; `--json` prints the catalog object with a filtered `images` array.
- `image inspect` keeps the local view for a bare local reference and adds the remote view for a namespaced or host-qualified reference.
- The fixture site has one redistributable image and one abandonware image. The abandonware image proves the gate.
- The fixture writer and static server live in `dosker-test-support` and take manifest, config, and tar bytes. The per-crate test support builds those bytes from an `OciStore`. This avoids a dev-dependency cycle.
- The client rejects a bare local reference. `pull` needs a namespace or an explicit host.

## Registry Requirements

The client needs these from the registry. Slice 3 already provides them; this list is the contract to test and to keep.

1. `/index.json` returns the catalog with `schema`, `generated_at`, `registry`, and `images[]`.
2. `/v1/<owner>/<name>/index.json` returns the image index with `tags` and `versions[]`.
3. Each `versions[]` entry carries `version`, `digest`, `config_digest`, `manifest_url`, `config_url`, `asset_url`, `asset_digest`, `asset_size`, `published_at`, `license_summary`, and `licenses_url`.
4. `manifest_url` and `config_url` return the exact OCI blobs, so `sha256` matches `digest` and `config_digest`.
5. `asset_url` is a public HTTPS URL whose body hashes to `asset_digest` and has size `asset_size`. It is the OCI tar, and the tar's manifest and config match the verified documents.
6. `tags` maps every live tag to a manifest digest. A moved tag points at a new digest.
7. The verified config carries `licenses`. `licenses.json` is for the HTML site and is not needed by `pull`.
8. The catalog `license_summary` and `requires_acceptance` are advisory. The client recomputes the summary from the config. The registry must still keep them consistent for the UI and validation.
9. A revoked version is absent from `tags` and `versions`. The client fails closed.
10. The site is public. There are no accounts, tokens, or private images.
11. Version paths use `sha256-<hex>` and stay immutable. A new digest gets a new path.
12. GitHub Release downloads may redirect. The final URL must stay HTTPS. The client follows HTTPS redirects only.

Gaps the fixture site must model until the real registry has them:

- The real registry has one redistributable image today. The fixture adds an abandonware image for the gate.
- The real client never reads the publish sidecar `<version>.json`. The fixture site does not include it.
- The fixture site serves HTTP on loopback. Production must serve HTTPS.
- The fixture documents must deserialize into the client types. A Rust test checks this.

## Tasks

- [x] **Task 1: Fixture site and loopback static server**
    - **Implements:** design §Acceptance Scenarios, roadmap slice `Done when`
    - **Create** `crates/dosker-test-support/src/registry_site.rs` and add `pub mod registry_site` to `src/lib.rs`. Add `sha2` to `crates/dosker-test-support/Cargo.toml`.
        - `StaticServer::start(root: PathBuf) -> io::Result<Self>` binds `127.0.0.1:0`, serves `GET` from `root`, sends `200` with `Content-Length`, and sends `404` for a missing path or a path with `..`.
        - `url(&self) -> String`, `requests(&self) -> Vec<String>`, `reset_requests()`, and a `Drop` that stops the loop. The listener is non-blocking and checks a stop flag every few milliseconds.
        - `SiteImage { owner, name, version, tag, description, homepage, deprecated, license_summary, manifest_bytes, config_bytes, asset_tar, published_at }`.
        - `build_site(destination: &Path, base_url: &str, generated_at: &str, images: &[SiteImage]) -> io::Result<()>` computes the three digests and sizes, writes `assets/<asset_digest>.tar`, `v1/<owner>/<name>/index.json`, `v1/<owner>/<name>/versions/sha256-<hex>/{manifest.json,config.json}`, and `index.json`.
        - The builder writes `requires_acceptance = license_summary != "redistributable"` and uses `base_url` for every URL.
        - Test order: create the directory, start `StaticServer`, then call `build_site` with `server.url()`.
    - **Modify** `crates/dosker-image/tests/support/mod.rs` (it exists; add `pub mod site;`) and create `crates/dosker-image/tests/support/site.rs`.
        - `site_image(store: &OciStore, reference: &ImageReference, version: &str, tag: &str, description: &str, published_at: &str) -> SiteImage` reads the manifest and config blobs, saves the tar with `archive::save_image`, and computes the summary.
        - Add `dosker-test-support` to `[dev-dependencies]` of `crates/dosker-image` and `crates/dosker-build`.
    - **Create** `crates/dosker-test-support/tests/static_server.rs`: a document is served, a missing path is 404, a `..` path is 404, and the request log records the path.
    - **Verify:** `cargo test -p dosker-test-support --test static_server`

- [x] **Task 2: Blocking HTTP client**
    - **Implements:** FR-018, FR-019, design §Assumptions And Constraints
    - **Modify** `Cargo.toml`, `crates/dosker-image/Cargo.toml`, and `Cargo.lock`: add `rustls` (`default-features = false`, features `ring`, `std`, `tls12`), `webpki-roots`, and `url`. The build needs crates.io access; the local registry has sources but not every `.crate` archive.
    - **Create** `crates/dosker-image/src/http.rs` and add `pub mod http` to `src/lib.rs`.
        - `pub const MAX_ASSET_BYTES: u64 = 2 * 1024 * 1024 * 1024`.
        - `pub const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024`.
        - `pub struct DownloadReport { pub bytes: u64, pub digest: String }`.
        - `pub fn get(url: &str, max_bytes: u64) -> Result<Vec<u8>, HttpError>`.
        - `pub fn download(url: &str, max_bytes: u64, destination: &Path) -> Result<DownloadReport, HttpError>` streams to a unique temporary file in the destination directory, hashes as it reads, removes the temporary file on failure, and renames on success.
        - `pub enum HttpError` with `InvalidUrl`, `Scheme`, `Redirect`, `Status`, `TooLarge`, `Timeout`, `Protocol`, `Tls`, and `Io`.
        - Scheme rule: `https` always; `http` only for `localhost` and `127.0.0.1`.
        - Redirect rule: at most five `301`, `302`, `303`, `307`, or `308`; resolve `Location`; apply the scheme rule to every target; rebuild the `Host` header per hop.
        - Send `Host`, `Connection: close`, `Accept-Encoding: identity`, and `User-Agent: dosker/<version>`. Set a 30 s connect timeout and a 120 s read timeout.
        - Read `Content-Length` and chunked bodies; a missing length reads to close.
        - Build `rustls::ClientConfig` with `builder_with_provider(ring)` and `webpki_roots::TLS_SERVER_ROOTS`.
        - `fn redirect_target(origin: &Url, location: &str) -> Result<Url, HttpError>` is a pure helper for the scheme and resolution rules.
    - **Create** `crates/dosker-image/tests/http.rs` with a one-shot loopback responder.
        - `a_loopback_document_is_fetched`.
        - `a_remote_plain_http_url_is_refused`.
        - `an_https_to_http_redirect_is_refused` (unit-test `redirect_target` with an `https://` origin and an `http://` location).
        - `a_loopback_redirect_is_followed`.
        - `a_body_over_the_cap_is_refused`.
        - `a_chunked_body_is_read`.
        - `a_missing_resource_reports_the_status`.
        - `a_connect_timeout_is_reported`.
    - **Verify:** `cargo test -p dosker-image --test http`

- [x] **Task 3: Registry document types and client**
    - **Implements:** FR-006, FR-007, FR-008, FR-027
    - **Create** `crates/dosker-image/src/remote.rs` and add `pub mod remote` to `src/lib.rs`.
        - `Catalog`, `CatalogImage`, `Latest`, `ImageIndex`, and `VersionRecord` with the schema field names.
        - `RegistryClient::new(base: &str) -> Result<Self, RemoteError>` and `RegistryClient::for_reference(&RegistryReference) -> Result<Self, RemoteError>`. An explicit loopback host uses `http://`; every other host uses `https://`; no host uses `default_registry_url()`.
        - `catalog(&self) -> Result<Catalog, RemoteError>`, `image_index(&self, &Repository) -> Result<ImageIndex, RemoteError>`, and `resolve(&self, &ImageIndex, &ReferenceSelector) -> Result<&VersionRecord, RemoteError>`.
        - `manifest(&self, &VersionRecord) -> Result<Vec<u8>, RemoteError>` verifies `sha256` against `digest` and the manifest config descriptor against `config_digest`.
        - `config(&self, &VersionRecord) -> Result<ImageConfig, RemoteError>` verifies `sha256` against `config_digest`, parses the config, and validates it.
        - `download_asset(&self, &VersionRecord, &Path) -> Result<http::DownloadReport, RemoteError>` verifies the reported digest and size.
        - `RemoteError` with `Http`, `Status`, `ImageNotFound`, `TagNotFound`, `DigestNotFound`, `DigestMismatch`, `InvalidDocument`, `UnsafeUrl`, `Io`, and `Store(ImageError)`.
        - The base URL gains a trailing slash. Joins use relative paths, so a base with a path prefix works.
    - **Create** `crates/dosker-image/tests/remote.rs`.
        - `a_catalog_is_read`.
        - `an_image_index_resolves_a_tag`.
        - `a_digest_selects_the_version`.
        - `a_tampered_manifest_fails`.
        - `a_tampered_config_fails`.
        - `a_missing_image_reports_not_found`.
        - `an_explicit_loopback_host_uses_http`.
        - `the_fixture_documents_deserialize`.
    - **Verify:** `cargo test -p dosker-image --test remote`

- [x] **Task 4: Acceptance store**
    - **Implements:** FR-028
    - **Create** `crates/dosker-image/src/acceptance.rs` and add `pub mod acceptance` to `src/lib.rs`.
        - `AcceptedImage { schema_version, digest, config_digest, image, license_summary, accepted_at }`.
        - `AcceptanceStore::new(root: PathBuf)`, `is_accepted(&str) -> Result<bool, ImageError>`, `record(&AcceptedImage) -> Result<(), ImageError>`, `list() -> Result<Vec<AcceptedImage>, ImageError>`, and `path(&str) -> Result<PathBuf, ImageError>`.
        - `path` parses `ImageDigest` and joins `<root>/accepted/<hex>.json`. Writes are atomic.
    - **Create** `crates/dosker-image/tests/acceptance.rs`.
        - `an_acceptance_round_trips`.
        - `a_config_digest_is_recorded`.
        - `a_corrupt_record_is_reported`.
        - `an_unsafe_digest_is_rejected`.
    - **Verify:** `cargo test -p dosker-image --test acceptance`

- [x] **Task 5: Asset cache**
    - **Implements:** FR-019
    - **Extend** `crates/dosker-image/src/remote.rs`.
        - `RemoteCache::new(root: PathBuf)`, `asset_path(&str) -> Result<PathBuf, RemoteError>`, and `get_or_download(&self, client: &RegistryClient, version: &VersionRecord) -> Result<(PathBuf, bool), RemoteError>`.
        - A cached file is re-hashed against `asset_digest` and `asset_size`. A mismatch deletes it and downloads again. The flag is `true` on a cache hit.
        - Documents are not cached.
    - **Create** `crates/dosker-image/tests/remote_cache.rs`.
        - `a_second_pull_reuses_the_cached_asset`.
        - `a_tampered_cached_asset_is_downloaded_again`.
        - `a_tampered_server_asset_fails`.
        - `a_new_digest_uses_a_new_cache_entry`.
        - `an_unsafe_digest_is_rejected`.
    - **Verify:** `cargo test -p dosker-image --test remote_cache`

- [x] **Task 6: Pull core and targeted archive load**
    - **Implements:** FR-006, FR-009, FR-027, FR-028
    - **Modify** `crates/dosker-image/src/archive.rs`: add `load_image_as(store, source, reference, expected_manifest, expected_config)`. Share the verification with `load_image`; before any index write, require the tar's manifest digest to equal `expected_manifest` and its config digest to equal `expected_config`; commit only `reference` and ignore the archive ref annotations.
    - **Modify** `crates/dosker-image/src/oci.rs`: add `pub fn root(&self) -> &Path`.
    - **Extend** `crates/dosker-image/src/remote.rs`:
        - `PullRequest { reference: RegistryReference, accept: Option<ImageDigest> }`.
        - `PullReport { image: String, tag: Option<String>, digest: String, config_digest: String, license_summary: LicenseSummary, asset_bytes: u64, cached: bool }`.
        - `PullError` with `Remote`, `MissingLicense { image }`, `LicenseNotAccepted { image, digest, config_digest, summary, entries }`, `AcceptMismatch { expected, provided }`, and `Image`. `LicenseNotAccepted`'s `Display` prints the accept hint.
        - `pull(client, store, cache, acceptance, request, now) -> Result<PullReport, PullError>` follows the documented order. It rejects an empty license list. It checks the acceptance before the asset download. It computes the local reference: the requested tag, or `owner/name:<version>` for a digest pull. It calls `load_image_as` with the verified digests and records the acceptance after success.
    - **Create** `crates/dosker-image/tests/pull.rs`.
        - `a_redistributable_tag_pull_stores_and_tags`.
        - `a_digest_pull_tags_the_version`.
        - `a_moved_tag_replaces_the_local_tag`.
        - `an_unknown_tag_fails`.
        - `a_gate_refuses_an_abandonware_image`.
        - `an_acceptance_unlocks_the_exact_digest`.
        - `a_different_digest_needs_a_new_acceptance`.
        - `an_acceptance_is_recorded_after_success`.
        - `a_catalog_summary_does_not_bypass_the_gate`.
        - `a_refused_pull_requests_no_asset`.
        - `a_missing_license_list_fails`.
        - `an_asset_with_a_different_manifest_fails`.
        - `a_pull_loads_only_blobs_that_match_their_digests`.
    - **Verify:** `cargo test -p dosker-image --test pull`

- [x] **Task 7: `pull` and `search` commands**
    - **Implements:** FR-006, FR-007, FR-009, FR-028
    - **Modify** `crates/dosker-cli/src/cli.rs`, `src/commands/mod.rs`, `src/context.rs`, and `src/error.rs`.
        - `EXIT_LICENSE = 3`.
        - `Command::Pull(PullArgs { reference, accept_license: Option<String> })` and `Command::Search(SearchArgs { term, json: bool })`.
        - `Context` gains `acceptance: AcceptanceStore` and `registry_cache: RemoteCache`.
        - `CliError` gains `Pull(PullError)`; `exit_code` maps `LicenseNotAccepted` to `EXIT_LICENSE`.
    - **Create** `crates/dosker-cli/src/commands/pull.rs`.
        - Reject a bare local reference with a usage error.
        - Parse `RegistryReference` and `ImageDigest`; build the client; call `remote::pull`; print `pulled <ref> <digest> (<n> bytes)` or `already current`.
        - Map `PullError::LicenseNotAccepted` to `EXIT_LICENSE` and print its message.
    - **Create** `crates/dosker-cli/src/commands/search.rs`.
        - Fetch the catalog; filter name, description, and tags; print `owner/name  summary  tags  description`, or the catalog object with a filtered `images` array with `--json`.
    - **Modify** `crates/dosker-cli/tests/common/mod.rs`: add `command_with_registry(url)` that sets `DOSKER_REGISTRY`.
    - **Create** `crates/dosker-cli/tests/cli_pull.rs`.
        - `pull_accepts_a_redistributable_image`.
        - `pull_requires_acceptance_for_an_abandonware_image` (exit 3, hint, no asset request).
        - `pull_with_accept_license_stores_the_image`.
        - `pull_rejects_a_bare_local_reference`.
        - `search_lists_a_matching_image`.
        - `search_json_returns_the_catalog_entries`.
    - **Verify:** `cargo test -p dosker-cli --features mock-engine --test cli_pull`

- [x] **Task 8: Remote and local `image inspect`**
    - **Implements:** FR-008
    - **Modify** `crates/dosker-cli/src/cli.rs`, `src/commands/image.rs`, and `src/commands/mod.rs`.
        - `ImageCommand::Inspect(ImageInspectArgs { reference })`.
        - A bare local reference shows the local tag, digest, config digest, size, and licenses.
        - A namespaced or host-qualified reference fetches the index, resolves the tag or digest, verifies the manifest and config, and prints the tag, version, digest, config digest, asset URL, size, asset digest, summary, and license entries.
    - **Create** `crates/dosker-cli/tests/cli_image_inspect.rs`.
        - `inspect_shows_a_remote_image`.
        - `inspect_shows_a_local_image`.
        - `inspect_reports_an_unknown_tag`.
    - **Verify:** `cargo test -p dosker-cli --features mock-engine --test cli_image_inspect`

- [x] **Task 9: Auto-pull for `run`, `create`, and `build`**
    - **Implements:** FR-023
    - **Modify** `crates/dosker-cli/src/cli.rs`, `src/commands/create.rs`, `src/commands/run.rs`, `src/commands/build.rs`, and `src/error.rs`.
        - Add `--no-pull` to `RunArgs`, `CreateArgs`, and `BuildArgs`.
        - Add `no_pull` to `CreateOptions` and `BuildOptions`.
        - `create_container` parses a bare name or digest with `ImageRef`; otherwise parses `RegistryReference`, derives the local `ImageRef`, and on `NotFound` with a namespace or host and `!no_pull` pulls and retries.
        - `CliError::Build(BuildError::Pull(LicenseNotAccepted))` maps to `EXIT_LICENSE`.
    - **Modify** `crates/dosker-build/src/build.rs` and `src/error.rs`.
        - `BuildOptions.no_pull`.
        - `resolve_image_base` on `RegistryBaseNotLocal` and `!no_pull` pulls the base and retries.
        - `BuildError::Pull(PullError)` and the `CliError` mapping.
    - **Update every `BuildOptions` literal** with `no_pull: false`: `crates/dosker-cli/src/commands/build.rs`, `crates/dosker-build/tests/build.rs`, `run_in_dos.rs`, `freedos_base.rs`, `license_inheritance.rs`, `license_warning.rs`, and `build_requires.rs`.
    - **Create** `crates/dosker-cli/tests/cli_auto_pull.rs` and extend `crates/dosker-build/tests/build.rs`.
        - `run_pulls_a_missing_remote_image`.
        - `run_with_no_pull_does_not_pull`.
        - `create_pulls_a_missing_remote_image`.
        - `build_pulls_a_missing_remote_base`.
        - `build_with_no_pull_keeps_the_pull_hint`.
        - `build_maps_a_license_refusal_to_exit_3`.
    - **Verify:** `cargo test -p dosker-build --test build && cargo test -p dosker-cli --features mock-engine --test cli_auto_pull`

- [x] **Task 10: Documentation and full verification**
    - **Implements:** design §Client commands, roadmap slice `Done when`
    - **Modify** `docs/registry.md`, `docs/images.md`, `docs/builds.md`, `docs/base-images.md`, and `README.md`.
        - Remove the "network client is a later slice" notes.
        - Document `pull`, `search`, `image inspect`, `--accept-license`, the exit code `3`, auto-pull, and `--no-pull`.
        - Document the cache and acceptance paths.
        - Correct the `docs/base-images.md` statement about FreeDOS redistribution; the registry publishes a FreeDOS-based image.
    - **Verify:** `cargo test -p dosker-image -p dosker-build && cargo test -p dosker-cli --features mock-engine && cargo clippy -p dosker-image -p dosker-build -p dosker-cli --all-targets --features mock-engine -- -D warnings && cargo fmt --all -- --check`

## Definition Of Done

- `pull` resolves a tag and a digest through the fixture site, verifies the manifest, config, asset, the tar's own digests, and every blob, and stores and tags the image.
- A non-redistributable image refuses with exit `3`, prints the ready-to-run accept command, and downloads no asset.
- An acceptance unlocks the exact digest, is recorded with the config digest, and is reused.
- `search` and `image inspect` work against the fixture site.
- `run`, `create`, and `build` auto-pull a missing remote reference and respect `--no-pull`.
- A cached asset is re-verified on every pull.
- The full test suite, clippy, and fmt pass.

## Deviations

- **The fixture writer lives in `dosker-test-support`.** It takes the manifest, config, and tar bytes, so it has no dependency on `dosker-image`. Each test crate builds those bytes from its own store. This avoids a dev-dependency cycle and keeps one site writer.
- **The static server makes each accepted stream blocking.** On macOS the accepted socket inherits the listener's non-blocking flag; a large asset was truncated at about 327 KiB before this fix.
- **`PullError::LicenseNotAccepted` carries a boxed `LicenseRefusal`.** Clippy rejects the 144-byte variant under `result_large_err`.
- **`build --no-pull` keeps the existing pull hint.** The auto-pull for `build` is proven through the CLI fixture; the build test keeps the local-only path.
- **The run and create auto-pull tests use an imported disk image.** A tree image without a raw disk cannot start, so the fixture imports a disk.
- **An offline run test was added.** After a pull, the server is dropped and `run` starts the local tag with no registry request.
- **`pull` and `push` are not synonyms.** A tag pull stores `owner/name:<tag>`; a digest pull stores `owner/name:<version>`.
- **The client rejects a bare local reference.** `pull` requires a namespace or an explicit host.
- **The HTTP response type is aliased and the char trims use arrays.** Both are clippy fixes.
