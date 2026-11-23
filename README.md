# DOSker

DOSker is a Docker-like system for pre-configured DOS systems. The host
runtime runs QEMU containers on Linux x86_64, macOS arm64, and Windows
x86_64, with a DOS serial agent for command execution and file transfer.

## Install

The `v0.1.2` release has artifacts for Linux x86_64, macOS arm64, and Windows
x86_64. Download the archive for your host and `SHA256SUMS` from the
[`v0.1.2` release](https://github.com/i-zhirov/dosker/releases/tag/v0.1.2).

The artifacts are unsigned. See [Install](docs/install.md) for engine setup,
`PATH` changes, and signing details.

### Linux x86_64

```sh
curl -fLO https://github.com/i-zhirov/dosker/releases/download/v0.1.2/dosker-0.1.2-linux-x86_64.tar.gz
curl -fLO https://github.com/i-zhirov/dosker/releases/download/v0.1.2/SHA256SUMS
grep 'dosker-0.1.2-linux-x86_64.tar.gz$' SHA256SUMS | sha256sum -c -
tar -xzf dosker-0.1.2-linux-x86_64.tar.gz
sudo install -m 0755 dosker /usr/local/bin/dosker
sudo apt-get install -y qemu-system-x86
dosker --version
dosker doctor
```

Run the `grep` command in the directory that holds the archive. `SHA256SUMS`
lists all three artifacts, so the command selects the Linux line only. The
command must print `OK`. The `apt-get` command is for Debian or Ubuntu. Use the
QEMU package for your distribution.

### macOS arm64

```sh
curl -fLO https://github.com/i-zhirov/dosker/releases/download/v0.1.2/dosker-0.1.2-macos-arm64.tar.gz
curl -fLO https://github.com/i-zhirov/dosker/releases/download/v0.1.2/SHA256SUMS
grep 'dosker-0.1.2-macos-arm64.tar.gz$' SHA256SUMS | shasum -a 256 -c -
tar -xzf dosker-0.1.2-macos-arm64.tar.gz
xattr -d com.apple.quarantine dosker 2>/dev/null || true
sudo install -m 0755 dosker /usr/local/bin/dosker
brew install qemu bochs
dosker --version
dosker doctor
```

Run the `grep` command in the directory that holds the archive. `SHA256SUMS`
lists all three artifacts, so the command selects the macOS line only. The
command must print `OK`. The `xattr` command removes the quarantine flag when
Gatekeeper blocks the unsigned binary. Install DOSBox-X from the official
release or Homebrew if you need that engine.

### Windows x86_64

```powershell
Invoke-WebRequest -Uri https://github.com/i-zhirov/dosker/releases/download/v0.1.2/dosker-0.1.2-windows-x86_64.zip -OutFile dosker-0.1.2-windows-x86_64.zip
Invoke-WebRequest -Uri https://github.com/i-zhirov/dosker/releases/download/v0.1.2/SHA256SUMS -OutFile SHA256SUMS
$expected = (Get-Content SHA256SUMS | Where-Object { $_ -match 'dosker-0.1.2-windows-x86_64\.zip$' }).Split()[0]
$actual = (Get-FileHash -Algorithm SHA256 .\dosker-0.1.2-windows-x86_64.zip).Hash.ToLower()
if ($actual -ne $expected) { throw 'Checksum mismatch' }
Expand-Archive -Path .\dosker-0.1.2-windows-x86_64.zip -DestinationPath .\dosker
.\dosker\dosker.exe --version
.\dosker\dosker.exe doctor
```

Run the commands from the download directory. The PowerShell script throws
`Checksum mismatch` when the archive hash differs from `SHA256SUMS`.

### Nix

The flake builds DOSker from source and adds QEMU to the runtime `PATH`. It
exposes macOS arm64, Linux x86_64, and Linux arm64.

```sh
nix run github:i-zhirov/dosker
nix profile install github:i-zhirov/dosker
```

`dosker build` still needs `DOSKER_AGENT_EXE` and a FreeDOS source. Run
`nix develop` in a checkout for the development environment.

## Example: a small DOS environment

The repository includes a runnable environment in
[`examples/hello-dos`](examples/hello-dos). The environment contains the
`Doskerfile`, its `Doskerfile.lock`, a setup script, a verification checklist,
and three host text files: `HELLO.TXT`, `NOTES.TXT`, and `UPLOAD.TXT`. The
Doskerfile starts FreeDOS, records a TCP port, attaches a data volume, and
runs a command on start. For a clean-checkout checklist, see
[Manual verification](examples/hello-dos/VERIFY.md).

```doskerfile
FROM freedos:1.3

ENV APP_NAME=HELLO
MACHINE memory=16M
ENGINE qemu
REQUIRES boot,serial,headless
EXPOSE 80
VOLUME D:
CMD ECHO HELLO FROM DOSKER
```

`FROM freedos:1.3` selects the built-in FreeDOS base. `ENV` records a value
in the image configuration. `MACHINE` sets 16 MiB of guest memory. `ENGINE`
pins QEMU. `REQUIRES` checks the QEMU capabilities. `EXPOSE` records a TCP
port. `VOLUME` declares a data drive. `CMD` runs when a container starts.

### Prepare the first build

The first build needs QEMU, the DOS agent, and a FreeDOS 1.3 boot floppy.
Run the commands from a repository checkout.

Install QEMU. On Debian or Ubuntu:

```sh
sudo apt-get install -y qemu-system-x86
```

On macOS:

```sh
brew install qemu
```

Build the DOS agent with OpenWatcom 2.0, or download the
`dosker-agent-binaries` artifact from a successful `agent-build` CI job.
Then set the paths. The fetch script downloads and verifies the FreeDOS
source:

```sh
export DOSKER_ROOT="${TMPDIR:-/tmp}/dosker-demo"
export DOSKER_AGENT_EXE="$PWD/agent/build/DOSKER.EXE"
export DOSKER_QEMU=qemu-system-i386
export DOSKER_FREEDOS_SOURCE="$(
  DOSKER_FREEDOS_SOURCE='https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/distributions/1.3/official/FD13-FloppyEdition.zip' \
  bash .github/scripts/fetch-freedos-source.sh
)"
```

The first build creates and stores the `freedos:1.3` base. Later builds reuse
it.

To automate the setup, source the script:

```sh
. examples/hello-dos/setup.sh
```

The script creates a temporary data root, downloads the DOS agent and FreeDOS
source, builds `hello:latest`, and creates the `hello-data` volume. It prints
the `dosker run` command to start the container. If you use the script, skip
the build and volume creation commands below.

### Build and run

```sh
dosker build -t hello:latest examples/hello-dos
dosker volume create --size 64M hello-data
dosker run -d --name hello-demo --boot-timeout 120 \
  -v hello-data:D: -p 8080:80 hello:latest
```

The example ships a lock file pinned to QEMU 11.1.3 and agent 1.0.0. If your
versions differ, run the first build with `--update`:

```sh
dosker build --update -t hello:latest examples/hello-dos
```

`dosker run` prints the container id. The `CMD` line runs on start. Show the
state, the log, and the port mapping:

```sh
dosker ps
dosker logs hello-demo
dosker port hello-demo
```

The log records the `CMD` line and its exit code. `EXPOSE 80` is metadata.
The example does not start a guest server, so the mapping is visible but no
service answers on port 8080.

### Run commands

```sh
dosker exec hello-demo ECHO HELLO
dosker exec hello-demo MD 'C:\APP'
dosker exec hello-demo DIR 'C:\APP'
```

The `exec` command returns the guest exit code. The default work directory is
`C:\`. Use `-w` to change it:

```sh
dosker exec -w 'C:\APP' hello-demo DIR
```

### Copy files in and out

Upload the example files:

```sh
dosker cp examples/hello-dos/HELLO.TXT 'hello-demo:C:\APP\HELLO.TXT'
dosker cp examples/hello-dos/NOTES.TXT 'hello-demo:C:\APP\NOTES.TXT'
dosker exec hello-demo TYPE 'C:\APP\HELLO.TXT'
```

Download a file:

```sh
dosker cp 'hello-demo:C:\APP\HELLO.TXT' "$DOSKER_ROOT/HELLO-FROM-DOS.TXT"
```

The `cp` command moves one file at a time. Quote DOS paths so the host shell
keeps the backslashes.

### Use the data volume

The image declares `VOLUME D:`. The run command overrides it with the named
volume `hello-data`. Upload a file to C:, copy it to D: inside the guest, and
read it back:

```sh
dosker cp examples/hello-dos/UPLOAD.TXT 'hello-demo:C:\UPLOAD.TXT'
dosker exec hello-demo COPY 'C:\UPLOAD.TXT' 'D:\DATA.TXT'
dosker exec hello-demo TYPE 'D:\DATA.TXT'
dosker volume ls
```

Stop and start the container. The volume keeps the file:

```sh
dosker stop hello-demo
dosker start hello-demo --boot-timeout 120
dosker exec hello-demo TYPE 'D:\DATA.TXT'
```

The volume stays attached until you remove the container.

### Attach

Open the line shell:

```sh
dosker attach hello-demo
```

Type `TYPE C:\APP\HELLO.TXT` and then `EXIT`. The container keeps running.

### Save and load

```sh
dosker save -o "$DOSKER_ROOT/hello.tar" hello:latest
dosker rmi hello:latest
dosker load "$DOSKER_ROOT/hello.tar"
dosker images
```

`save` writes an OCI image-layout tar. `load` restores the tags. `rmi` on a
tag only removes the tag. The manifest stays until it is untagged and unused.

### Clean up

```sh
dosker stop hello-demo
dosker rm hello-demo
dosker volume rm hello-data
dosker system prune
```

`system prune` removes stopped containers, untagged images, and stale cache
entries. It does not remove named volumes. Remove `hello-data` first.

### Notes

- QEMU is the only engine for the build path, in-DOS `RUN`, NAT, and named
  volumes. DOSBox-X and Bochs are runtime-only alternatives.
- This example uses runtime `dosker cp` for files. `COPY` and `RUN` in a
  Doskerfile are build instructions. See [Builds](docs/builds.md).
- The example uses a POSIX shell. Windows users can run the same commands in
  PowerShell with Windows paths and `$env:NAME` variables.

## Example: a self-contained DOS environment

[`examples/hello-dos-bundled`](examples/hello-dos-bundled) is the bundled
variant of the small example. It bakes three host text files into the
image with `COPY`, creates `DATA.TXT` with `RUN`, and records a start
command. It has no volume and needs no runtime `dosker cp`.

```doskerfile
FROM freedos:1.3

ENV APP_NAME=HELLO_BUNDLED
MACHINE memory=16M
ENGINE qemu
COPY HELLO.TXT C:/APP/HELLO.TXT
COPY NOTES.TXT C:/APP/NOTES.TXT
COPY UPLOAD.TXT C:/UPLOAD.TXT
RUN COPY C:\UPLOAD.TXT C:\APP\DATA.TXT
REQUIRES boot,serial,headless
CMD ECHO HELLO FROM BUNDLED DOSKER
```

The setup reuses the hello-dos environment preparation. It builds
`hello-bundled:latest` from the `COPY`/`RUN` Doskerfile and creates no
volume. It needs the same prerequisites as the hello-dos example.

```sh
. examples/hello-dos-bundled/setup.sh
```

Run the printed command, then inspect the image tree:

```sh
dosker exec hello-bundled-demo 'TYPE C:\APP\HELLO.TXT'
dosker exec hello-bundled-demo 'TYPE C:\APP\NOTES.TXT'
dosker exec hello-bundled-demo 'TYPE C:\APP\DATA.TXT'
dosker exec hello-bundled-demo 'DIR C:\APP'
```

The `COPY` steps create `HELLO.TXT` and `NOTES.TXT` in `C:\APP`. The
`RUN COPY` step creates `DATA.TXT` during the image build. The runtime
container has no volume, and `dosker cp` is not used.

The start command is in the image config. For this projected image,
`dosker logs` did not show the start command, so use `dosker exec` to
inspect the files.

The image can be saved and loaded without a volume. The archive is about
32 MiB.

[Manual verification](examples/hello-dos-bundled/VERIFY.md).

## Example: OpenWatcom C/C++ on FreeDOS with a toolchain volume

[`examples/openwatcom-dos`](examples/openwatcom-dos) runs the DOS-hosted
OpenWatcom v2 toolchain inside FreeDOS. A host source directory is mounted
at `D:` for source and build files. OpenWatcom is installed on the
`watcom-tools` volume at `E:`.

The setup downloads the pinned OpenWatcom v2 DOS package, verifies its
SHA-256, extracts the toolchain, creates the volume, and prints the run
command. It needs `curl`, `unzip`, `mtools`, and QEMU.

```sh
. examples/openwatcom-dos/setup.sh
```

Run the printed command, then build and run:

```sh
dosker exec openwatcom-demo 'D:\BUILD.BAT'
dosker exec openwatcom-demo 'D:\HELLO.EXE'
```

`BUILD.BAT` compiles `HELLO.C` with `WCC` and links it with `WLINK`. The
host `src` directory receives `hello.obj`, `hello.exe`, and `hello.map`.
QEMU vvfat may write the output names in lowercase.

A host edit is not live in QEMU vvfat. Recreate the container after
changing `HELLO.C` or `BUILD.BAT`:

```sh
dosker stop openwatcom-demo
dosker rm openwatcom-demo
# Run the setup's printed command again.
```

The image and the `watcom-tools` volume are reused. The source bind uses
QEMU vvfat beta and can corrupt host files. Use it for development only.
The source directory must contain only 8.3 names. Remove `.DS_Store` and
similar files. Do not put `.gitignore` in `src`.

The OpenWatcom package is about 122 MiB. The example caches the download
and does not commit the toolchain.

Clean up the container, the 256 MiB volume, and the data root:

```sh
dosker stop openwatcom-demo
dosker rm openwatcom-demo
dosker volume rm watcom-tools
rm -rf "$DOSKER_ROOT"
```

[Manual verification](examples/openwatcom-dos/VERIFY.md).

## Example: Bundled OpenWatcom C/C++ on FreeDOS

[`examples/openwatcom-dos-bundled`](examples/openwatcom-dos-bundled) is a
separate, self-contained workflow. Its setup builds a 256 MiB FreeDOS
base disk and imports it as `openwatcom-bundled-base:1.3`. The
`Doskerfile` then copies OpenWatcom to `C:\WATCOM`, copies `HELLO.C` and
`BUILD.BAT` to `C:\APP`, and runs `BUILD.BAT` during the image build. The
final image tree contains the compiler, the source, and the built
`HELLO.EXE`.

The image needs no `watcom-tools` volume and no source bind. The setup
uses the workspace `target/debug/dosker` or `target/release/dosker` when
present, builds the workspace binary if Rust is available, and otherwise
falls back to an installed `dosker`. It also needs `curl`, `unzip`,
`mtools`, and QEMU.

```sh
. examples/openwatcom-dos-bundled/setup.sh
```

Run the printed command, then verify the build-time compiler run and the
image:

```sh
dosker exec openwatcom-bundled-demo 'C:\APP\HELLO.EXE'
dosker exec openwatcom-bundled-demo 'DIR C:\WATCOM\BINW\WCC.EXE'
dosker exec openwatcom-bundled-demo 'DIR C:\APP'
```

The quoted paths are required in POSIX shells because the backslashes are
otherwise consumed by the shell. `BUILD.BAT` already ran during
`dosker build`; `C:\APP` contains `HELLO.OBJ`, `HELLO.MAP`, and
`HELLO.EXE`.

The source is baked into the image. To change `HELLO.C` or `BUILD.BAT`,
rebuild the base and the `COPY`/`RUN` image by sourcing the setup again.
This example does not use QEMU vvfat or a host bind.

The image can be saved and loaded without the base tag:

```sh
dosker save -o "$DOSKER_ROOT/openwatcom-bundled.tar" openwatcom-bundled:latest
dosker rmi openwatcom-bundled:latest
dosker rmi openwatcom-bundled-base:1.3
dosker load "$DOSKER_ROOT/openwatcom-bundled.tar"
dosker run -d --name openwatcom-bundled-loaded --boot-timeout 120 openwatcom-bundled:latest
```

The save archive is about 375 MiB. It contains the 256 MiB base raw disk
and the image tree with the toolchain, source, and build outputs. Loading
it restores only `openwatcom-bundled:latest`; the base tag is not needed.

The OpenWatcom package is about 122 MiB and is not committed. Publishing
a saved image redistributes OpenWatcom; check the OpenWatcom license
before distributing the archive. Do not commit the disk, archive, or
toolchain.

[Manual verification](examples/openwatcom-dos-bundled/VERIFY.md).

## CI setup

The CI workflow is in [`.github/workflows/ci.yml`](.github/workflows/ci.yml).
It runs formatting, the Rust workspace tests on Linux, macOS, and Windows, the
OpenWatcom agent build, and the QEMU conformance suites. A `nix` job builds
the flake and runs it on `ubuntu-latest` and `macos-15`.

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
- [Example environment](examples/hello-dos)
- [Example verification](examples/hello-dos/VERIFY.md)
- [Bundled hello-dos example](examples/hello-dos-bundled)
- [Bundled hello-dos verification](examples/hello-dos-bundled/VERIFY.md)
- [OpenWatcom volume example](examples/openwatcom-dos)
- [OpenWatcom volume verification](examples/openwatcom-dos/VERIFY.md)
- [Bundled OpenWatcom example](examples/openwatcom-dos-bundled)
- [Bundled OpenWatcom verification](examples/openwatcom-dos-bundled/VERIFY.md)
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
