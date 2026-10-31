# DOSker — QEMU-Only Deep Dive

This asset explores a hypothetical QEMU-only DOSker. The current design keeps the multi-engine model. This document records what would change, which previously discussed questions it affects, and the positive outcomes of each change. The negative outcomes are in [`engine-fallback-analysis.md`](engine-fallback-analysis.md).

## Premise

Assume the project drops DOSBox-X and Bochs and keeps QEMU as the only engine. The goal is to understand the full simplification, not to recommend the drop now. QEMU is already the first engine and the only engine required for M1 and M2. The QEMU-only shape is therefore close to the current critical path.

## Summary

A QEMU-only system would remove the adapter layer, the capability matrix, two engine crates, two engine artifact sets, two M0 spikes, M4, and a large part of the host-specific engine discovery. It would make the Doskerfile smaller and the Windows support simpler. It would also unlock QEMU-specific features that the multi-engine design cannot assume, especially qcow2 overlays, snapshots, suspend and resume, QMP events, and the QEMU sandbox on Linux.

The main cost is product capability. DOSBox-X is the stronger engine for games, sound, and timing-sensitive software. Bochs is the stronger engine for accurate real-mode debugging. A QEMU-only product is simpler, but it is a narrower product and it no longer matches the original `task.md` requirement.

## Previously Discussed Questions And The QEMU-Only Change

| Previously discussed point | QEMU-only change | Positive outcome | Cost or limit |
| --- | --- | --- | --- |
| Primary use case: build and test first, other uses later | QEMU still serves build and test. Games and BBS lose their strongest engine. | Focused product for the first use case | Weaker for games, BBS, and timing-sensitive software |
| Several execution engines | Keep QEMU only | One engine to build, pin, test, and ship | The original requirement is not met |
| DOS sources: FreeDOS, BYO DOS, booter images | QEMU boots all three from a disk image | One boot path for every source | None |
| Build layers: file-tree deltas | Keep the layer model. Add qcow2 snapshots as an optional accelerator. | Inspectable layers plus faster builds | Snapshot extraction needs a spike |
| Capability-based adapters | Remove the adapter trait or keep a one-implementation seam | Less code, fewer tests, one configuration format | The abstraction rots if it stays unused |
| `REQUIRES` instruction | Remove | Smaller Doskerfile and fewer build errors | No way to express engine requirements |
| `ENGINE` instruction | Remove | No engine selection errors | Less future flexibility |
| `ENGINE-CONFIG` escape hatch | Rename to `QEMU-CONFIG` or keep as a QEMU-only escape hatch | Direct QEMU power and less mapping code | Doskerfiles become QEMU-specific |
| Engine pin in the lock file | Keep only the QEMU version | Simpler lock file | None |
| Image config `engine_pin` and `requires` | Remove | Smaller image config | None |
| `dosker engines` command | Replace with `dosker qemu` or fold into `doctor` | Less CLI surface | None |
| Engine crates | Remove `dosker-engine-dosbox-x` and `dosker-engine-bochs` | Smaller workspace and faster builds | Re-adding an engine costs more |
| Adapter contract tests | Keep only the QEMU contract | Faster CI | No cross-engine coverage |
| M0 spikes 3 and 4 | Remove the DOSBox-X and Bochs spikes | Faster M0 | No DOSBox-X or Bochs evidence |
| M4 milestone | Remove | Faster roadmap | No extra engines |
| Windows M1-W track | Verify QEMU only and drop the M4 Windows checks for the other engines | Simpler Windows support | None |
| Engine capability matrix | Replace with a QEMU feature table | Less documentation and fewer unknowns | None |
| DOSBox-X `BOOT` divergence | Gone | One boot model for every image | Lose DOSBox-X compatibility |
| DOSBox-X headless and debugger uncertainty | Gone | No capability gaps or workarounds | Lose DOSBox-X |
| Bochs gdbstub build | Gone | No custom engine build | Lose Bochs debugger |
| NAT on DOSBox-X and Bochs | Gone | One NAT implementation | None |
| QEMU `vvfat` development mode | Decide vvfat versus sync without an engine matrix | Simpler and possibly safer default | `vvfat` risk remains if kept |
| COM2 agent mapping | One QEMU serial mapping | Less code | None |
| Native debugger pass-through | QEMU gdbstub and monitor only | One clear debug path and richer QMP control | Lose Bochs internal debugger |
| Display and screenshot | QEMU VNC and monitor only | One display path | None |
| Container writable disk | Use a qcow2 overlay on the read-only image | Fast create, less disk, snapshots | Requires `qemu-img` and QEMU |
| One boot per `RUN` | Use qcow2 snapshots after each command to batch `RUN` steps | Faster builds and per-step layers | Needs a spike and snapshot extraction work |
| Stop and start | Keep the qcow2 overlay. Add `savevm` and `loadvm` optionally | Fast resume and exact machine state | Saved state is tied to the QEMU version |
| FAT image volumes | Add qcow2 overlays and snapshots optionally | Fast volume rollback and snapshots | `qemu-img` dependency and host tooling |
| User-mode NAT | Keep QEMU NAT. Add a shared L2 network with QEMU hub or socket backends | Easier multi-container networking | More work and still deferred |
| Audio | Use QEMU `-audiodev` when audio is added | Simpler audio path | Still deferred |
| CD-ROM and ISO media | Use QEMU `-cdrom` when the feature is added | Easier to add than an engine-neutral design | Needs a DOS CD driver and is still deferred |
| Trusted-image security | Fewer binaries. Add QEMU `-sandbox` on Linux | Smaller surface and optional hardening | TCG is still not a boundary. No sandbox on macOS or Windows |
| MIT or Apache-2.0 license | Review only QEMU's GPL-2.0 terms | Simpler license review | GPL obligations remain if QEMU is bundled |
| Linux, macOS, and Windows hosts | Ship one engine artifact set per host | Simpler packaging, checksums, and CI | None |
| Layered test strategy | Remove the engine matrix and keep QEMU integration | Faster CI and fewer fixtures | No cross-engine tests |
| Documentation | Remove the engine matrix and engine-specific notes | Less documentation | None |
| Future re-add of an engine | Keep the seam only if the drop might be temporary | Lower re-add cost | Small abstraction cost if the seam stays |
| Product fit | A QEMU wrapper with a Doskerfile | Clear and focused | Narrower than a multi-engine DOS container system |

## What Does Not Change

These decisions are engine-neutral or already QEMU-specific. A QEMU-only system keeps them:

- The DOS agent on COM2 and the serial command protocol.
- The line-oriented `attach` shell.
- The FreeDOS base builder from pinned inputs.
- The 8.3 filename policy and the path validation.
- The file-tree canonical image and the FAT projection.
- The local OCI image layout and the lock file.
- The `RUN` inside DOS default and the `RUN --host` staged tree.
- The `ENV`, `WORKDIR`, and `CMD` metadata enforced by the agent.
- The boot metadata capture after `RUN`.
- The build cache key with the parent tree digest and the config digest.
- The `image import` command for DOS and booter images.
- The `exec`, `logs`, `screenshot`, `debug`, `cp`, and `port` commands.
- The per-container supervisor and the daemonless model.
- The exclusive volume attachment and the partitioned FAT16 volume image.
- The default `127.0.0.1` bind for published ports.
- The trusted-image security statement.
- The Windows named-pipe control transport and the Job Object cleanup.
- The MIT or Apache-2.0 license for DOSker.

## What Can Be Removed

- `dosker-engine-dosbox-x` and `dosker-engine-bochs`.
- The adapter trait if the drop is permanent. Keep a `QemuBackend` type instead.
- The `REQUIRES` instruction and the capability negotiation.
- The `ENGINE` instruction and the `--engine` CLI option.
- The `engine_pin` and `requires` fields in the image config.
- The engine name in the lock file.
- The `dosker engines` command. Fold the QEMU check into `dosker doctor`.
- The engine capability matrix.
- M0 spikes 3 and 4.
- M4.
- The DOSBox-X and Bochs sections of the engine research.
- The DOSBox-X and Bochs risks and open questions.
- The adapter contract tests for the other engines.

## What Can Be Simplified

### Doskerfile

A QEMU-only Doskerfile keeps only the portable build and runtime instructions:

```doskerfile
FROM freedos:1.3
MACHINE cpu=486dx2 memory=16M
COPY ./app C:\APP
RUN INSTALL.EXE /S
CMD APP.EXE
```

`ENGINE-CONFIG` becomes `QEMU-CONFIG` or disappears in favor of a richer `MACHINE` and direct QEMU options.

### Engine selection

There is no selection. The build and run paths use QEMU. A missing or mismatched QEMU version is the only engine error. This removes the `REQUIRES` failure path, the engine matrix lookup, and the "engine does not provide this capability" error class.

### Lock file

The lock file keeps the base image digest, the QEMU version, and the agent version. It drops the engine name and the per-engine map.

### Windows

One engine artifact set on Windows. The named-pipe control transport and the Job Object cleanup stay. The `dosker-engine` discovery logic shrinks to one QEMU probe.

### Testing

The unit, mock, QEMU integration, and Windows smoke suites stay. The adapter contract suite can become a QEMU backend contract suite. The macOS and Windows engine discovery tests shrink to one binary.

## What QEMU Unlocks

### qcow2 Overlays For The Writable Disk

The current design copies the image disk into each container. A QEMU-only design can create a qcow2 overlay on the read-only raw image:

```text
qemu-img create -f qcow2 -b base.raw -F raw overlay.qcow2
```

Positive outcomes:

- Fast container create.
- Less disk use.
- The base image stays read-only.
- The overlay persists across stop and start.
- `rm` deletes one file.
- Internal snapshots become available.

Cost: a dependency on `qemu-img` and QEMU-specific disk state. The raw image export for `save` and `load` still works through QEMU tooling.

### qcow2 Snapshots For Build Steps

The multi-engine design uses one boot per `RUN` because other engines cannot attribute changes per command. QEMU can create a new overlay after each command with a QMP block snapshot. The build engine can then extract the FAT from each overlay and compute file deltas.

Positive outcomes:

- Batch several `RUN` steps in one boot.
- Keep one layer per instruction.
- Faster builds for many small `RUN` steps.
- Exact per-step machine state for debugging.

Cost: a spike is needed. The build engine must extract a readable FAT from each overlay. `qemu-img convert` or `qemu-nbd` can do it. The work is QEMU-specific but simpler than an engine-neutral snapshot model.

### Save, Resume, And Snapshots

QEMU can save and restore machine state with `savevm` and `loadvm`. A QEMU-only design can add:

- `dosker pause` and `dosker resume`.
- Exact machine restore across host restarts.
- Snapshot and rollback for containers and volumes.
- A `dosker snapshot` command.

Positive outcomes: fast resume, reproducible debugging states, and an archival feature that is hard to build across engines.

Cost: saved state depends on the QEMU version and the machine configuration. It is not portable across QEMU versions.

### QMP And The Monitor

QEMU's QMP and monitor become the main control surface. A QEMU-only design can use:

- QMP events for shutdown, reset, stop, and guest panic.
- `query-status` for health.
- `screendump` for screenshots.
- `sendkey` for keyboard automation.
- `system_reset` and `system_powerdown`.
- Block jobs and snapshots.
- `human-monitor-command` for advanced commands.

Positive outcomes: a richer and more reliable supervisor. The current design already uses QMP, but the QEMU-only design can lean on it without an engine-neutral abstraction.

### Debugging And Tracing

QEMU offers a gdbstub, monitor commands, `-d` logging, and `-trace`. A QEMU-only design can:

- Attach GDB to a Unix socket or a Windows localhost TCP port.
- Dump CPU and memory state with monitor commands.
- Enable QEMU tracing for performance work.
- Use `-no-shutdown` to pause on guest shutdown for debugging.
- Use `-action` to control reboot and panic behavior.

Positive outcomes: one debug path, more diagnostic data, and fewer engine-specific debugging gaps.

### Shared L2 Networking

QEMU can build a virtual hub with `-netdev socket` or `hubport`. A QEMU-only design can add a shared L2 network for client-server tests and multi-node BBS setups without an engine-neutral switch.

Positive outcomes: direct container-to-container networking and a stronger BBS and client-server story.

Cost: a virtual network manager, address allocation, and more tests. It is still a later milestone.

### Audio, CD-ROM, And Hardware

QEMU supports `-audiodev`, `-cdrom`, rich `-device` models, and multiple machine types. A QEMU-only design can expose these directly through `MACHINE` or `QEMU-CONFIG`.

Positive outcomes: audio, CD-ROM, and hardware options are easier to add. The `MACHINE` mapping code becomes a QEMU option generator, not a cross-engine translation layer.

### Linux Sandbox

QEMU supports `-sandbox` with seccomp on Linux. A QEMU-only design can offer an opt-in hardening mode.

Positive outcomes: defense in depth for CI. It does not replace the outer VM because TCG is still not a security boundary, and the option is not available on macOS or Windows.

## Two Shapes Of A QEMU-Only System

### Clean cut

Remove the adapter trait, the capability names, the `REQUIRES` and `ENGINE` instructions, and the engine crates. Use a `QemuBackend` type directly. This is the smallest and clearest codebase. It gives up the option to add an engine later without design work.

### Seam-preserving cut

Keep the `EngineAdapter` trait, the `EngineSpec`, and the capability names, but ship only the QEMU implementation. Remove the `ENGINE` and `REQUIRES` instructions from user-facing Doskerfiles, or keep them but hide them. This keeps most of the simplification and leaves a clean re-add path. The cost is a small unused seam and a contract test for one implementation.

Recommendation for a temporary drop: use the seam-preserving cut. Recommendation for a permanent drop: use the clean cut.

## What To Keep Even In A QEMU-Only System

- The file-tree canonical image and the FAT projection. They keep the image portable and inspectable.
- The DOS agent and the serial protocol. They are engine-independent.
- The FreeDOS base builder.
- The OCI image layout and the lock file.
- The 8.3 filename policy.
- The staged `RUN --host` model.
- The trusted-image security statement.
- The per-container supervisor and the named-pipe or Unix-socket control transport.
- The capability names if the drop may be temporary.

## Positive Outcomes Summary

### Delivery

- One engine to implement, test, document, and package.
- Two fewer adapter crates.
- Two fewer engine artifact sets per host.
- Two fewer M0 spikes.
- No M4.
- Faster M1 and M1-W.
- Simpler Windows support.
- Faster CI and fewer flaky tests.
- Smaller documentation set.

### Design

- Smaller Doskerfile and image config.
- Simpler lock file.
- Fewer error paths.
- One configuration format.
- One debug path.
- One display and screenshot path.
- One NAT implementation.
- One engine version to reason about.

### QEMU-Specific Gains

- qcow2 overlays for fast container create.
- qcow2 snapshots for build batching and per-step layers.
- Save, resume, and snapshot features.
- QMP events for lifecycle and health.
- Monitor commands for screenshots, keyboard, and reset.
- GDB, QEMU logging, and tracing.
- Shared L2 networking through a virtual hub.
- Audio, CD-ROM, and richer hardware options.
- Linux sandbox hardening.

### Operations

- One engine binary per host to verify.
- One checksum and one version pin.
- One engine probe in `dosker doctor`.
- Simpler macOS and Windows packaging.
- Fewer license and security reviews.

## Costs And Limits

The fallback analysis records the full negative list. The main limits are:

- DOSBox-X compatibility is lost for games, sound, and timing-sensitive software.
- Bochs accuracy and its internal debugger are lost.
- macOS arm64 loses the native DOSBox-X build and keeps QEMU under TCG.
- The original multi-engine requirement is not met.
- A single engine is a single point of failure.
- qcow2 features add a QEMU and `qemu-img` dependency.
- The abstraction is not exercised, so a later re-add costs more unless the seam is kept.

## Decision Gate

Adopt the QEMU-only shape only after these conditions are true:

- QEMU passes the agreed build-and-test use cases.
- The DOSBox-X and Bochs spikes show that their cost is higher than their value.
- The user accepts the loss of the original multi-engine requirement.
- The Windows and macOS QEMU paths are proven.
- The qcow2 changes are proven by a spike.

If the drop is temporary, keep the seam. If the drop is permanent, remove the seam and take the clean cut.

## Recommendation

Keep the multi-engine design for now. The QEMU-only option is a credible simplification and a strong fallback if the other engines do not pay for themselves. The most valuable QEMU-only changes are the qcow2 overlay for container disks, qcow2 snapshots for build batching, QMP events, and the simpler Windows path. Revisit the decision after M4 with evidence.
