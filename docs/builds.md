# Builds

A build turns a Doskerfile into an image. `RUN --host` executes on the host.
`RUN` executes inside a transient DOS virtual machine. This page describes
in-DOS `RUN`.

## RUN semantics

- Each `RUN` boots its own virtual machine. Consecutive `RUN` instructions
  are not batched.
- The command runs through `COMMAND.COM`.
- The agent captures the command output, so output redirection is not
  required.

## Timeouts

- `--timeout` defaults to 300 seconds and applies to the agent command.
- The boot wait is 60 seconds.
- The shutdown grace is 10 seconds.

## Disk projection

- The tree is written over a private writable copy of the image's raw disk.
  The stored image is never modified.
- The effective MBR is written first, then the FAT region is located from it.
- Only FAT12 and FAT16 volumes are supported.
- For an unchanged boot, the projection preserves the geometry, label, media
  descriptor, and disk size.

## Runtime projection

`create` and `run` project the final tree onto the private C: disk for a
non-booter image whose history contains a non-empty `COPY` or `RUN`.
This uses the same `project_tree` path as in-DOS `RUN`, so `COPY` and
`RUN` files are visible in the running container.

Config-only, imported, and FreeDOS-base images keep the raw-disk copy
unchanged. Booter images are never projected. The projected disk is
runtime state only; the image's raw disk and its annotation do not change.
Boot overrides are not applied by `create`.

## Partition overrides

A layer can carry a boot override. A changed MBR, partition table, or boot
sector is recorded in the layer. The next `RUN` reapplies the effective MBR,
locates the partition from it, and writes the boot sector at the new region.
See [Images](images.md) for how the effective boot metadata is resolved.

## The DOSKER directory

- `CONFIG.INI` is written before the guest boots.
- `C:\DOSKER` and the agent's temporary files are excluded from the layer.
- A layer never modifies the agent files.

## Serial layout

The build VM maps the effective `serial_ports` count and places the agent on
`control_port`. `CONFIG.INI` carries the effective control port and baud.

## Output

The captured agent output is reported after `EXEC` returns, including when
the command exits non-zero. Empty output reports exactly:

```text
RUN produced no captured output
```

Truncated output reports exactly:

```text
RUN output was truncated
```

## Failures

A non-zero exit, command timeout, boot failure, missing agent, or guest
reboot fails the build and commits no layer. A guest reboot reports:

```text
the guest rebooted during the command
```

## Cache

The cache key includes:

- the parent tree digest;
- the build config digest;
- the normalized instruction;
- the engine version used at that step;
- the agent version;
- the raw disk digest.

A cache hit boots no virtual machine.

## Engine selection

Each `ENGINE` instruction applies to later steps. The lock records every
engine used, and a `RUN` uses the engine selected for its own step.

## Engine support

`qemu` is the only engine for in-DOS `RUN` and the build path. `dosbox-x`
and bochs are available for `run` and `create` with a prepared image. A
build whose final engine is `dosbox-x` or `bochs` fails with `engine
dosbox-x: unsupported engine` or `engine bochs: unsupported engine` after
the `REQUIRES` check passes, before the step loop. The final engine must
satisfy every `REQUIRES` in the Doskerfile; the check runs before the step
loop. The RUN virtual machine has no NIC, so a command must not depend on
network access.

## Prerequisites

A build needs QEMU, `agent/build/DOSKER.EXE`, and a FreeDOS base with the
agent. A tree-only image without a raw disk fails before the cache is
consulted.

## Example

```dockerfile
FROM freedos:1.3
COPY HELLO.TXT C:\
RUN COPY HELLO.TXT MADE.TXT
CMD ECHO BUILT
```

`RUN` boots once and creates `MADE.TXT`. `CMD` runs when a container starts
from the resulting image, not during the build.
