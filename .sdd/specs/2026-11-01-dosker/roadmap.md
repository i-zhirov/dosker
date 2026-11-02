# DOSker Roadmap

> Each slice gets its own plan, written when the slice starts. Execute a
> slice with the `executing-plans` skill.

- **Date**: 2026-11-01
- **Status**: Approved
- **Design**: `.sdd/specs/2026-11-01-dosker/design.md`
- **Strategy**: Thirteen slices in dependency order. Slices 1 to 4 are the
  first-release runtime gate. Slice 1 is the smallest end-to-end tracer: a
  mock engine proves the supervisor, the control transport, the state store,
  and the CLI before any QEMU or DOS code lands. Slice 2 adds the guest side:
  the 16-bit agent and the frame codec. Slice 3 joins both with the real QEMU
  engine and reaches the design's M1 definition of done. Slice 4 completes the
  Windows runtime track, which runs in parallel and gates the first release.
  Slices 5 to 8 are the build path. Slice 5 gives the canonical file tree,
  layers, and the OCI store. Slice 6 adds the Doskerfile parser, `COPY`,
  `RUN --host`, and the cache. Slice 7 builds the deterministic FreeDOS base
  and injects the agent. Slice 8 adds `RUN` inside DOS, which needs the agent,
  the image builder, and the base image together. Slices 9 to 13 add data,
  network, more engines, and packaging. Slice 9 adds volumes and QEMU NAT.
  Slice 10 adds development mode. Slices 11 and 12 add the capability-gated
  adapters. Slice 13 packages and releases. QEMU is the only required engine
  for the first release. DOSBox-X and Bochs are later, capability-gated
  slices and must not block the QEMU path. A slice starts only when its spike
  and toolchain prerequisites are done. The design is too large for one plan
  and one session, so each slice gets a plan just in time.

## Slices

### Slice 1 — runtime-foundation

Delivers the host runtime that every later slice builds on. It covers the
`dosker-platform` crate (control transport, process cleanup seam, data
directory, console), the state store with atomic writes, the supervisor
process, the JSON-lines control protocol, a mock engine, the CLI skeleton,
`DOSKER_ROOT`, and stale-state recovery. The mock engine lets the full path
run without QEMU, DOS, or a 16-bit toolchain, so the slice stands alone.

In scope: the container record and its lifecycle states, the control endpoint
with the Unix socket backend and the Windows named-pipe backend behind one
trait, the Unix process-group cleanup, and the stale-supervisor rule. Out of
scope: any real engine, the DOS agent, image storage, the build path, and the
Windows Job Object.

- **Status:** Implemented
- **Depends on:** —
- **Requirements:** FR-008, FR-019, FR-022, FR-023
- **Manual prerequisites:** Rust toolchain. A Windows CI runner for the
  named-pipe contract tests. No QEMU and no DOS toolchain.
- **Plan:** [`plan-runtime-foundation.md`](plan-runtime-foundation.md)
- **Done when:** a mock container starts detached on Linux and macOS, answers
  `exec`, `status`, and `stop` over the Unix socket, and writes valid state.
  Killing the supervisor makes the next `dosker ps` report `exited` and leaves
  no mock process. The named-pipe backend passes the same transport contract
  suite on the Windows runner against the mock supervisor. `DOSKER_ROOT`
  redirects all state. A long `DOSKER_ROOT` starts and stops through the short
  socket fallback.

### Slice 2 — dos-agent-serial

Delivers the guest side of the control channel. It covers the 16-bit DOS
agent, the host frame codec in `dosker-agent-proto`, the QEMU serial mapping
for COM2, and the `HELLO`, `EXEC`, `PUT`, `GET`, `STAT`, and `SHUTDOWN`
commands. It also proves the agent side of the line shell with a sequence of
`EXEC` requests. A `/TEST` self-check mode proves the agent without the
supervisor, so the slice stands alone.

In scope: the frame format, CRC and retry, stop-and-wait, the 4096-byte
payload limit, the 1 MiB output limit, the 127-character command limit, and a
conformance test that runs the Rust codec against the real agent in QEMU. Out
of scope: the supervisor, the CLI, the image builder, and every engine other
than QEMU.

- **Status:** Implemented
- **Depends on:** Slice 1. The QEMU/FreeDOS spike and the 16-bit toolchain
  are manual prerequisites.
- **Requirements:** FR-009, FR-010
- **Manual prerequisites:** before this slice, a FreeDOS 1.3 Floppy Edition
  boot floppy (`144m/x86BOOT.img` from `FD13-FloppyEdition.zip`) supplied
  through `DOSKER_FREEDOS_SOURCE`, with QEMU, `mtools`, `fdisk`, `sfdisk`,
  and `timeout` on the fixture host. OpenWatcom or ia16-gcc, plus the
  OpenWatcom runtime license check.
- **Plan:** `.sdd/specs/2026-11-01-dosker/plan-dos-agent-serial.md`
- **Done when:** a generated FreeDOS base image boots under QEMU headless
  and the agent sends `HELLO`. The host completes the handshake.
  `EXEC ECHO HELLO` returns `HELLO` and status 0. `PUT` and `GET` round-trip
  a file. `STAT` returns size, the directory flag, and attributes.
  `SHUTDOWN` gets `SHUTDOWN_ACK`. A CRC error retries three times and then
  closes the session. The `/TEST` self-check passes.

### Slice 3 — qemu-runtime-slice

Delivers the first usable runtime. It covers the QEMU adapter, image import
for a DOS image and a booter blob, `create`, `start`, `stop`, `restart`, `rm`,
`run`, `exec`, `cp`, `attach`, `ps`, `inspect`, `logs`, readiness, shutdown,
`screenshot`, `engines`, `doctor`, and QEMU gdbstub pass-through. It adds the
minimal image record that
holds an imported disk, the private writable disk with reflink or full copy,
and the `C:\DOSKER\CONFIG.INI` write before boot. The design's M1 definition of
done verifies the slice on its own.

The image comes from the M0 spike and already contains the agent. The import
layer that injects an agent into an arbitrary DOS image arrives in Slice 7,
with the FreeDOS base builder. The OCI layout and the general tree and layer
model arrive in Slice 5. This slice keeps a minimal import record so the
runtime can be proven first. Out of scope: the build path, layers, volumes,
NAT, and the Windows host.

- **Status:** Unplanned
- **Depends on:** Slices 1 and 2.
- **Requirements:** FR-005, FR-006, FR-008, FR-009, FR-010, FR-011, FR-018,
  FR-019, FR-021
- **Manual prerequisites:** QEMU builds for Linux and macOS. A prepared
  FreeDOS image from the M0 spike, for `image import`.
- **Plan:** —
- **Done when:** the design's M1 definition of done passes on Linux x86_64 and
  macOS arm64. A prepared FreeDOS image imports and boots headless. `exec`
  returns output and status. `attach` gives a working shell. `stop` flushes and
  `start` returns to the same disk. `restart` returns a stopped container to
  running. `cp` moves a small file in and out of a running container. `exec` on
  a booter image returns `no_agent`. `screenshot` writes a file. `engines` and
  `doctor` report the QEMU version and capabilities. The gdbstub is reachable
  on a socket.

### Slice 4 — windows-runtime-track

Completes the Windows x86_64 host. It covers the named-pipe control transport
end to end, the Job Object with kill-on-close, the Windows data root, QEMU
discovery and version checks, localhost TCP engine endpoints, and the Windows
smoke tests. The design's M1-W definition of done verifies the slice on its
own.

In scope: a pipe DACL for the current user only, `ReplaceFileW` state writes,
Job Object cleanup after a supervisor crash, `%LOCALAPPDATA%\dosker`, and QMP,
serial, monitor, gdbstub, VNC, and NAT over `127.0.0.1`. Out of scope: the
build path and the DOSBox-X and Bochs engines.

- **Status:** Unplanned
- **Depends on:** Slices 1 and 3.
- **Requirements:** FR-021, FR-022, FR-023
- **Manual prerequisites:** a Windows CI runner with QEMU. A Windows QEMU
  build with a pinned version and checksum.
- **Plan:** —
- **Done when:** the design's M1-W definition of done passes on Windows
  x86_64. The CLI reaches the supervisor through a named pipe. The engine uses
  localhost TCP. A supervisor crash leaves no QEMU process. `engines` finds the
  Windows QEMU build and reports its capabilities.

### Slice 5 — image-store-and-tree

Delivers the canonical image model. It covers the local OCI image layout and
the DOSker media types, the DOS file tree, normal layers as a canonical tar
with a JSON sidecar, whiteouts, the layer parent rule, boot metadata, and the
build cache record. It also moves image import onto the OCI store and adds the
`images`, `rmi`, and `tag` commands. Digest and layout tests verify the slice
on its own.

In scope: tree application in layer order, uppercase 8.3 path normalization,
digest stability, the config digest as canonical JSON, the read-only image
rule, a private writable disk per container, a whiteout of a missing path, and
the file-versus-directory conflict error. Out of scope: the Doskerfile parser,
the build instructions, the FAT projection, and the cache invalidation wiring.

- **Status:** Unplanned
- **Depends on:** Slice 1.
- **Requirements:** FR-002, FR-003, FR-012, FR-017, FR-018
- **Manual prerequisites:** none.
- **Plan:** —
- **Done when:** layers apply in order and produce a tree. Two independent
  writes of the same tree produce the same layer digest. A whiteout removes a
  path and a missing whiteout is ignored. The OCI index, manifest, config, and
  blobs validate against the OCI layout. An imported disk and an imported
  booter blob round-trip through the store. A write to a stored image is
  rejected.

### Slice 6 — doskerfile-and-host-build

Delivers the host-side build. It covers the Doskerfile parser with every
instruction, placeholder expansion, the normalized instruction form, the
config-only instructions, `COPY` with 8.3 validation and symlink and traversal
rejection, `RUN --host` on the staged tree, the lock file, and the build cache
wiring. A full host-only build with a cache hit verifies the slice on its own.

In scope: `FROM`, `COPY`, `RUN --host`, `ENV`, `WORKDIR`, `CMD`, `MACHINE`,
`ENGINE`, `ENGINE-CONFIG`, `EXPOSE`, `VOLUME`, `REQUIRES`, `.doskerignore`, the
fixed `1980-01-01T00:00:00Z` timestamp, the `C:\DOSKER` reservation, and the
cache key from parent tree digest, config digest, normalized instruction, and
input hashes. Out of scope: `RUN` inside DOS, the FreeDOS base builder, and
agent injection into an imported image.

- **Status:** Unplanned
- **Depends on:** Slice 5.
- **Requirements:** FR-001, FR-002, FR-003, FR-013, FR-017
- **Manual prerequisites:** none.
- **Plan:** —
- **Done when:** a Doskerfile with `FROM` an imported image, `COPY`,
  `RUN --host`, `ENV`, `WORKDIR`, `CMD`, and `MACHINE` builds an image that
  boots with Slice 3. A second build reports a cache hit for every step and
  boots no VM. An `ENV` change invalidates the later steps. A bad 8.3 name, a
  symlink, or a `..` path fails before execution and names the path. A build
  writes a version 1 lock file. A changed lock pin stops the build and
  `--update` refreshes it.

### Slice 7 — freedos-base-builder

Delivers the default base and the agent injection path. It covers the
deterministic FreeDOS base builder from pinned binaries, the agent binary in
`C:\DOSKER`, the wrapped `AUTOEXEC.BAT`, the `CONFIG.SYS` settings, the import
layer for an arbitrary DOS image, and the base and agent pins. A digest match
on two hosts and a boot to the agent handshake verify the slice on its own.

In scope: MBR, partition table, geometry, boot sector, and volume label from
the pinned inputs, the `LASTDRIVE=Z`, `FILES=20`, and `BUFFERS=10` settings,
and `FROM freedos:<version>`. Out of scope: `RUN` inside DOS, which needs the
read-back path from Slice 8.

- **Status:** Unplanned
- **Depends on:** Slices 5 and 6. The FreeDOS builder spike is a manual
  prerequisite.
- **Requirements:** FR-004, FR-005
- **Manual prerequisites:** the FreeDOS binary distribution and the boot
  sector inputs. Before this slice, the deterministic FreeDOS builder spike.
- **Plan:** —
- **Done when:** a clean build of the base image from the pinned inputs
  produces the same digest on two hosts. The image boots on QEMU and reaches
  the agent handshake. `FROM freedos:<version>` resolves the built base. An
  imported DOS image receives a layer with `C:\DOSKER\DOSKER.EXE` and a wrapped
  `AUTOEXEC.BAT`, and the result boots and answers `exec`. The lock file
  records the base digest and the agent version.

### Slice 8 — in-dos-run

Delivers `RUN` inside DOS. It covers the bootable FAT projection of the tree,
the transient QEMU VM per instruction, the agent `EXEC`, the tree read-back,
the boot metadata capture, the restore of `C:\DOSKER`, the `guest_rebooted`
failure, and the cache entry for a `RUN` step. The design's SC-002 verifies
the slice on its own.

In scope: one boot per instruction, the 300 second default timeout, the
read-back of MBR, partition table, and boot sector, the exclusion of
`C:\DOSKER` from the delta, and the private writable disk for the build VM. Out
of scope: batching consecutive `RUN` steps.

- **Status:** Unplanned
- **Depends on:** Slices 2, 5, 6, and 7.
- **Requirements:** FR-002, FR-003, FR-007
- **Manual prerequisites:** QEMU. The FreeDOS base and the agent from Slices
  2 and 7.
- **Plan:** —
- **Done when:** a `RUN` that writes a file changes the tree and produces a
  layer. A `RUN` that changes the boot sector updates the layer boot metadata.
  A non-zero exit fails the build and commits no layer. A command that reboots
  the guest fails with `guest_rebooted`. A second build of the same Doskerfile
  reports a cache hit and boots no VM. The design's SC-002 passes.

### Slice 9 — volumes-and-nat

Delivers named volumes and network publishing. It covers partitioned FAT16
volume images, the exclusive attach lock, `-v name:D[:ro]`, the anonymous
volume from `VOLUME`, `dosker volume create`, `ls`, and `rm`, `-p host:guest`
with the default `127.0.0.1` bind, and QEMU user-mode NAT. The volume attach
test and the published-port test verify the slice on its own.

In scope: the volume `meta.json` and lock, the `superfloppy` layout for floppy
sizes, the port conflict error, `dosker port`, and the NE2000 guest NIC. Out of
scope: NAT on any engine other than QEMU, and the shared L2 network.

- **Status:** Unplanned
- **Depends on:** Slices 3 and 5.
- **Requirements:** FR-014, FR-015
- **Manual prerequisites:** QEMU. A DOS packet driver and a TCP/IP stack for
  the integration test.
- **Plan:** —
- **Done when:** a partitioned FAT16 volume attaches to one running container
  and survives `rm`. A second attach fails and names the holder. A read-only
  volume rejects guest writes. A guest TCP service reaches the host at
  `127.0.0.1:<published>` through the packet driver and the TCP/IP stack. Two
  containers with the same host port fail the second start and leave the first
  running. `dosker port` reports the effective mapping.

### Slice 10 — development-mode

Delivers `--bind D:/host/path[:ro]`. It covers the QEMU `vvfat` live host
directory, the sync fallback when `vvfat` is unavailable, the 8.3 validation
of the host directory, and the read-only mount. The host-edit visibility test
and the invalid-name refusal verify the slice on its own.

In scope: live visibility of a host edit in the guest, refusal of an invalid
8.3 name with the path named, a read-only guard, and the documented beta
warning that keeps this mode out of CI. Out of scope: a shared L2 network and
any engine other than QEMU.

- **Status:** Unplanned
- **Depends on:** Slices 3 and 9.
- **Requirements:** FR-015 (the capability gate), FR-017
- **Manual prerequisites:** QEMU. A host filesystem for the `vvfat` test.
- **Plan:** —
- **Done when:** a host edit is visible in the guest without a rebuild. A file
  name that is not valid 8.3 is refused before boot and the invalid path is
  named. A read-only bind rejects a guest write. The sync fallback shows the
  same content when `vvfat` is disabled. The warning appears in `doctor` and
  the docs.

### Slice 11 — dosbox-x-adapter

Delivers the DOSBox-X adapter behind the capability model. It covers `BOOT`,
serial, headless, the host directory, and the debugger, as the spike proves
them. It populates the DOSBox-X column of the capability table and enforces
`REQUIRES`. The design's SC-003 gives the slice a definite outcome.

In scope: the `dosker-engine-dosbox-x` crate, the `ENGINE dosbox-x` and
`ENGINE-CONFIG` path, the capability declaration from the spike evidence, the
reject message that names the engines that provide a missing capability, and
the NAT decision. Out of scope: any capability that the spike does not prove.
An unproven cell stays unproven and is rejected.

- **Status:** Unplanned
- **Depends on:** Slices 3, 9, and 10. The DOSBox-X spike is a manual
  prerequisite.
- **Requirements:** FR-011, FR-016
- **Manual prerequisites:** the DOSBox-X `BOOT`, headless, and debugger spike.
  Official DOSBox-X builds. Before declaring NAT on DOSBox-X, the NAT spike.
- **Plan:** —
- **Done when:** the same image boots on DOSBox-X and answers `exec` when the
  spike proves serial and headless. Every DOSBox-X cell in the capability table
  is backed by an integration test or marked unproven. A `REQUIRES` of a
  missing capability fails before boot and names QEMU or Bochs. `dosker
  engines` reports the DOSBox-X version and capabilities. SC-003 has a definite
  outcome.

### Slice 12 — bochs-adapter

Delivers the Bochs adapter. It covers `bochsrc` generation, the serial socket,
`nogui`, the internal debugger, and the gdbstub when the build supports it. It
populates the Bochs column of the capability table. The headless boot test and
the debugger test verify the slice on its own.

In scope: the `dosker-engine-bochs` crate, the geometry rule that the image
size equals `C*H*S*512`, the `-dbg` internal debugger pass-through, and the
capability declaration from the spike evidence. Out of scope: a gdbstub in a
build that does not have it.

- **Status:** Unplanned
- **Depends on:** Slices 3, 9, and 10. The Bochs spike is a manual
  prerequisite.
- **Requirements:** FR-011, FR-016
- **Manual prerequisites:** the Bochs headless, serial, and debugger spike.
  Bochs builds. Before declaring NAT on Bochs, the NAT spike.
- **Plan:** —
- **Done when:** the same image boots headless on Bochs. Serial reaches the
  agent. The internal debugger accepts commands. `dosker debug` on Bochs
  returns a working endpoint. Every Bochs cell in the capability table is
  backed by a test or marked unproven. `dosker engines` reports the Bochs
  version and capabilities.

### Slice 13 — packaging-and-release

Delivers the release surface. It covers `save`, `load`, `system prune`, engine
discovery on macOS and Windows, the security statement in the README and
`doctor`, install instructions, and release artifacts for Linux x86_64, macOS
arm64, and Windows x86_64. It also records the post-M4 decision on the
multi-engine model. The save and load round-trip on three hosts verifies the
slice on its own.

In scope: a tar round-trip of images, pruning of stopped containers, unused
images, and stale cache entries, checksum-verified engine discovery, and the
MIT or Apache-2.0 dual license files. Out of scope: a registry, compose files,
and a shared L2 network.

- **Status:** Unplanned
- **Depends on:** Slices 1 to 12.
- **Requirements:** FR-020, FR-021, and the release-facing parts of the
  design.
- **Manual prerequisites:** release runners for Linux, macOS, and Windows,
  and signing credentials. A macOS CI runner with the engine builds.
- **Plan:** —
- **Done when:** `save` and `load` round-trip an image on all three hosts.
  `system prune` removes only stopped containers, unused images, and stale
  cache entries. Engine discovery works on Linux, macOS, and Windows with
  pinned versions. The README and `doctor` state that DOSker does not sandbox
  untrusted images. Installable artifacts exist for all three hosts. SC-001 to
  SC-007 pass. The multi-engine decision is recorded with the M4 evidence.

## Definition Of Done

All thirteen slices are `Implemented`. Each slice plan is `Approved` and
executed. The design's success criteria SC-001 to SC-007 pass. The unit, mock,
and adapter contract suites pass on Linux, macOS, and Windows. The QEMU
integration suite passes on Linux and in the nightly macOS job. No open
Critical or Important finding remains. No released feature depends on an
unproven capability. The README and `doctor` state the security limits.
Release artifacts exist for Linux x86_64, macOS arm64, and Windows x86_64.
