# Release acceptance

## Rule

A host is marked met only when the command ran on that host and the result is
recorded. A local build is recorded as `built and verified locally`. A
workflow run is recorded as `published by the release workflow`. A row marked
`not run on this host` is not met.

A local build on the required host always satisfies the Definition Of Done for
artifact existence. Record the host. Only the `published by the release
workflow` status is gated.

An integration suite returns early and reports success when
`DOSKER_INTEGRATION` is not exactly `1`. Record that result as
`not run on this host`, never `pass`. When the gate is set and a required
image, engine, or helper is absent, the suite panics. Record `fail` and the
missing prerequisite. An integration suite is `pass` only when it ran with
`DOSKER_INTEGRATION=1` on that host. A mock or unit suite or a documentation
check is `pass` when its command exits 0 on that host.

## Verification matrix

| Criterion | Host | Command | Result |
| --- | --- | --- | --- |
| SC-001 | Linux x86_64 | `DOSKER_INTEGRATION=1` with `DOSKER_QEMU_IMAGE` and `DOSKER_QEMU`; `qemu_runtime` | pass, CI run 44902589361; QEMU 10.2.1 |
| SC-001 | macOS arm64 | `DOSKER_INTEGRATION=1` with `DOSKER_QEMU_IMAGE` and `DOSKER_QEMU`; `qemu_runtime` | pass, local (QEMU 11.1.1) and nightly run 44903500634 (QEMU 11.1.3) |
| SC-001 | Windows x86_64 | `DOSKER_INTEGRATION=1` with `DOSKER_QEMU_IMAGE`, `DOSKER_QEMU`, `DOSKER_QEMU_VERSION=11.1.0`, and `DOSKER_QEMU_SHA256`; `windows_qemu_runtime` | pass, CI run 44902589361 |
| SC-002 | Linux x86_64 | `DOSKER_INTEGRATION=1`; `qemu_build` | pass, CI run 44902589361 |
| SC-003 | macOS arm64 | `DOSKER_INTEGRATION=1` with `DOSKER_DOSBOX_X`, `DOSKER_DOSBOX_X_VERSION=2026.10.01`, `DOSKER_DOSBOX_X_SHA256`, and `DOSKER_DOSBOX_X_IMAGE`; `dosbox_x_runtime` | pass |
| SC-004 | all | `cargo test -p dosker-core --test lockfile` | pass, CI and local |
| SC-004 | Linux x86_64 | `DOSKER_INTEGRATION=1` for `qemu_freedos` | pass, CI run 44902589361 |
| SC-005 | all | `cargo test -p dosker-engine` and `cargo test -p dosker-cli --test cli_requires` | pass, CI and local |
| SC-006 | Linux x86_64 | workspace suite, then `qemu_runtime`, `qemu_volumes`, `qemu_freedos`, `qemu_run`, and `qemu_ports` | pass, CI run 44902589361 |
| SC-006 | macOS arm64 | workspace suite, then `qemu_runtime`, `qemu_volumes`, `qemu_run`, and `qemu_ports` | pass, local and nightly run 44903500634 |
| SC-007 | Linux x86_64 | `DOSKER_INTEGRATION=1` for `qemu_runtime` | pass, CI run 44902589361 |
| SC-007 | macOS arm64 | `DOSKER_INTEGRATION=1` for `qemu_runtime` | pass, local and nightly run 44903500634 |
| SC-007 | Windows x86_64 | `DOSKER_INTEGRATION=1` for `windows_qemu_runtime` | pass, CI run 44902589361 |
| Release workflow | GitHub-hosted runners | push `v0.1.0` or `workflow_dispatch` with `tag=v0.1.0` | pass; run 44904029239 published `v0.1.0` |
| Release artifacts | Linux x86_64 | release workflow | published by the release workflow |
| Release artifacts | macOS arm64 | release workflow | published by the release workflow; local build also verified |
| Release artifacts | Windows x86_64 | release workflow | published by the release workflow |
| Published checksums | macOS arm64 | `shasum -a 256 -c SHA256SUMS` | pass, all three archives |
| Save/load | macOS arm64 | `cli_save_load` | pass |
| Prune | macOS arm64 | `cli_system` | pass |
| Engine discovery | macOS arm64 | `cli_engine_pins` and `dosker engines` | pass |
| Security | macOS arm64 | README check and `dosker doctor` | pass |
| Decision | macOS arm64 | `test -s docs/multi-engine-decision.md` | pass |

The release workflow publishes `dosker-0.1.0-linux-x86_64.tar.gz`,
`dosker-0.1.0-macos-arm64.tar.gz`, `dosker-0.1.0-windows-x86_64.zip`, their
`.sha256` files, and `SHA256SUMS` through `gh release create`. Run 44904029239
published them for `v0.1.0`. The three archives were downloaded from the
release, `shasum -a 256 -c SHA256SUMS` passed, and `file` reported ELF 64-bit
x86-64, Mach-O 64-bit arm64, and PE32+ x86-64. The macOS binary printed
`dosker 0.1.0`.

## Verification hosts

Local host: the macOS arm64 host is Darwin, arm64. Installed QEMU: `11.1.1`. Installed
DOSBox-X: `2026.10.01` SDL2, checksum
`6934abe66fe55d71487ded37e097416de65981250c902a9cca2bfd1284e98c96`.
Installed Bochs: `3.1`, checksum
`8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c`.
The optional Bochs gdbstub build is at `$HOME/bochs-gdbstub/bin/bochs`.
The `SETBOOT.EXE` and `REBOOT.EXE` helpers came from the CI
`dosker-agent-binaries` artifact; `agent/build.sh` produces the same files
from `agent/test/setboot.c` and `agent/test/reboot.c`.

CI hosts: GitHub-hosted `ubuntu-latest` with QEMU `10.2.1`, `macos-15` with
Homebrew QEMU `11.1.3`, and `windows-latest` with QEMU `11.1.0` from the
pinned Windows archive.

### Local record

| Command | Result | Evidence |
| --- | --- | --- |
| `cargo test --workspace --features dosker-cli/mock-engine` | pass | unit and mock suites |
| `cargo test -p dosker-core --test lockfile` | pass | SC-004 lock format |
| `cargo test -p dosker-engine` | pass | SC-005 capabilities |
| `cargo test -p dosker-cli --test cli_requires` | pass | SC-005 `REQUIRES` |
| `cargo test -p dosker-cli --features mock-engine --test cli_save_load` | pass | save/load |
| `cargo test -p dosker-cli --features mock-engine --test cli_system` | pass | prune |
| `cargo test -p dosker-cli --test cli_engine_pins` | pass | engine pins |
| `cargo test -p dosker-cli --test cli_engines` | pass | `engines` and `doctor` |
| `DOSKER_INTEGRATION=1` `qemu_runtime` | pass | SC-001 and SC-007 on macOS arm64; QEMU 11.1.1 |
| `DOSKER_INTEGRATION=1` `qemu_volumes` | pass | SC-006 on macOS arm64 |
| `DOSKER_INTEGRATION=1` `qemu_ports` | pass | SC-006 on macOS arm64 |
| `DOSKER_INTEGRATION=1` `dosbox_x_runtime` | pass | SC-003 |
| `DOSKER_INTEGRATION=1` `bochs_runtime` | pass | Bochs runtime evidence |
| `DOSKER_INTEGRATION=1` `gdbstub_build` | pass | optional Bochs gdbstub build |
| `DOSKER_INTEGRATION=1` `qemu_run` | pass | SC-006 in-DOS RUN and guest reboot |
| `actionlint` | pass | release, CI, and nightly workflows |
| macOS manual release commands | built and verified locally | `dosker-0.1.0-macos-arm64.tar.gz` |
| published `SHA256SUMS` | pass | all three archives; macOS `dosker --version` prints `dosker 0.1.0` |

### CI record

| Run | Result | Evidence |
| --- | --- | --- |
| CI run 44902589361 | pass | `fmt`, `preflight`, `test` on Ubuntu, macOS, and Windows, `agent-build`, `agent-qemu`, `windows-qemu`, and FreeDOS determinism |
| `agent-qemu` | pass | `qemu_runtime`, `qemu_volumes`, `qemu_freedos`, `qemu_build`, `qemu_run`, and `qemu_ports`; QEMU 10.2.1 |
| `windows-qemu` | pass | `windows_qemu_runtime`; QEMU 11.1.0 |
| `test (windows-latest)` | pass | unit, mock, adapter, transport, and detached lifecycle suites |
| QEMU nightly run 44903500634 | pass | Homebrew QEMU 11.1.3 pin; `qemu_runtime`, `qemu_volumes`, `qemu_run`, and `qemu_ports` |
| Release run 44904029239 | pass | published `v0.1.0`, all archives, `.sha256` files, and `SHA256SUMS` |

The published artifacts are unsigned. The macOS binary has only the ad-hoc
linker signature from the build, and no Developer ID identity or
notarization. The release notes state `unsigned` for all three hosts. The
published `SHA256SUMS` covers all three archives.

## Release 0.1.1

- Workflow run 45018165204 published `v0.1.1` from commit `8d487a8`.
- The run published `dosker-0.1.1-linux-x86_64.tar.gz`,
  `dosker-0.1.1-macos-arm64.tar.gz`, `dosker-0.1.1-windows-x86_64.zip`,
  their `.sha256` files, and `SHA256SUMS`.
- `shasum -a 256 -c SHA256SUMS` passed for all three archives.
- The macOS binary printed `dosker 0.1.1`.
- The Homebrew tap formula was updated in commit `a77aa6e`.
- `brew reinstall dosker` installed `0.1.1` from the tap.
- The installed binary passed `cd dosker` and `dir` in an attached shell
  without `BAD_PATH`.
- The release is unsigned.

## Release 0.1.4

- Workflow run 45239581834 published `v0.1.4` from commit `daaeb81`.
- The run published `dosker-0.1.4-linux-x86_64.tar.gz`,
  `dosker-0.1.4-macos-arm64.tar.gz`, `dosker-0.1.4-windows-x86_64.zip`, and
  `SHA256SUMS`.
- The macOS binary printed `dosker 0.1.4`.
- The registry repository `i-zhirov/dosker-registry` and its Pages site are
  live. Registry publish run 45243208834 built `i-zhirov/hello-dos:1.0.0`,
  deployed the site, and the catalog and license record pass the schemas.
- On this macOS arm64 host, `dosker pull i-zhirov/hello-dos:latest` pulled
  `sha256:9e818d96d3982b36533ced11fb58db4d3b0f1a063728a45739a71b6fe5a20c86` and
  verified the asset `sha256:c883423a58479ccdc91ec014b3612e8391869ee7ced16a9265d4efea51c34eba`.
  The image was not started on this host.
- The `validate.yml` workflow was not run on a pull request.
- The `takedown` workflow is deployed; no revocation has been exercised.

## Takedown exercise (2026-11-23)

- Test image `i-zhirov/takedown-test:0.1.0`, digest
  `sha256:7766235096ac413c0564458d19e124c2dc768543d30fbd28c1ed4420c4490341`.
- Publish run 45245988511 built and published it; a client `pull` on this macOS
  arm64 host verified the image and asset.
- The revocation was recorded in `revoked.toml`; publish run 45246219919
  omitted the image from the catalog and its index. Takedown run 45246343466
  deleted the tar asset (the sidecar stayed) and redeployed Pages.
- A fresh `dosker pull i-zhirov/takedown-test:takedown-test` failed with
  `image not found`. The local tag from before the takedown stayed in the local
  store.
- The exercise found a re-publish gap. Publish dispatch 45246649797 rebuilt the
  same version to a new digest, and the site listed the image again, because
  the CI release directory started empty and skipped the version-immutability
  check. The publish workflow now downloads the existing sidecars before
  `scripts/publish.py`; a same-version rebuild fails with "bump the version".
  The `test_a_different_digest_fails` test covers the script behavior.
- The test image, its revocation record, and its Release were removed after the
  exercise. The site lists only `i-zhirov/hello-dos`, and its pull still
  verifies the digest.

## Takedown content-immutability exercise (2026-11-23)

- Test image `i-zhirov/takedown-test:0.1.0`, digest
  `sha256:487aef351d55c6aab285ff7904897a4b8988fa72f6036c5af621bc3c03abb305`,
  content digest `sha256:25d6f56fb7c3e9d9a865036c8369a5a349b8533a036a03244e4a7a02efcfd181`.
- Publish run 45258017912 wrote the sidecar with the content digest. The
  revocation omitted the image, and takedown run 45259437696 stamped the
  sidecar and deleted the tar.
- The version was then bumped to 0.1.1. Publish run 45259644181 failed with
  `image i-zhirov/takedown-test content is revoked; change the content before
  publishing a new version`. The release kept only the 0.1.0 sidecar, and the
  site stayed on `i-zhirov/hello-dos`.
- The test image, its revocation record, and its Release were removed after the
  exercise.

## Registry CI gate exercise (2026-11-23)

- Positive PR #1: validate run 45261051551 passed the metadata checks, the
  pinned release install, the FreeDOS base and DOS agent preparation, and
  `dosker registry validate`.
- Negative PR #2: validate run 45261448147 failed with
  `duplicate tag latest: images/i-zhirov/bad/registry.toml and
  images/i-zhirov/hello-dos/registry.toml`. The bad-license fixture is rejected
  by the gate.
- Both pull requests were closed without merging and their branches deleted.

## Release 0.1.2

- Workflow run 45027052360 published `v0.1.2` from commit `c3e64d0`.
- The run published `dosker-0.1.2-linux-x86_64.tar.gz`,
  `dosker-0.1.2-macos-arm64.tar.gz`, `dosker-0.1.2-windows-x86_64.zip`,
  their `.sha256` files, and `SHA256SUMS`.
- `shasum -a 256 -c SHA256SUMS` passed for all three archives.
- The macOS binary printed `dosker 0.1.2`.
- The Homebrew tap formula was updated in commit `64f7615`.
- `brew reinstall dosker` installed `0.1.2` from the tap.
- The installed help shows a description for each command and no
  `Arguments for` placeholders.
- The release is unsigned.

## Waived and narrowed prerequisites

The roadmap's signing-credentials prerequisite is waived. Signing credentials
are optional and unsigned artifacts are the default. The macOS and Windows
signing steps are skipped when their secrets are absent.

The Linux QEMU version pin is narrowed to the opt-in `DOSKER_QEMU_VERSION`.
Linux records the installed `qemu-system-i386 --version`; macOS enforces the
exact `11.1.3` release pin, and Windows enforces the exact `11.1.0` release
pin with its checksum. The roadmap is left unchanged; this acceptance records
the waiver and the narrowing. The roadmap sentence `Engine discovery works on Linux, macOS, and Windows with pinned versions.` is narrowed on Linux as described here.

## Resolved findings

- The Windows volume suites initially failed because holder metadata lived in
  the locked `.lock` file. The store now writes `holder.json`, and the Windows
  unit and mock suites pass.

## Gaps

No open gaps. Signing credentials remain optional and waived, so the published
artifacts are unsigned.
