# Bochs Spike

- **Date**: 2026-11-20
- **Status**: Complete
- **Purpose**: Settle the Bochs cells of the engine capability matrix before
  the Bochs adapter is planned for execution.
- **Consumes**: `design.md` §Engine Capability Matrix, §Testing Strategy;
  `assets/engine-research.md` §Bochs; `contracts/agent-protocol.md`;
  `contracts/doskerfile.md` §ENGINE, §ENGINE-CONFIG.
- **Feeds**: the `dosker-engine-bochs` implementation plan and the Bochs
  column of the capability table.

## Ground Rules

- The spike is throwaway. Do not add product code, and do not commit a spike
  harness.
- Work on copies of the fixture images under a scratch directory. Never boot
  or write the fixtures in place.
- Record raw evidence (command lines, bochsrc files, serial transcripts, exit
  codes, timings) under `assets/bochs-spike/<spike>/`. The results table at the
  end is the only thing the plan reads.
- A capability is declared only when its spike row says `proven`. An unproven
  row stays undeclared, and the adapter rejects a `REQUIRES` for it.
- If the `BOOT` spike fails, stop: the slice cannot meet its acceptance bar and
  the roadmap needs a new decision.

## Environment

Record all of these before the first spike:

| Item | Value |
| --- | --- |
| Host OS and architecture | macOS 15.5 (Darwin 24.5.0), arm64 |
| Bochs path | `/opt/homebrew/bin/bochs` (reference); `$HOME/bochs-gdbstub/bin/bochs` (gdbstub build) |
| `bochs --version` first line | `Bochs x86 Emulator 3.1` / `Built from GitHub snapshot on August 23, 2026` |
| Bochs source | Homebrew `bochs 3.1`; local `REL_3_1_FINAL` build for the gdbstub |
| Build flags | Homebrew: `--enable-debugger --enable-debugger-gui --with-nogui --with-sdl2 --with-term`. gdbstub: `--enable-gdb-stub --with-nogui --enable-cpu-level=6` |
| SHA-256 of the binary | Homebrew `8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c`; gdbstub `56aa939652578a81bb2e0f915fc48b80643053e24a5e046ba7af86c65643ae95` |
| Whether the build includes the internal debugger | Yes (Homebrew; prompt `bochs:1>`) |
| Whether the build includes the gdbstub | Yes in the local build; no in the Homebrew build |
| Bochs share directory (`$BXSHARE`) | `/opt/homebrew/share/bochs`; `$HOME/bochs-gdbstub/share/bochs` |
| Fixture image path and SHA-256 (`dosker-agent.img`) | `agent/fixtures/dosker-agent.img`, `17a621ab7e3658091104f367e5b4bb23f81b608bbd8760c4be6b5cf46c1131df` |
| Fixture image path and SHA-256 (`dosker-net.img`) | `agent/fixtures/dosker-net.img`, `c9621effdc3746ea3aa630988057d8c397f25418e08ce0ce4ff9090fd077dfb3` |
| Host tooling (`gdb`, `socat`, `mcopy`, `curl`) | `gdb` and `lldb` present; `socat` missing; mtools and `curl` present |

The Homebrew build is `bochs 3.1` on macOS arm64. It is configured with
`--with-nogui --with-sdl2 --with-term`; the research does not say whether it
includes the debugger or the gdbstub. The internal debugger needs a build with
`--enable-debugger`; the gdbstub needs `--enable-gdb-stub`. The two flags are
mutually exclusive, so a full debugger spike may need two builds.

The reference config is the sample `bochsrc` from the installed share
directory, or the upstream `bochsrc-sample.txt`. Read it before each spike; it
documents every key named below.

### Fixtures

- `agent/fixtures/dosker-agent.img` — a partitioned FreeDOS IDE disk with
  `C:\DOSKER\DOSKER.EXE` and an `AUTOEXEC.BAT` that starts the agent. The agent
  reads `C:\DOSKER\CONFIG.INI` for the control port and baud.
- `agent/fixtures/dosker-net.img` — a booter image that serves one HTTP
  response on a guest TCP port, used by the published-port suite.

Before a spike, copy the image and write the agent config into the copy:

```sh
cp agent/fixtures/dosker-agent.img "$HOME/bochs-spike/agent.img"
printf 'CONTROL_PORT=2\r\nBAUD=115200\r\n' > "$HOME/bochs-spike/CONFIG.INI"
mcopy -o -i "$HOME/bochs-spike/agent.img@@1M" \
  "$HOME/bochs-spike/CONFIG.INI" ::/DOSKER/CONFIG.INI
```

If `mcopy` cannot address the partition, use a one-off Rust program calling
`dosker_image::write_config(&image, 2, 115200)` and record that instead. The
image must contain exactly `CONTROL_PORT=2\r\nBAUD=115200\r\n` before the
`BOOT` spike.

### Disk geometry

Bochs requires the image size to equal `C*H*S*512` exactly. The 32 MiB fixture
is `128` cylinders, `16` heads, and `32` sectors per track, which matches the
partition table written by the fixture builder. Record the geometry Bochs
accepts, and whether `mode=flat` boots the partitioned image.

### Host agent client

The BOOT and headless spikes need the host side of the agent protocol. Write a
throwaway client that:

1. Connects to the Bochs serial endpoint (TCP by default).
2. Uses `dosker-agent-proto::AgentClient` to send `HELLO`, then `EXEC ECHO
   HELLO`.
3. Prints the `HELLO` fields, the `EXEC` output bytes, and the status.

Use `crates/dosker-agent-proto/tests/qemu_handshake.rs` and its `support`
module as the reference. Keep the client outside the repository or delete it
after the spike.

### Serial endpoint candidates

Bochs serial modes are `null`, `term`, `file`, `raw`, `mouse`, `socket-client`,
`socket-server`, `pipe-client`, and `pipe-server`. The agent owns COM2. Try, in
order, and record the first that carries the agent frames:

```ini
com2: enabled=1, mode=socket-server, dev=localhost:5500
com2: enabled=1, mode=socket-client, dev=localhost:5500
com2: enabled=1, mode=term, dev=<pty>
```

`socket-server` makes Bochs listen and the host connect; `socket-client` makes
Bochs connect to a host listener. The endpoint kind and direction are part of
the adapter decision.

## Spike 1 — BOOT, serial, exec, and stop

**Question.** Can Bochs boot `dosker-agent.img`, reach the agent `HELLO` on the
serial link, run `EXEC ECHO HELLO` with status 0, and stop cleanly?

**Why it matters.** `BOOT` is the only path to running a real DOS image on
Bochs. Without it the adapter cannot run DOSker images, and `Boot`, `Serial`,
and `Stop` stay undeclared.

**Procedure.**

1. Build a `bochsrc` with the candidate serial line and the disk:

   ```ini
   romimage: file=$BXSHARE/BIOS-bochs-latest
   vgaromimage: file=$BXSHARE/VGABIOS-lgpl-latest.bin
   cpu: model=i486dx4
   megs: 16
   ata0-master: type=disk, path="$HOME/bochs-spike/agent.img", mode=flat, cylinders=128, heads=16, spt=32
   boot: disk
   com1: enabled=1, mode=null
   com2: enabled=1, mode=socket-server, dev=localhost:5500
   display_library: nogui
   log: $HOME/bochs-spike/bochs.log
   ```

2. Start Bochs with `bochs -q -f <bochsrc>` and no window.
3. Connect the host agent client to the serial endpoint. Record the `HELLO`
   fields (`protocol_version`, `agent_version`, `control_port`, `max_payload`,
   `features`) and the boot time.
4. Run `EXEC ECHO HELLO`. Record the output bytes and status.
5. Stop the container the way the adapter would:
   - Send the agent `SHUTDOWN` and observe whether the process exits.
   - If it does not, try `SIGTERM`, the internal debugger `quit` when the build
     has it, and `-rc` startup commands. Record the first clean mechanism and
     the exit code.
6. Record whether a second `HELLO` appears (guest reboot) and how the process
   behaves after the guest halts.

**Evidence to record.**

- The working `bochsrc` and the full command line.
- The serial line form that worked and its direction.
- The `HELLO` fields, `EXEC` output, and status.
- The boot time from process start to `HELLO`.
- The stop mechanism, the process exit code, and the time to exit.
- The geometry Bochs accepted and whether the partitioned image booted.
- Any Bochs log warning or error.

**Result.** The fixture booted in about 2.7 s. `HELLO` carried protocol 1,
control port 2, `max_payload` 4096, and `EXEC ECHO HELLO` returned `HELLO`
with status 0. The agent `SHUTDOWN` was acknowledged but did not exit Bochs;
`SIGTERM` exited with code 143 in about 0.01 s. `cpu: model=486` is invalid;
the 486 model is `i486dx4`. Bochs accepted `cylinders=128, heads=16, spt=32`.

**Pass criteria.**

- `HELLO` arrives within 60 seconds with protocol version 1 and control port 2.
- `EXEC ECHO HELLO` returns `HELLO` and status 0.
- A chosen stop mechanism exits the process with a recorded exit code, or a
  recorded kill is the only fallback.

**Decision if it fails.** Stop the slice. Record the failure and the next
option (for example, a floppy image, a different disk mode, or dropping Bochs
to a shell-only engine) for a roadmap revision.

## Spike 2 — Headless, long-running

**Question.** Can Bochs run with no window and stay alive while the guest
runs, so a detached container can serve the agent?

**Why it matters.** `nogui` is documented as "no display at all". If the only
headless mode exits or needs a terminal, the adapter cannot host a long-running
container without a display, and `Headless` stays undeclared.

**Procedure.**

1. Repeat the BOOT spike with each candidate, recording the process lifetime
   and whether a window appears:

   ```sh
   bochs -q -f <bochsrc>                      # display_library: nogui
   bochs -q -f <bochsrc>                      # display_library: term
   ```

2. For each, check whether the process stays alive after `HELLO` and whether
   the agent still answers `EXEC`.
3. Record the display library, the terminal requirement, and any display
   error.
4. Check whether `-q` changes the startup behavior when the config file is
   valid.

**Evidence to record.**

- Each candidate `bochsrc` and command line.
- Window present: yes or no.
- Process lifetime after `HELLO` (at least 60 seconds).
- Whether `EXEC` still answers while headless.
- Any display-library or terminal error.

**Pass criteria.** One candidate runs with no window, stays alive for at least
60 seconds, and keeps the agent link working.

**Decision if it fails.** Leave `Headless` undeclared. The adapter must then
require a display, and `run -d` on Bochs is rejected with a message that names
the missing capability.

## Spike 3 — Debugger

**Question.** Which debugger does the build include, and can a detached
container expose a scriptable debugger channel without an attached terminal?

**Why it matters.** `DebugNative` is only useful through `dosker debug`. The
internal debugger reads commands from stdin and `-rc`; the gdbstub is a TCP
socket. The adapter needs one non-interactive endpoint, or the capability stays
undeclared.

**Procedure.**

1. Check which debugger the build includes:
   - `bochs -dbg -q -f <bochsrc>` for the internal debugger.
   - `gdbstub: enabled=1, port=1234` plus `bochs -q -f <bochsrc>` for the
     gdbstub.
   Record which starts and what it prints.
2. Internal debugger:
   - Start with `-dbg` and stdin from a pipe or a file, not a terminal.
   - Feed `r`, `s`, `c`, and `quit`; record the output and whether the process
     exits.
   - Try `-rc <file>` with the same commands at startup.
3. gdbstub:
   - Connect with `gdb` (`target remote :1234`), send `info registers`, and
     record the response.
   - Confirm whether 16-bit real mode works.
4. Confirm that the channel survives a detached process (no controlling
   terminal, stdin closed) and that the command output is scriptable.

**Evidence to record.**

- Debugger present in the build: internal, gdbstub, both, or neither.
- The exact startup flags and config keys.
- The endpoint kind (stdin, `-rc`, TCP, or terminal only).
- A sample command and response, if any.
- Whether the channel works with stdin closed and no terminal.

**Pass criteria.** A non-interactive endpoint accepts at least one debugger
command from a host process while the container runs detached.

**Decision if it fails.** Leave `DebugNative` undeclared. `dosker debug` on
Bochs reports the capability unavailable and names QEMU or DOSBox-X.

## Deferred spikes

The host-directory and NAT cells are not covered by this spike. The design
matrix expects `vvfat` for host directories and leaves NAT unproven. Unless
their own spikes run, the Bochs plan marks `host_dir_live` and `nat` unproven
and the adapter rejects `--bind` and `-p`.

## Results

Fill this table before planning execution. The adapter's capability set is the
set of rows marked `proven`.

| Capability | Result | Evidence |
| --- | --- | --- |
| `boot` | proven | `ata0-master` with CHS 128/16/32 and `boot: disk` boots the copy; `HELLO` in ~2.7 s. `assets/bochs-spike/boot/` |
| `serial` | proven | `com2: enabled=1, mode=socket-server, dev=localhost:<port>`; Bochs listens, the host connects; `EXEC ECHO HELLO` returns `HELLO` and status 0. `assets/bochs-spike/boot/serial-transcript.txt` |
| `stop` | proven | Agent `SHUTDOWN` is acknowledged but Bochs stays alive; `SIGTERM` exits with code 143 in ~0.01 s. `assets/bochs-spike/boot/stop-transcript.txt` |
| `headless` | proven | `display_library: nogui`: no window, alive past 65 s, `EXEC` answers. `assets/bochs-spike/headless/` |
| `host_dir_live` | not tested; do not declare | — |
| `debug_native` | proven | Homebrew `-dbg` internal debugger reads `r`/`s`/`quit` from a pipe, needs no terminal, and exits 0 on `quit`. `assets/bochs-spike/debugger/internal-session.txt` |
| `gdbstub` | proven in a gdbstub build; not in the Homebrew reference build | `gdbstub: enabled=1, port=1235`; GDB connects, reads 16-bit real-mode registers, and disassembles the BIOS. Binds all interfaces. `assets/bochs-spike/debugger/gdb-session.txt` |
| `screenshot` | not tested; do not declare | — |
| `vnc` | build-dependent; not tested | — |
| `nat` | not tested; do not declare | — |

Also record the adapter decisions the plan consumes:

| Decision | Value |
| --- | --- |
| Working boot command and bochsrc | `bochs -q -f <bochsrc>` with `ata0-master: type=disk, path="<image>", mode=flat, cylinders=128, heads=16, spt=32`, `boot: disk`, `cpu: model=i486dx4`, `megs: 16`, `$BXSHARE` ROMs, and `display_library: nogui` |
| Working serial line | `com2: enabled=1, mode=socket-server, dev=localhost:<port>`; `com1: enabled=1, mode=null` is the guest line |
| Serial endpoint kind and direction | TCP; Bochs listens and the host connects |
| Working headless flags | `display_library: nogui` |
| Stop mechanism and exit code | Agent `SHUTDOWN`, then `SIGTERM`; exit code 143 in ~0.01 s |
| Debug endpoint kind, if any | Internal debugger: pty master held by the adapter, drained, and bridged to a Unix socket for the client (`-dbg`, prompt `bochs:1>`, `c` to boot, `SIGINT` to break, `quit` exits 0). gdbstub: TCP (`gdbstub: enabled=1, port=<port>`) in a gdbstub build; 16-bit real mode works; binds all interfaces |
| Host-directory mechanism, if any | None tested; do not declare |
| NAT backend and port-forward key, if any | None tested; do not declare |
| Pinned Bochs version and checksum | 3.1; Homebrew `8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c`; gdbstub `56aa939652578a81bb2e0f915fc48b80643053e24a5e046ba7af86c65643ae95` |

## Evidence

Raw evidence is under `assets/bochs-spike/`:

- `environment.md` — host, both binaries, build flags, fixtures, tooling.
- `boot/` — working `bochsrc`, serial transcript, stop transcript, logs.
- `headless/` — `nogui` config, transcript, and startup log.
- `debugger/` — internal debugger session, pty session/config/log, and gdbstub session/config/log.

## Reporting

The Results and Decisions tables above are complete. `BOOT`, `serial`, and
`headless` are `proven`; `debug_native` is proven with the internal debugger;
`gdbstub` is proven only in a gdbstub build. The plan is written from these
rows.
