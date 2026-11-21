# Engines

An engine runs a DOS image. DOSker implements an adapter for each engine it
can start. This page describes engine selection, the capability model,
and the QEMU, DOSBox-X, and Bochs runtime engines.

## Engine selection

- The engines qemu, dosbox-x, and bochs have adapters.
- `--engine` on `create` and `run` overrides the image `ENGINE` pin for one
  command.
- Without `--engine` and without an `ENGINE` pin, the engine is `qemu`.
- In-DOS `RUN` and the build path stay QEMU-only. A build whose final engine
  is `dosbox-x` or `bochs` fails with `engine dosbox-x: unsupported engine`
  or `engine bochs: unsupported engine` after the `REQUIRES` check passes,
  before the step loop.
- The post-M4 decision to keep multi-engine is recorded in
  [docs/multi-engine-decision.md](multi-engine-decision.md).

## ENGINE-CONFIG

An `ENGINE-CONFIG` line carries engine-specific configuration text. The raw
text is passed to the named adapter.

- QEMU splits the raw line into command-line arguments with shell-like
  quoting. Whitespace separates arguments; single and double quotes group; a
  backslash escapes the next character outside single quotes; an unterminated
  quote is an error.
- DOSBox-X appends raw lines verbatim before `[autoexec]`. A line may be a
  `[section]` header or a `key=value` pair.
- Bochs appends raw lines verbatim after the fixed `bochsrc` keys. A line is
  a `bochsrc` key. A later key overrides an earlier one.

A DOSBox-X example:

```doskerfile
ENGINE-CONFIG dosbox-x [cpu]
ENGINE-CONFIG dosbox-x cputype = 486
```

A Bochs example:

```doskerfile
ENGINE-CONFIG bochs clock: sync=realtime, time0=local
```

## REQUIRES

`REQUIRES` names capabilities a Doskerfile needs. `create`, `run`, and
`dosker build` enforce it after engine selection and before any VM starts.
The final selected engine must satisfy every `REQUIRES` in the Doskerfile. A
requirement is not checked against the engine in effect at its own step.

A capability is declared only when it is proven for the reference build. An
undeclared capability is rejected through the capability gate.

```doskerfile
REQUIRES headless
```

A missing capability is an error that names the missing capability and the
engines that provide it:

- One provider:

  ```text
  engine {engine} does not provide the {capability} capability; the {provider} engine provides it
  ```

- Two providers:

  ```text
  engine {engine} does not provide the {capability} capability; the {first} and {second} engines provide it
  ```

- No provider:

  ```text
  engine {engine} does not provide the {capability} capability; no engine provides it
  ```

- Three or more providers:

  ```text
  engine {engine} does not provide the {capability} capability; the {first}, {second}, and {third} engines provide it
  ```

An image with `MACHINE net=ne2000` is rejected on DOSBox-X and Bochs when
`nat` is not provided.

## Capability table

The QEMU, DOSBox-X, and Bochs capability sets are:

| Capability | QEMU | DOSBox-X | Bochs |
| --- | --- | --- | --- |
| `boot` | yes | yes | yes |
| `serial` | yes | yes | yes |
| `stop` | yes | yes | yes |
| `headless` | yes | yes | yes |
| `host_dir_live` | yes | unproven | unproven |
| `debug_native` | yes | yes | yes |
| `gdbstub` | yes | no | no |
| `screenshot` | yes | no | unproven |
| `vnc` | yes | no | unproven |
| `nat` | yes | unproven | unproven |
| `audio` | no | no | no |

`host_dir_live` and `nat` are unproven on DOSBox-X. `MOUNT` before `BOOT`
creates a FAT snapshot: a host edit after the mount is not visible in the
guest, and guest writes do not reach the host. The slirp port forwards bind
all host interfaces rather than loopback.

DOSBox-X has no live host directory, so `--bind` is rejected through the
capability gate. Named volumes are not available on DOSBox-X in this release;
`create` rejects them.

Bochs does not declare `host_dir_live` or `nat` in this release, so `--bind`
and `-p` are rejected. Named volumes are rejected on Bochs.

## Machine

DOSBox-X uses fixed default machine keys:

- `machine = svga_s3`
- `memsize = 16`
- `cputype = 486`
- `sbtype = sb16`
- `[ide, primary] enable = true`

`MACHINE` overrides are not applied on DOSBox-X.

Bochs uses fixed default machine keys:

- `cpu: model=i486dx4`
- `megs: 16`
- an IDE disk on `ata0-master`
- VGA
- two serial ports

`MACHINE` overrides are not applied on Bochs.

## Display

The default display is `none`. Bochs runs with `display_library: nogui`.
Bochs does not support a window display in this release; `--display window`
fails with `unsupported engine feature: window display`.

## Debugger

On Unix, Bochs starts with `-dbg` when the build has the internal
debugger. The adapter sends `c` after start, so the guest boots.
`dosker debug` sends `SIGINT` to break the guest at the debugger prompt and
prints `native debugger <socket>`. Attach a terminal with `nc -U <socket>`
or `socat - UNIX-CONNECT:<socket>`. `r` prints registers, `c` continues, and
`quit` exits. A build without the internal debugger starts without `-dbg`,
and the endpoint is unavailable. It still declares `debug_native` in the
table, because the declared set is the reference build's. Windows has no
native debugger endpoint. The optional gdbstub build is not declared as a
capability.

## Discovery

- `DOSKER_QEMU` overrides the program name. The default is `qemu-system-i386`.
- `DOSKER_QEMU_VERSION` pins the exact QEMU version. It is opt-in on Linux,
  where the distribution QEMU version is reported rather than pinned.
- `DOSKER_QEMU_SHA256` pins the checksum.
- `DOSKER_DOSBOX_X` overrides the program name. The default is `dosbox-x`.
- `DOSKER_DOSBOX_X_VERSION` pins the exact version.
- `DOSKER_DOSBOX_X_SHA256` pins the checksum.
- The reference build is DOSBox-X `2026.10.01` SDL2 (Homebrew on macOS).

Bochs discovery:

- `DOSKER_BOCHS` overrides the program name. The default is `bochs`.
- `DOSKER_BOCHS_VERSION` pins the exact version.
- `DOSKER_BOCHS_SHA256` pins the checksum.
- The reference build is Bochs `3.1`.
- The version comes from the startup banner, not `--version`.

The reference checksum is:

```text
8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c
```

### Release engine pins

| Engine | Version | Checksum variable | Checksum |
| --- | --- | --- | --- |
| QEMU | `11.1.3` | `DOSKER_QEMU_SHA256` | Managed or official builds. The Windows installer uses `DOSKER_WINDOWS_QEMU_SHA256`. |
| DOSBox-X | `2026.10.01` SDL2 | `DOSKER_DOSBOX_X_SHA256` | `6934abe66fe55d71487ded37e097416de65981250c902a9cca2bfd1284e98c96` |
| Bochs | `3.1` | `DOSKER_BOCHS_SHA256` | `8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c` |

### Discovery by host

- Linux x86_64 uses the distribution QEMU and reports its installed version.
  The `DOSKER_QEMU_VERSION` pin is opt-in on Linux.
- macOS arm64 sources come from Homebrew QEMU and Bochs, and from the official
  DOSBox-X arm64 release or Homebrew. The exact release pins apply to these
  managed sources.
- Windows x86_64 uses the official engine builds. The exact release pins
  apply to these builds.
- Homebrew verifies its bottles. The binary checksum pin applies to managed or
  official builds.

## Integration tests

The Bochs suites are Unix-only. The prepared-image import needs the real
agent at `agent/build/DOSKER.EXE`.

The `bochs_runtime` suite needs `DOSKER_INTEGRATION=1`, `DOSKER_BOCHS`,
`DOSKER_BOCHS_VERSION=3.1`, `DOSKER_BOCHS_SHA256`, and
`DOSKER_BOCHS_IMAGE=agent/fixtures/dosker-agent.img`. `DOSKER_BOCHS_SHA256`
is the reference build's checksum; do not reuse it for the gdbstub build.
Run it with:

```sh
DOSKER_INTEGRATION=1 DOSKER_BOCHS=bochs DOSKER_BOCHS_VERSION=3.1 \
DOSKER_BOCHS_SHA256=<sha256> \
DOSKER_BOCHS_IMAGE=agent/fixtures/dosker-agent.img \
cargo test -p dosker-cli --features qemu-engine --test bochs_runtime
```

The optional `gdbstub_build` suite needs the three gates
`DOSKER_INTEGRATION=1`, `DOSKER_BOCHS_GDBSTUB`, and `DOSKER_BOCHS_IMAGE`.
Run it with:

```sh
DOSKER_INTEGRATION=1 DOSKER_BOCHS_GDBSTUB=<path> \
DOSKER_BOCHS_IMAGE=agent/fixtures/dosker-agent.img \
cargo test -p dosker-engine-bochs --test gdbstub_build
```

## Reporting

`dosker engines` prints one line per implemented engine:

- a found engine: `{name} {version} {path} {capabilities}`;
- a missing engine: `{name} not found: {error}`;
- an unusable engine: `{name} unavailable: {error}`.

It exits 1 only when no engine is found.

`dosker doctor` prints one line per implemented engine:

- a found engine: `{name}: {version} ({path}) [{capabilities}]`;
- a missing engine: `{name}: not found ({error})`;
- an unusable engine: `{name}: unavailable ({error})`.

It exits 0 when the data root is writable and at least one engine is found.
