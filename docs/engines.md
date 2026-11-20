# Engines

An engine runs a DOS image. DOSker implements an adapter for each engine it
can start. This page describes engine selection, the capability model, and
the DOSBox-X runtime engine.

## Engine selection

- `qemu` and `dosbox-x` have adapters. `bochs` is a known engine name but has
  no adapter in this release.
- `--engine` on `create` and `run` overrides the image `ENGINE` pin for one
  command.
- Without `--engine` and without an `ENGINE` pin, the engine is `qemu`.
- In-DOS `RUN` and the build path stay QEMU-only. A build whose final engine
  is `dosbox-x` fails with `engine dosbox-x: unsupported engine` before the
  step loop.

## ENGINE-CONFIG

An `ENGINE-CONFIG` line carries engine-specific configuration text. The raw
text is passed to the named adapter.

- QEMU splits the raw line into command-line arguments with shell-like
  quoting. Whitespace separates arguments; single and double quotes group; a
  backslash escapes the next character outside single quotes; an unterminated
  quote is an error.
- DOSBox-X appends raw lines verbatim before `[autoexec]`. A line may be a
  `[section]` header or a `key=value` pair.

A DOSBox-X example:

```doskerfile
ENGINE-CONFIG dosbox-x [cpu]
ENGINE-CONFIG dosbox-x cputype = 486
```

## REQUIRES

`REQUIRES` names capabilities a Doskerfile needs. `create`, `run`, and
`dosker build` enforce it after engine selection and before any VM starts.
The final selected engine must satisfy every `REQUIRES` in the Doskerfile. A
requirement is not checked against the engine in effect at its own step.

A capability is declared only when it is proven for the installed build. An
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

An image with `MACHINE net=ne2000` is rejected on DOSBox-X when `nat` is not
provided.

## Capability table

The QEMU and DOSBox-X capability sets are:

| Capability | QEMU | DOSBox-X |
| --- | --- | --- |
| `boot` | yes | yes |
| `serial` | yes | yes |
| `stop` | yes | yes |
| `headless` | yes | yes |
| `host_dir_live` | yes | unproven |
| `debug_native` | yes | yes |
| `gdbstub` | yes | no |
| `screenshot` | yes | no |
| `vnc` | yes | no |
| `nat` | yes | unproven |
| `audio` | no | no |

`host_dir_live` and `nat` are unproven on DOSBox-X. `MOUNT` before `BOOT`
creates a FAT snapshot: a host edit after the mount is not visible in the
guest, and guest writes do not reach the host. The slirp port forwards bind
all host interfaces rather than loopback.

DOSBox-X has no live host directory, so `--bind` is rejected through the
capability gate. Named volumes are not available on DOSBox-X in this release;
`create` rejects them.

## Machine

DOSBox-X uses fixed default machine keys:

- `machine = svga_s3`
- `memsize = 16`
- `cputype = 486`
- `sbtype = sb16`
- `[ide, primary] enable = true`

`MACHINE` overrides are not applied on DOSBox-X.

## Discovery

- `DOSKER_DOSBOX_X` overrides the program name. The default is `dosbox-x`.
- `DOSKER_DOSBOX_X_VERSION` pins the exact version.
- `DOSKER_DOSBOX_X_SHA256` pins the checksum.
- The reference build is DOSBox-X `2026.10.01` SDL2 (Homebrew on macOS).

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
