# DOSker

DOSker is a Docker-like system for pre-configured DOS systems. The host
runtime runs QEMU containers on Linux x86_64, macOS arm64, and Windows
x86_64, with a DOS serial agent for command execution and file transfer.

## CI setup

The CI workflow is in [`.github/workflows/ci.yml`](.github/workflows/ci.yml).
It runs formatting, the Rust workspace tests on Linux, macOS, and Windows, the
OpenWatcom agent build, and the QEMU conformance suites.

A `preflight` job checks all required QEMU repository variables before
`agent-qemu` or `windows-qemu` starts. The macOS nightly has the same check.
It reports every missing variable and its expected format in one message.

### FreeDOS source variable

Set the repository variable `DOSKER_FREEDOS_SOURCE` in
*Settings > Secrets and variables > Actions > Variables*. It is not a secret.
The `agent-qemu` job fails when the variable is absent.

Set it with `gh`:

```sh
gh variable set DOSKER_FREEDOS_SOURCE \
  --body 'https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/distributions/1.3/official/FD13-FloppyEdition.zip'
```

The value can be:

- a FreeDOS Floppy Edition ZIP URL;
- a raw boot floppy image URL;
- a local path to a boot floppy image.

For a ZIP, CI verifies the archive and extracts `144m/x86BOOT.img` before it
builds the base image.

### QEMU runtime suites

The `agent-qemu` job runs the Rust QEMU runtime integration suite on Linux
after the conformance and self-check scripts. The suite uses the prepared
fixture at `agent/fixtures/dosker-agent.img`. It runs:

```sh
DOSKER_INTEGRATION=1 \
DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img \
DOSKER_QEMU=qemu-system-i386 \
cargo test -p dosker-cli --features qemu-engine --test qemu_runtime -- --nocapture
```

The macOS arm64 suite runs in
[`.github/workflows/qemu-nightly.yml`](.github/workflows/qemu-nightly.yml)
at 03:00 UTC each day and on manual dispatch. macOS arm64 uses TCG, so the
job is slower and runs nightly.

### Windows runtime

Windows x86_64 uses a named-pipe control channel, loopback TCP engine
endpoints, and a Job Object for cleanup. See
[Windows runtime](docs/windows.md) for the full host behavior and environment
variables.

The `windows-qemu` CI job requires these repository variables:

- `DOSKER_WINDOWS_QEMU_URL`
- `DOSKER_WINDOWS_QEMU_SHA256`
- `DOSKER_WINDOWS_QEMU_VERSION`
- `DOSKER_QEMU_IMAGE_URL`

The job fails when any value is empty. The first three describe the QEMU ZIP
archive. `DOSKER_QEMU_IMAGE_URL` describes the prepared image.

### QEMU image URL variable

Set the repository variable `DOSKER_QEMU_IMAGE_URL` in
*Settings > Secrets and variables > Actions > Variables*. It is not a secret.
The nightly workflow fails when the variable is absent.

Set it with `gh`:

```sh
gh variable set DOSKER_QEMU_IMAGE_URL \
  --body 'https://example.com/dosker-agent.img'
```

The value can be a ZIP archive that contains a `.img` file or a raw image URL.
The workflow downloads and extracts a ZIP archive. It downloads a raw image
directly.

### Pinned Floppy Edition ZIP

CI uses the official FreeDOS 1.3 Floppy Edition archive:

```text
https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/distributions/1.3/official/FD13-FloppyEdition.zip
```

SHA-256:

```text
75a4e11a7fce6f124e20927b3022b4b715a2a3f7324c5f5bfea42d90d80eb072
```

The workflow checks this hash with `sha256sum -c` before it extracts the boot
floppy.

### OpenWatcom install

The `agent-build` job downloads the pinned OpenWatcom 2.0 Linux archive:

```text
https://github.com/open-watcom/open-watcom-v2/releases/download/2026-11-01-Build/open-watcom-2_0-c-linux-x64
```

The archive is a self-extracting ZIP. CI installs `unzip`, extracts the
archive directly into `/usr/watcom`, makes the `binl64` and `binl` tools
executable, and sets `PATH` and `INCLUDE`. This avoids the installer's
terminfo floating-point exception on the runner.

The `agent-build` job uploads `DOSKER.EXE` and `EXITCODE.EXE`. The
`agent-qemu` job downloads that artifact, sets `DOSKER_SKIP_BUILD=1`, builds
the FreeDOS image, and runs the QEMU conformance and self-check scripts. It
then runs the Rust QEMU runtime suite.

## Documentation

- [Design](.sdd/specs/2026-11-01-dosker/design.md)
- [Agent protocol](.sdd/specs/2026-11-01-dosker/contracts/agent-protocol.md)
- [Roadmap](.sdd/specs/2026-11-01-dosker/roadmap.md)
- [DOS agent plan](.sdd/specs/2026-11-01-dosker/plan-dos-agent-serial.md)
- [Windows runtime](docs/windows.md)
