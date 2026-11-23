# Release

This page describes the release runners, the optional signing steps, the
release workflow, the manual fallback commands, and the acceptance record.

## Release runners

Release jobs use GitHub-hosted runners: `ubuntu-latest` for Linux x86_64,
`macos-15` for macOS arm64, and `windows-latest` for Windows x86_64. No
self-hosted runner is used. The tag-triggered release workflow is the source
of truth.

## Signing

Artifacts are unsigned by default. Signing credentials are optional, and the
signing-credentials prerequisite is waived. The macOS signing and
notarization step needs `MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PASSWORD`,
`MACOS_NOTARY_APPLE_ID`, `MACOS_NOTARY_PASSWORD`, and `MACOS_TEAM_ID`. The
Windows signing step needs `WINDOWS_CERTIFICATE_BASE64` and
`WINDOWS_CERTIFICATE_PASSWORD`.

The steps are skipped when the secrets are absent. The release notes and
`docs/install.md` state `unsigned` and give the checksum commands. The
acceptance does not require the signing secrets.

## Release workflow

The workflow has five jobs: `version`, `linux-x86_64`, `macos-arm64`,
`windows-x86_64`, and `publish`. It uploads the artifacts
`dosker-linux-x86_64`, `dosker-macos-arm64`, and `dosker-windows-x86_64`.
The `publish` job writes `SHA256SUMS` from bare archive filenames and runs
`gh release create`.

The version is the workspace version and equals the tag without the leading
`v`. The `publish` job fails when the tag and the version differ.

## Manual release commands

Use these commands when a hosted runner is unavailable. Run them from the
repository root and write the artifacts into `dist`. The Linux commands:

```sh
VERSION=0.1.4
mkdir -p dist
cargo build -p dosker-cli --release --target x86_64-unknown-linux-gnu --bin dosker --features qemu-engine
strip target/x86_64-unknown-linux-gnu/release/dosker
stage="$(mktemp -d)"
install -m 0755 target/x86_64-unknown-linux-gnu/release/dosker "$stage/dosker"
install -m 0644 README.md LICENSE-MIT LICENSE-APACHE "$stage/"
tar -czf "dist/dosker-$VERSION-linux-x86_64.tar.gz" -C "$stage" .
sha256sum "dist/dosker-$VERSION-linux-x86_64.tar.gz" > "dist/dosker-$VERSION-linux-x86_64.tar.gz.sha256"
```

The macOS commands replace the triple with `aarch64-apple-darwin` and
`sha256sum` with `shasum -a 256`.

The Windows commands replace the build target with `x86_64-pc-windows-msvc`,
skip `strip`, use `Compress-Archive` over `dosker.exe`, `README.md`,
`LICENSE-MIT`, and `LICENSE-APACHE`, and use `Get-FileHash -Algorithm SHA256`.
Write the archive and its `.sha256` into `dist`.

After all three artifacts are collected, run:

```sh
cd dist && sha256sum dosker-$VERSION-*.tar.gz dosker-$VERSION-*.zip > SHA256SUMS
```

Each `SHA256SUMS` line names a bare archive filename. Publish `SHA256SUMS` as
the only checksum manifest.

## Checksums

`SHA256SUMS` is the only accepted manifest, and it lists bare archive
filenames. Run the check from the directory that holds the archives. The
accepted commands are:

- Linux: `sha256sum -c SHA256SUMS`
- macOS: `shasum -a 256 -c SHA256SUMS`
- Windows: `Get-FileHash -Algorithm SHA256` and compare the printed hash with
  `SHA256SUMS`.

## Prerequisite handling

- **Release runners:** use the repository's GitHub Actions with GitHub-hosted
  runners: `ubuntu-latest` (Linux x86_64), `macos-15` (macOS arm64), and
  `windows-latest` (Windows x86_64). No self-hosted runners. The
  tag-triggered release workflow is the source of truth. If a hosted runner is
  unavailable or the release cannot run in Actions, produce the same artifacts
  locally with the commands in `docs/release.md` and record the gap in
  `docs/release-acceptance.md`; the acceptance distinguishes
  "built and verified locally" from "published by the release workflow".
- **Signing credentials:** signing credentials are optional; unsigned
  artifacts are the default; the roadmap's signing-credentials prerequisite is
  waived. Build unsigned artifacts with SHA-256 checksums by default. The
  macOS signing/notarization step is gated on `MACOS_CERTIFICATE`,
  `MACOS_CERTIFICATE_PASSWORD`, `MACOS_NOTARY_APPLE_ID`,
  `MACOS_NOTARY_PASSWORD`, and `MACOS_TEAM_ID`; the Windows signing step is
  gated on `WINDOWS_CERTIFICATE_BASE64` and `WINDOWS_CERTIFICATE_PASSWORD`.
  Both steps are skipped when their variables are absent, and the release
  notes and `docs/install.md` state `unsigned` and how to verify the checksum.
  The slice's acceptance does not require the secrets; the release workflow
  succeeds unsigned.
- **macOS CI runner with the engine builds:** use the existing
  `qemu-nightly.yml` hosted macOS arm64 runner for the QEMU suite and the
  Homebrew QEMU install. The nightly covers the QEMU suite and Homebrew QEMU
  only; it does not install DOSBox-X or Bochs. The DOSBox-X and Bochs suites
  run locally and are recorded in `docs/release-acceptance.md`. The release
  workflow builds the macOS artifact on `macos-15`. If the runner or an engine
  install is unavailable, run the suites locally and record the evidence in
  `docs/release-acceptance.md`; the local fallback on a macOS arm64 host always
  produces the macOS artifact, and only the "published by the release workflow"
  status is gated. Do not mark SC-006 or SC-007 met on a host that did not run
  them.

## macOS engine builds

The existing `qemu-nightly.yml` hosted macOS arm64 runner covers the QEMU
suite and the Homebrew QEMU install only. It does not install DOSBox-X or
Bochs. The DOSBox-X and Bochs suites run locally and are recorded in
`docs/release-acceptance.md`. The release workflow builds the macOS artifact
on `macos-15`.

A missing runner or engine install is recorded in
`docs/release-acceptance.md`. The local fallback on a macOS arm64 host always
produces the macOS artifact, and only the "published by the release workflow"
status is gated. Record the host. Do not mark SC-006 or SC-007 met on a host
that did not run them.

## Acceptance

See [the release acceptance record](release-acceptance.md).
"built and verified locally" and "published by the release workflow" are
distinct results.
