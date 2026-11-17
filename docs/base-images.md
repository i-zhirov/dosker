# FreeDOS base images

DOSker builds its default `freedos:1.3` base locally from a pinned FreeDOS
distribution. It does not redistribute FreeDOS.

## Inputs

The base build reads these inputs:

- `DOSKER_FREEDOS_SOURCE` is a local FreeDOS 1.3 boot floppy image. For the
  official archive it is `144m/x86BOOT.img` from the FreeDOS 1.3 Floppy
  Edition ZIP:

  ```text
  https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/distributions/1.3/official/FD13-FloppyEdition.zip
  ```

  The ZIP is pinned by SHA-256:

  ```text
  75a4e11a7fce6f124e20927b3022b4b715a2a3f7324c5f5bfea42d90d80eb072
  ```

- `DOSKER_AGENT_EXE` is the DOS agent executable. It defaults to
  `agent/build/DOSKER.EXE`.
- `agent/fixtures/base/mbr.bin` and `agent/fixtures/base/boot-sector.bin` are
  the committed MBR and boot-sector templates.

## Pins

`load_freedos_binaries` compares `KERNEL.SYS`, `COMMAND.COM`, and
`DOSKER.EXE` with the exact `sha256:<hex>` strings returned by
`sha256_digest`. The `freedos_inputs` test checks the two template pins against
their constants. CI compares the ZIP archive's bare SHA-256 hexadecimal value
with `PINNED_ZIP_SHA256`.

| File | SHA-256 | Bytes |
| --- | --- | --- |
| `KERNEL.SYS` | `sha256:f34a7483c575fcf2709d9a7d0bc3db81c6211c279530f9e1bf78576b9233924d` | 46 485 |
| `COMMAND.COM` | `sha256:93ed257d3aa2454821aa305b731544550dcccff898865b590deb972c47b48610` | 85 480 |
| `DOSKER.EXE` | `sha256:590fcc601e8ad9e85ebd8b3b4575c67460b51f46e0bcf7d89f6bff57f9f05591` | 43 314 |
| `mbr.bin` | `sha256:d2cc0119c6d3caec354658eb822d3b0dfb69884a521d7c8aa28d59cacdc638ff` | 512 |
| `boot-sector.bin` | `sha256:b27479134bbd40f1aa45a12e8540a1d1e9cd1fd73cea8bec9d942372132256b7` | 512 |

The FreeDOS version is `1.3` and the agent version is `1.0.0`.

## Updating a pin

The file-pin constants in `crates/dosker-image/src/freedos/inputs.rs` carry
the `sha256:` prefix. The ZIP pin constant `PINNED_ZIP_SHA256` in
`.github/scripts/fetch-freedos-source.sh` is bare hexadecimal, matching the
`sha256sum` or `shasum -a 256` output.

To update a pin, replace the source binary or template, recompute its digest
with `shasum -a 256` or `sha256sum`, and update the matching constant in
`crates/dosker-image/src/freedos/inputs.rs`. Then run:

```sh
cargo test -p dosker-image --test freedos_inputs
```

Finally rerun the two-host determinism job so both hosts agree on the new
bytes.

Re-extract `agent/fixtures/base/mbr.bin` and
`agent/fixtures/base/boot-sector.bin` only when
`agent/fixtures/freedos-base.img` changes:

```sh
dd if=agent/fixtures/freedos-base.img of=agent/fixtures/base/mbr.bin bs=512 count=1
dd if=agent/fixtures/freedos-base.img of=agent/fixtures/base/boot-sector.bin bs=512 skip=2048 count=1
```

## Agent artifact check

`load_freedos_binaries` compares the agent file with `FREEDOS_AGENT_SHA256`.
The CI jobs download one `dosker-agent-binaries` artifact, so both hosts use
the same bytes. A mismatch is a build error that names the expected and actual
digests.

## Source input

The CLI requires `DOSKER_FREEDOS_SOURCE` to be a local boot floppy image only
when the base must be built. A valid stored `freedos:1.3` base is reused
without the source or agent inputs. A stale or invalid base is rebuilt, which
needs both.

A URL or ZIP is a CI repository-variable value only. Before the build,
`.github/scripts/fetch-freedos-source.sh` resolves it to a local path: it
verifies and extracts the pinned ZIP, downloads a raw image, or accepts a local
path. The CLI itself never downloads the source.

An old source-only `dosker-build:` tag from before agent injection is not
reused. It can be removed with `dosker rmi`.

## Layout

- Size: 32 MiB (`33 554 432` bytes).
- MBR partition: bootable, type 6, start LBA 2048, 63 488 sectors.
- File system: FAT16, 512-byte sectors and clusters, 2 FATs, 512 root entries,
  media descriptor `0xF8`.
- Hidden sectors: 2048.
- Volume label: `DOSKER`.
- Volume id: `0x0045105C`.
- Geometry: 124 cylinders, 16 heads, 32 sectors per track, 512 bytes per
  sector.

## Files

The base writes this fixed file list:

- `KERNEL.SYS` and `COMMAND.COM` from the source image.
- `DOSKER\DOSKER.EXE` with the agent bytes.
- `CONFIG.SYS`:

  ```text
  FILES=20\r\nBUFFERS=10\r\nLASTDRIVE=Z\r\n
  ```

- `AUTOEXEC.BAT`:

  ```text
  @ECHO OFF\r\nSET COMSPEC=C:\COMMAND.COM\r\nC:\DOSKER\DOSKER.EXE\r\n
  ```

`\r\n` is a CRLF pair. Every generated entry uses the timestamp
`1980-01-01T00:00:00Z` and has no read-only, hidden, system, or archive bit.

## Reusing `freedos:1.3`

A Doskerfile with `FROM freedos:1.3` reuses a valid stored base by tag. Reuse
needs no source or agent inputs; a stale or invalid base is rebuilt, which
needs both. The lock pins the base by name, tag, and manifest digest, and pins
the agent by version:

```toml
version = 1

[[base]]
name = "freedos"
tag = "1.3"
digest = "sha256:<manifest digest>"

[agent]
version = "1.0.0"
```

`dosker build --update` refreshes a moved pin. It does not rebuild a stored
base. To rebuild the base, remove its tag with `dosker rmi freedos:1.3` and
run the build again with the source and agent available.

## Determinism

A repeat build in two different data roots produces an identical manifest
digest. CI builds the base twice under separate roots in
`freedos-base-determinism` and compares the digest from Ubuntu and macOS in
`compare-freedos-base`.
