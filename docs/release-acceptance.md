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
| SC-001 | Linux x86_64 | `DOSKER_INTEGRATION=1` with `DOSKER_QEMU_IMAGE` and `DOSKER_QEMU`; `qemu_runtime` | not run on this host |
| SC-001 | macOS arm64 | `DOSKER_INTEGRATION=1` with `DOSKER_QEMU_IMAGE` and `DOSKER_QEMU`; `qemu_runtime` | pass, local; QEMU 11.1.1; nightly pin 11.1.3 not run |
| SC-001 | Windows x86_64 | `DOSKER_INTEGRATION=1` with `DOSKER_QEMU_IMAGE`, `DOSKER_QEMU`, `DOSKER_QEMU_VERSION=11.1.3`, and `DOSKER_QEMU_SHA256`; `windows_qemu_runtime` | not run on this host |
| SC-002 | Linux x86_64 | `DOSKER_INTEGRATION=1`; `qemu_build` | not run on this host |
| SC-003 | macOS arm64 | `DOSKER_INTEGRATION=1` with `DOSKER_DOSBOX_X`, `DOSKER_DOSBOX_X_VERSION=2026.10.01`, `DOSKER_DOSBOX_X_SHA256`, and `DOSKER_DOSBOX_X_IMAGE`; `dosbox_x_runtime` | pass |
| SC-004 | all | `cargo test -p dosker-core --test lockfile` | pass |
| SC-004 | Linux x86_64 | `DOSKER_INTEGRATION=1` for `qemu_freedos` | not run on this host |
| SC-005 | all | `cargo test -p dosker-engine` and `cargo test -p dosker-cli --test cli_requires` | pass |
| SC-006 | Linux x86_64 | workspace suite, then `qemu_runtime`, `qemu_volumes`, `qemu_freedos`, `qemu_run`, and `qemu_ports` | not run on this host |
| SC-006 | macOS arm64 | workspace suite, then `qemu_runtime`, `qemu_volumes`, `qemu_run`, and `qemu_ports` | partial: `qemu_run` not run on this host, missing `SETBOOT.EXE` and `REBOOT.EXE` |
| SC-007 | Linux x86_64 | `DOSKER_INTEGRATION=1` for `qemu_runtime` | not run on this host |
| SC-007 | macOS arm64 | `DOSKER_INTEGRATION=1` for `qemu_runtime` | pass, local |
| SC-007 | Windows x86_64 | `DOSKER_INTEGRATION=1` for `windows_qemu_runtime` | not run on this host |
| Release workflow | GitHub-hosted runners | push `v0.1.0` or `workflow_dispatch` with `tag=v0.1.0` | not run; configuration linted, no publication |
| Release artifacts | macOS arm64 | manual macOS commands from `docs/release.md` | built and verified locally |
| Release artifacts | Linux x86_64 | manual Linux commands from `docs/release.md` | not run on this host |
| Release artifacts | Windows x86_64 | manual Windows commands from `docs/release.md` | not run on this host |
| Save/load | macOS arm64 | `cli_save_load` | pass |
| Prune | macOS arm64 | `cli_system` | pass |
| Engine discovery | macOS arm64 | `cli_engine_pins` and `dosker engines` | pass |
| Security | macOS arm64 | README check and `dosker doctor` | pass |
| Decision | macOS arm64 | `test -s docs/multi-engine-decision.md` | pass |

The release workflow publishes `dosker-0.1.0-linux-x86_64.tar.gz`,
`dosker-0.1.0-macos-arm64.tar.gz`, `dosker-0.1.0-windows-x86_64.zip`, their
`.sha256` files, and `SHA256SUMS` through `gh release create`. The
`workflow_dispatch` input is `tag=v0.1.0`. None of these ran from the release
workflow on this host.

## Local record

Host: the macOS arm64 host is the only available release host (Darwin,
arm64). Installed QEMU: `11.1.1`. Installed
DOSBox-X: `2026.10.01` SDL2, checksum
`6934abe66fe55d71487ded37e097416de65981250c902a9cca2bfd1284e98c96`.
Installed Bochs: `3.1`, checksum
`8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c`.
The optional Bochs gdbstub build is at `$HOME/bochs-gdbstub/bin/bochs`.

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
| `DOSKER_INTEGRATION=1` `qemu_runtime` | pass | SC-001 and SC-007 on macOS arm64 |
| `DOSKER_INTEGRATION=1` `qemu_volumes` | pass | SC-006 on macOS arm64 |
| `DOSKER_INTEGRATION=1` `qemu_ports` | pass | SC-006 on macOS arm64 |
| `DOSKER_INTEGRATION=1` `dosbox_x_runtime` | pass | SC-003 |
| `DOSKER_INTEGRATION=1` `bochs_runtime` | pass | Bochs runtime evidence |
| `DOSKER_INTEGRATION=1` `gdbstub_build` | pass | optional Bochs gdbstub build |
| `qemu_run` | not run on this host | missing `SETBOOT.EXE` and `REBOOT.EXE` |
| `qemu_freedos` | not run on this host | Linux suite; no FreeDOS source gate |
| `windows_qemu_runtime` | not run on this host | no Windows x86_64 host |
| `actionlint` | pass | release, CI, and nightly workflows |
| macOS manual release commands | built and verified locally | `dosker-0.1.0-macos-arm64.tar.gz` |
| `shasum -a 256 -c SHA256SUMS` | pass | local macOS-only manifest |
| extracted `dosker --version` and `dosker doctor` | pass | `dosker 0.1.0`; QEMU, DOSBox-X, Bochs found |

The macOS artifact is unsigned. The binary has only the ad-hoc linker
signature from the local build, and no Developer ID identity or notarization.
The artifact name is `dosker-0.1.0-macos-arm64.tar.gz`. The local
`SHA256SUMS` lists only this archive because the Linux and Windows archives
were not built on this host.

## Waived and narrowed prerequisites

The roadmap's signing-credentials prerequisite is waived. Signing credentials
are optional and unsigned artifacts are the default. The macOS and Windows
signing steps are skipped when their secrets are absent.

The Linux QEMU version pin is narrowed to the opt-in `DOSKER_QEMU_VERSION`.
Linux records the installed `qemu-system-i386 --version`; macOS and Windows
enforce the exact release pin. The roadmap is left unchanged. The roadmap
sentence `Engine discovery works on Linux, macOS, and Windows with pinned versions.` is narrowed on Linux as described here.

## Gaps

- No Linux x86_64 host is available. This gates the Linux artifact,
  SC-001 on Linux, SC-002, SC-004 on Linux, SC-006 on Linux, and SC-007 on
  Linux.
- No Windows x86_64 host is available. This gates the Windows artifact,
  SC-001 on Windows, and SC-007 on Windows.
- The `qemu_run` helpers `SETBOOT.EXE` and `REBOOT.EXE` are absent. This
  gates the macOS `qemu_run` row of SC-006.
- The local QEMU is `11.1.1`. The nightly `DOSKER_QEMU_VERSION=11.1.3` pin was
  not run on this host.
- No release workflow run is recorded. The workflow was checked with
  `actionlint`, but no artifact is `published by the release workflow`.
- No signing secrets are available. The macOS artifact is unsigned, as
  stated in the release notes and `docs/install.md`.
