# DOSker

DOSker is a Docker-like system for pre-configured DOS systems. The host
runtime runs QEMU containers on Linux x86_64, macOS arm64, and Windows
x86_64, with a DOS serial agent for command execution and file transfer.

## Install

Install a release artifact for Linux x86_64, macOS arm64, or Windows x86_64,
verify its checksum, and run `dosker doctor`. See
[Install](docs/install.md) for the steps.

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

For CI the value can be:

- a FreeDOS Floppy Edition ZIP URL;
- a raw boot floppy image URL;
- a local path to a boot floppy image.

`.github/scripts/fetch-freedos-source.sh` resolves the value to a local boot
floppy image before the build. It verifies and extracts a ZIP, downloads a raw
image, or uses a local path.

The CLI loader is stricter than CI: when it has to build the base, it requires
`DOSKER_FREEDOS_SOURCE` to name a local boot floppy image. A URL or ZIP is a CI
repository-variable value only. A stored `freedos:1.3` base is reused without
the source or agent inputs. The same local path feeds a first build or a
rebuild.

The base build also needs the DOS agent. It reads `agent/build/DOSKER.EXE` by
default, or the path in `DOSKER_AGENT_EXE`. See
[FreeDOS base images](docs/base-images.md) for the pinned inputs, the source
layout, and the update procedure.

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

### Images

DOSker stores images in the local OCI layout and provides `image import`,
`images`, `tag`, `rmi`, `save`, and `load`. `dosker save` writes a tagged image
to an OCI image-layout tar and `dosker load` restores its tags.
`dosker system prune` removes stopped containers, untagged images, and stale
cache entries. See [Images](docs/images.md) for the store layout, the media
types, and the untag-only `rmi` behavior.

### Engines

`qemu`, `dosbox-x`, and `bochs` have runtime adapters. See
[Engines](docs/engines.md) for engine selection, the capability table, and
the runtime behavior. DOSBox-X and Bochs are runtime engines for the
capabilities listed in the capability table.

### Volumes and networking

Named volumes attach a FAT disk image to a running container, and published
ports forward a guest TCP port to the host. Use `-v` to mount a volume, `-p` to
publish a port, `dosker volume` to manage volumes, and `dosker port` to show
the effective mappings. See
[Volumes and network](docs/volumes-and-network.md) for the layout, drive, and
NAT rules.

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

The job fails when any value is empty. The first three describe the QEMU
installer. `DOSKER_QEMU_IMAGE_URL` describes the prepared image.

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

### QEMU net image URL variable

Set the repository variable `DOSKER_QEMU_NET_IMAGE_URL` to a raw `.img` URL.
The image is a booter image that starts an NE2000 packet driver and a TCP
server on port 80. The `agent-qemu` and nightly macOS jobs download it to
`agent/fixtures/dosker-net.img` and set `DOSKER_QEMU_NET_IMAGE`.

Set it with `gh`:

```sh
gh variable set DOSKER_QEMU_NET_IMAGE_URL \
  --body 'https://example.com/dosker-net.img'
```

### Pinned Floppy Edition ZIP

CI uses the official FreeDOS 1.3 Floppy Edition archive:

```text
https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/distributions/1.3/official/FD13-FloppyEdition.zip
```

SHA-256:

```text
75a4e11a7fce6f124e20927b3022b4b715a2a3f7324c5f5bfea42d90d80eb072
```

The fetch script checks this hash with `sha256sum` or `shasum -a 256` before
it extracts the boot floppy.

### OpenWatcom install

The `agent-build` job downloads the pinned OpenWatcom 2.0 Linux archive:

```text
https://github.com/open-watcom/open-watcom-v2/releases/download/2026-11-01-Build/open-watcom-2_0-c-linux-x64
```

The archive is a self-extracting ZIP. CI installs `unzip`, extracts the
archive directly into `/usr/watcom`, makes the `binl64` and `binl` tools
executable, and sets `PATH` and `INCLUDE`. This avoids the installer's
terminfo floating-point exception on the runner.

The `agent-build` job uploads `DOSKER.EXE`, `EXITCODE.EXE`, `SETBOOT.EXE`, and `REBOOT.EXE`. The
`agent-qemu` job downloads that artifact, sets `DOSKER_SKIP_BUILD=1`, builds
the FreeDOS image, and runs the QEMU conformance and self-check scripts. It
then runs the Rust QEMU runtime suite.

## Security

DOSker does not sandbox untrusted images. QEMU TCG is not a security boundary.
Use an outer virtual machine or an isolated CI runner for untrusted input.

## License

DOSker is dual-licensed under the MIT License or the Apache License 2.0
(SPDX: MIT OR Apache-2.0), at your option. See LICENSE-MIT and LICENSE-APACHE.

## Documentation

- [Install](docs/install.md)
- [Release](docs/release.md)
- [Multi-engine decision](docs/multi-engine-decision.md)
- [Design](.sdd/specs/2026-11-01-dosker/design.md)
- [Agent protocol](.sdd/specs/2026-11-01-dosker/contracts/agent-protocol.md)
- [Roadmap](.sdd/specs/2026-11-01-dosker/roadmap.md)
- [DOS agent plan](.sdd/specs/2026-11-01-dosker/plan-dos-agent-serial.md)
- [Builds](docs/builds.md)
- [FreeDOS base images](docs/base-images.md)
- [Volumes and network](docs/volumes-and-network.md)
- [Windows runtime](docs/windows.md)
