# Multi-engine decision after M4

## Status

Accepted, 2026-11-21.

## Decision

Keep multi-engine. QEMU remains the required engine and the only engine for
the build path, in-DOS `RUN`, and NAT. DOSBox-X and Bochs remain
capability-gated runtime engines behind `REQUIRES`.

## Evidence

- `assets/dosbox-x-spike.md`
- `assets/bochs-spike.md`
- `assets/engine-fallback-analysis.md`
- `assets/qemu-only-deep-dive.md`
- `crates/dosker-engine/src/provider.rs`

The design capability matrix and the table in `docs/engines.md` are the
design-time and current summaries.

## Capability evidence

QEMU proves every declared capability. DOSBox-X proves `boot`, `serial`,
`stop`, `headless`, and `debug_native`. Bochs proves `boot`, `serial`, `stop`,
`headless`, and `debug_native`; `gdbstub` is proven only in an optional
gdbstub build and is not declared in the reference capability set.

## Unproven cells

DOSBox-X: `host_dir_live` and `nat` are unproven; `gdbstub`, `screenshot`, and
`vnc` are not tested and not declared.

Bochs: `host_dir_live`, `screenshot`, `vnc`, and `nat` are not declared; `vnc`
is build-dependent and untested; `gdbstub` is not in the reference build and is
not declared.

## Adapters and tests

- `crates/dosker-engine-dosbox-x` with
  `crates/dosker-cli/tests/dosbox_x_runtime.rs` and
  `crates/dosker-engine-dosbox-x/tests/discovery.rs`.
- `crates/dosker-engine-bochs` with
  `crates/dosker-cli/tests/bochs_runtime.rs`,
  `crates/dosker-engine-bochs/tests/discovery.rs`, and
  `crates/dosker-engine-bochs/tests/gdbstub_build.rs`.

## Alternative considered

A QEMU-only system, using the criteria in
`assets/engine-fallback-analysis.md`. Both adapters proved boot, serial,
headless, and native debugging, so the "both adapters remain unproven after
M4" trigger did not fire. The unproven cells are capability-gated and do not
block the QEMU path.

## Consequences

- Release artifacts do not bundle engines.
- The install documentation pins the engine versions.
- Unproven capabilities stay rejected.
- QEMU stays the default engine.

## Re-evaluation triggers

- A required application fails on QEMU.
- Both adapters remain unproven after a later milestone.
- Packaging three engines becomes a release blocker.
