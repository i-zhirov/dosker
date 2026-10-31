# DOSker — Multi-Engine And QEMU-Only Fallback Analysis

This asset records the positive outcomes and the negative drawbacks of a later decision to drop DOSBox-X and Bochs and keep only QEMU. The current decision is to keep the multi-engine design.

The question-by-question QEMU-only change set and its positive outcomes are in [`qemu-only-deep-dive.md`](qemu-only-deep-dive.md).

## Current Decision

Keep the multi-engine design. QEMU is the only engine required for M1 and M2. DOSBox-X and Bochs remain capability-gated adapters, and their spikes must not block the QEMU slice. Re-evaluate after M4, when the DOSBox-X and Bochs spikes and adapters have produced evidence.

## Comparison

| Dimension | Multi-engine (current) | QEMU-only (fallback) |
| --- | --- | --- |
| Engine adapters | Three adapters, three configuration formats, three control paths | One adapter, one control path |
| Capability model | Required and exercised by three engines | Mostly vestigial, but still useful as a contract |
| DOS application compatibility | DOSBox-X covers games, sound, and timing-sensitive software | QEMU covers fewer DOS applications |
| Debugging | QEMU gdbstub, Bochs internal debugger, DOSBox-X debugger if proven | QEMU gdbstub only |
| Host platforms | Three engine artifact sets on each host | One engine artifact set on each host |
| CI cost | QEMU integration plus engine-specific smoke tests | QEMU integration only |
| Maintenance | Engine-specific bugs, versions, and spikes | One engine to track |
| Reproducibility | One pin per engine | One pin total |
| Packaging | Three binaries to find, verify, and pin | One binary to find, verify, and pin |
| Requirement fit | Matches the original requirement | Contradicts the original requirement |
| Future flexibility | Proven abstraction, easy to add another engine | Abstraction may rot |

## Positive Outcomes If We Drop DOSBox-X And Bochs

1. **Smaller codebase.** One adapter crate, one configuration generator, and one control path. The `dosker-engine-dosbox-x` and `dosker-engine-bochs` crates and their tests disappear.
2. **Smaller capability surface.** The `REQUIRES` matrix has no unproven cells. There is no need to map `headless`, `host_dir_live`, `debug_native`, `vnc`, `nat`, or `screenshot` across three engines.
3. **Lower maintenance.** No DOSBox-X `BOOT` divergence, no DOSBox-X debugger build question, no Bochs `--enable-gdb-stub` build, and no Bochs serial socket quirks.
4. **Simpler packaging.** One engine binary per host. Fewer checksums, fewer version pins, and fewer release artifacts.
5. **Simpler CI.** One engine integration suite. No DOSBox-X or Bochs runners, and no engine-specific flaky tests.
6. **More consistent behavior.** QMP, gdbstub, serial, VNC, screenshots, NAT, and shutdown work one way.
7. **Easier Windows support.** One Windows engine build to find and verify, not three.
8. **Faster M0 and M1.** The DOSBox-X and Bochs spikes are removed from the critical path.
9. **Smaller documentation.** One capability matrix, one set of engine notes, and one troubleshooting path.
10. **Lower security surface.** Fewer external binaries and fewer control channels.

## Negative Drawbacks If We Drop DOSBox-X And Bochs

1. **Loss of DOS compatibility.** DOSBox-X is the most compatible engine for games, sound, and timing-sensitive DOS software. QEMU covers fewer DOS applications.
2. **Loss of native Apple Silicon speed.** DOSBox-X has an official arm64 build. QEMU x86 guests on macOS arm64 run under TCG.
3. **Loss of accurate debugging.** Bochs has a strong internal debugger and accurate CPU emulation. QEMU's gdbstub is weaker for some 16-bit real-mode work.
4. **Loss of the engine comparison.** Users cannot compare behavior across engines for debugging or preservation.
5. **Loss of the fallback path.** When QEMU fails on an application, there is no second engine to try.
6. **The abstraction becomes unproven.** The adapter trait and capability model may rot because only one adapter exercises them.
7. **Re-adding an engine becomes harder.** The code path and tests must be rebuilt later.
8. **The original requirement is not met.** `task.md` asks for several engines.
9. **Some users are excluded.** BBS software, games, and real-mode debugging may need DOSBox-X or Bochs.
10. **Possible rework.** A later change of mind costs more than keeping the adapters now.
11. **The capability model looks like dead weight.** New contributors may remove it, which destroys the option to add engines later.
12. **Windows and macOS packaging become simpler, but less capable.** The tool becomes a QEMU wrapper with a Doskerfile, not a multi-engine system.

## What We Keep Now To Preserve The Option

- The `EngineAdapter` trait and the required core.
- The capability names and the `REQUIRES` instruction.
- One crate per adapter, even when an adapter is feature-gated.
- The `EngineSpec` translation layer.
- The `ENGINE` instruction and `ENGINE-CONFIG` escape hatch.
- The engine pin in the lock file.
- The adapter contract test suite with a mock engine.
- The capability matrix in the design and in `dosker engines`.
- The engine research asset and the M0 spike list.
- The engine-neutral image, layer, agent, and build model.

## Decision Criteria

Drop DOSBox-X and Bochs only when one of these is true:

- Both adapters remain unproven after M4.
- QEMU meets every target application in the agreed use cases.
- The maintenance, CI, and packaging cost is higher than the compatibility benefit.
- Windows or macOS packaging of three engines becomes a release blocker.

Keep DOSBox-X and Bochs when one of these is true:

- A required application fails on QEMU.
- Games, BBS software, or timing-sensitive software need DOSBox-X.
- Real-mode debugging needs Bochs.
- The user wants cross-engine comparison or preservation.
- The multi-engine requirement remains firm.

## Cost Of The Timing

- **Drop now:** saves M0 spikes 3 and 4, M4, and two engine artifact sets. It removes the original multi-engine requirement and future flexibility.
- **Drop later:** the adapters and tests may have decayed. The cost is higher, but the evidence is better.
- **Keep now:** pays the adapter and spike cost, but keeps the requirement, the fallback, and the option.

## Recommendation

Keep the multi-engine design. Finish the QEMU slice first. Keep the DOSBox-X and Bochs adapters behind capability gates and do not let them block M1 or M2. Re-evaluate after M4 with evidence from the spikes and real use. The design elements listed above keep the QEMU-only fallback cheap if the decision changes.
