# DOSBox-X Spike

- **Date**: 2026-11-19
- **Status**: Ready to run
- **Purpose**: Settle the DOSBox-X cells of the engine capability matrix before
  the DOSBox-X adapter is planned for execution.
- **Consumes**: `design.md` §Engine Capability Matrix, §Testing Strategy;
  `assets/engine-research.md` §DOSBox-X; `contracts/agent-protocol.md`;
  `contracts/doskerfile.md` §ENGINE, §ENGINE-CONFIG.
- **Feeds**: the `dosker-engine-dosbox-x` implementation plan and the
  DOSBox-X column of the capability table.

## Ground Rules

- The spike is throwaway. Do not add product code, and do not commit a spike
  harness.
- Work on copies of the fixture images under a scratch directory. Never boot
  or write the fixtures in place.
- Record raw evidence (command lines, config files, serial transcripts, exit
  codes, timings) under `assets/dosbox-x-spike/<spike>/`. The results table at
  the end is the only thing the plan reads.
- A capability is declared only when its spike row says `proven`. An unproven
  row stays undeclared, and the adapter rejects a `REQUIRES` for it.
- If the `BOOT` spike fails, stop: the slice cannot meet its acceptance bar and
  the roadmap needs a new decision.

## Environment

Record all of these before the first spike:

| Item | Value |
| --- | --- |
| Host OS and architecture | |
| DOSBox-X path | |
| `dosbox-x --version` first line | |
| DOSBox-X source (Homebrew, official release, build) | |
| SHA-256 of the binary | |
| Whether the build includes the debugger | |
| Fixture image path and SHA-256 (`dosker-agent.img`) | |
| Fixture image path and SHA-256 (`dosker-net.img`) | |
| Host tooling (`socat`, `mcopy`, `mtools`, `curl`) | |

The reference build is DOSBox-X `2026.10.01` SDL2. On this host it is
`/opt/homebrew/bin/dosbox-x` and reports:

```text
DOSBox-X version 2026.10.01 SDL2, copyright 2011-2026 The DOSBox-X Team.
```

The reference config is at
`/opt/homebrew/share/dosbox-x/dosbox-x.reference.full.conf` (or the equivalent
path for the installed build). Read it before each spike; it documents every
key named below.

### Fixtures

- `agent/fixtures/dosker-agent.img` — a partitioned FreeDOS IDE disk with
  `C:\DOSKER\DOSKER.EXE` and an `AUTOEXEC.BAT` that starts the agent. The agent
  reads `C:\DOSKER\CONFIG.INI` for the control port and baud.
- `agent/fixtures/dosker-net.img` — a booter image that serves one HTTP
  response on a guest TCP port, used by the published-port suite.

Before a spike, copy the image and write the agent config into the copy:

```sh
cp agent/fixtures/dosker-agent.img /tmp/dosbox-x-spike/agent.img
printf 'CONTROL_PORT=2\r\nBAUD=115200\r\n' > /tmp/dosbox-x-spike/CONFIG.INI
mcopy -i /tmp/dosbox-x-spike/agent.img@@1M ::/DOSKER/CONFIG.INI /tmp/dosbox-x-spike/CONFIG.INI
```

If `mcopy` cannot address the partition, use a one-off Rust program calling
`dosker_image::write_config(&image, 2, 115200)` and record that instead. The
image must contain exactly `CONTROL_PORT=2\r\nBAUD=115200\r\n` before the
`BOOT` spike.

### Host agent client

The BOOT and headless spikes need the host side of the agent protocol. Write a
throwaway client that:

1. Connects to the DOSBox-X serial endpoint (TCP by default).
2. Uses `dosker-agent-proto::AgentClient` to send `HELLO`, then `EXEC ECHO
   HELLO`.
3. Prints the `HELLO` fields, the `EXEC` output bytes, and the status.

Use `crates/dosker-agent-proto/tests/qemu_handshake.rs` and its `support`
module as the reference. Keep the client outside the repository or delete it
after the spike.

### Serial endpoint candidates

The reference config documents `serialX=nullmodem` with `server`, `port`,
`sock`, and `pipe` parameters on the same line. The exact TCP server form is
not settled. Try, in order, and record the first that accepts a connection:

```ini
[serial]
serial1=nullmodem server:127.0.0.1 port:5000 sock:0
serial1=nullmodem server:5000 sock:0
serial1=nullmodem port:5000 sock:0
serial1=nullmodem server:5000
```

`pipe:<path>` (server) and `sock:2` (Unix socket or named pipe) are the
fallbacks if TCP does not work.

## Spike 1 — BOOT, serial, exec, and stop

**Question.** Can DOSBox-X boot `dosker-agent.img`, reach the agent `HELLO` on
the serial link, run `EXEC ECHO HELLO` with status 0, and stop cleanly?

**Why it matters.** `BOOT` is the only path to running a real DOS image on
DOSBox-X. Without it the adapter cannot run DOSker images, and `Boot`,
`Serial`, and `Stop` stay undeclared.

**Procedure.**

1. Start from a windowed run so failures are visible.
2. Build a config file with the candidate serial line and an `[autoexec]`
   that boots the copy. Try, in order, and record the first that reaches the
   guest:
   ```ini
   [autoexec]
   IMGMOUNT C /tmp/dosbox-x-spike/agent.img -hdd
   BOOT -l c
   ```
   ```ini
   [autoexec]
   BOOT /tmp/dosbox-x-spike/agent.img
   ```
3. Start DOSBox-X with `-conf <file>` and no `-exit`.
4. Connect the host agent client to the serial endpoint. Record the `HELLO`
   fields (`protocol_version`, `agent_version`, `control_port`, `max_payload`,
   `features`) and the boot time.
5. Run `EXEC ECHO HELLO`. Record the output bytes and status.
6. Stop the container the way the adapter would:
   - Send the agent `SHUTDOWN` and observe whether the process exits.
   - If it does not, try the monitor socket (`[dosbox] monitor`), `-c "EXIT"`,
     or a signal. Record the first clean mechanism and the exit code.
7. Record whether a second `HELLO` appears (guest reboot) and how the process
   behaves after the guest halts.

**Evidence to record.**

- The working config file and the full command line.
- The serial line form that worked.
- The `HELLO` fields, `EXEC` output, and status.
- The boot time from process start to `HELLO`.
- The stop mechanism, the process exit code, and the time to exit.
- Whether `BOOT` accepts a partitioned IDE image, a raw booter image, or both.
- Any DOSBox-X warning or error in its log.

**Pass criteria.**

- `HELLO` arrives within 60 seconds with protocol version 1 and control port 2.
- `EXEC ECHO HELLO` returns `HELLO` and status 0.
- A chosen stop mechanism exits the process with a recorded exit code, or a
  recorded kill is the only fallback.

**Decision if it fails.** Stop the slice. Record the failure and the next
option (for example, `IMGMOUNT` only, a different machine type, or dropping
DOSBox-X to a shell-only engine) for a roadmap revision.

## Spike 2 — Headless, long-running

**Question.** Can DOSBox-X run with no window and stay alive while the guest
runs, so a detached container can serve the agent?

**Why it matters.** `-silent` is documented to exit after `AUTOEXEC.BAT`. If
the only headless mode exits, the adapter cannot host a long-running container
without a display, and `Headless` stays undeclared.

**Procedure.**

1. Repeat the BOOT spike with each candidate, recording the process lifetime
   and whether a window appears:
   ```sh
   dosbox-x -conf <file> -set "sdl videodriver=dummy"
   dosbox-x -conf <file> -silent
   dosbox-x -conf <file> -set "sdl output=surface" -set "sdl videodriver=dummy"
   ```
2. For each, check whether the process stays alive after `HELLO` and whether
   the agent still answers `EXEC`.
3. Record the display driver, the output driver, and any SDL error.
4. Check whether `-silent` changes the exit behavior when `AUTOEXEC` runs
   `BOOT` (the guest does not return until it exits).

**Evidence to record.**

- Each candidate command line and config.
- Window present: yes or no.
- Process lifetime after `HELLO` (at least 60 seconds).
- Whether `EXEC` still answers while headless.
- Any SDL or video-driver error.

**Pass criteria.** One candidate runs with no window, stays alive for at least
60 seconds, and keeps the agent link working.

**Decision if it fails.** Leave `Headless` undeclared. The adapter must then
require a display, and `run -d` on DOSBox-X is rejected with a message that
names the missing capability.

## Spike 3 — Debugger

**Question.** Do the official and Homebrew builds include a debugger, and can a
detached container expose a scriptable debugger channel without an attached
terminal?

**Why it matters.** `DebugNative` is only useful through `dosker debug`. An
ncurses debugger that needs the container's terminal cannot serve a detached
supervisor. The MCP channel is the only candidate for a non-interactive
endpoint.

**Procedure.**

1. Check whether the build includes the debugger:
   ```sh
   dosbox-x -break-start -conf <file>
   ```
   Record whether it breaks and what it prints.
2. Try the MCP channel with the reference key:
   ```sh
   dosbox-x -conf <file> -set "dosbox mcp_server=<port>"
   ```
   Record the socket address, the protocol, and whether a client can connect
   without a terminal.
3. Try the monitor socket (`[dosbox] monitor`) and record whether it exposes
   debugger commands.
4. Confirm that the channel survives a detached process (no controlling
   terminal, stdin closed).

**Evidence to record.**

- Debugger present in the build: yes or no.
- The exact startup flag and config keys.
- The endpoint kind (TCP, Unix socket, named pipe, or terminal only).
- A sample command and response, if any.
- Whether the channel works with stdin closed and no terminal.

**Pass criteria.** A non-interactive endpoint accepts at least one debugger
command from a host process while the container runs detached.

**Decision if it fails.** Leave `DebugNative` undeclared. `dosker debug` on
DOSBox-X reports the capability unavailable and names QEMU. `GdbStub` is never
declared in this slice.

## Spike 4 — Host directory

**Question.** Can a live host directory be exposed to the guest when DOSBox-X
boots a real DOS image?

**Why it matters.** `MOUNT` is a DOSBox-X shell command. It may not be
available inside a booted DOS image. If it is not, `HostDirLive` stays
undeclared for DOSBox-X and `--bind` is rejected there.

**Procedure.**

1. Create a host directory with `HOST.TXT` containing `ONE\r\n`.
2. Try `MOUNT` before `BOOT` in `[autoexec]`:
   ```ini
   [autoexec]
   MOUNT D /tmp/dosbox-x-spike/host
   IMGMOUNT C /tmp/dosbox-x-spike/agent.img -hdd
   BOOT -l c
   ```
3. Try `-c "MOUNT D /tmp/dosbox-x-spike/host"` before the config's boot.
4. After `HELLO`, run `TYPE D:\HOST.TXT`, then overwrite the host file with
   `TWO\r\n`, run `DIR D:\`, and run `TYPE D:\HOST.TXT` again.
5. Record whether D: exists, whether the guest sees the edit, and whether
   guest writes reach the host.

**Evidence to record.**

- The config or command line that created the mount.
- Whether D: exists after `BOOT`.
- The first and second `TYPE` output.
- Whether a guest write changed the host directory.

**Pass criteria.** After `BOOT`, the guest reads D: and sees a host edit after
re-reading the directory.

**Decision if it fails.** Leave `HostDirLive` undeclared; the adapter rejects
`--bind` with the capability message. Record the failure in the capability
table.

## Spike 5 — NAT and port publishing

**Question.** Can DOSBox-X provide user-mode NAT and host port forwarding
without root?

**Why it matters.** The design declares `Nat` only where it is proven. The
reference config documents a libslirp backend with `tcp_port_forwards`, which
is the candidate.

**Procedure.**

1. Boot `dosker-net.img` (a booter HTTP server) with the NE2000 slirp backend:
   ```ini
   [ne2000]
   ne2000  = true
   nicbase = 300
   nicirq  = 3
   backend = slirp

   [ethernet, slirp]
   tcp_port_forwards = 8080:80
   ```
   Use the same `[autoexec]` form that the BOOT spike proved, with the net
   image. The image is a booter, so no agent handshake is expected.
2. From the host, request the forwarded port:
   ```sh
   curl -sS --max-time 10 http://127.0.0.1:8080/
   ```
3. Record the response, the time to first response, and any libslirp message.
4. Repeat with the guest-side TCP server's real port and with a second
   forwarded port to confirm the mapping form.
5. Confirm no root privilege was needed and no host interface was opened.
6. If the booter image cannot use slirp, record whether the agent image with a
   packet driver can; the net fixture is the preferred evidence.

**Evidence to record.**

- The working `[ne2000]` and `[ethernet, slirp]` keys.
- The `tcp_port_forwards` form that worked.
- The host request, the guest response, and the timing.
- Whether root or a host interface was required.
- Any failure with a non-slirp backend.

**Pass criteria.** A host TCP request reaches the guest through a forwarded
port with no root privilege and no host interface capture.

**Decision if it fails.** Leave `Nat` undeclared. Publishing ports on DOSBox-X
is rejected by the existing capability gate, and the docs record why.

## Results

Fill this table before planning execution. The adapter's capability set is the
set of rows marked `proven`.

| Capability | Result | Evidence |
| --- | --- | --- |
| `boot` | | |
| `serial` | | |
| `stop` | | |
| `headless` | | |
| `host_dir_live` | | |
| `debug_native` | | |
| `gdbstub` | not tested; do not declare | |
| `screenshot` | not tested; do not declare | |
| `vnc` | not tested; do not declare | |
| `nat` | | |

Also record the adapter decisions the plan consumes:

| Decision | Value |
| --- | --- |
| Working boot command | |
| Working serial line | |
| Serial endpoint kind (TCP or pipe) | |
| Working headless flags | |
| Stop mechanism and exit code | |
| Debug endpoint kind, if any | |
| Host-directory mechanism, if any | |
| NAT backend and port-forward key, if any | |
| Pinned DOSBox-X version and checksum | |

## Reporting

Copy the completed Results and Decisions tables into this file and keep the
raw evidence beside it. The plan is written only after `BOOT` is `proven`;
`serial` and `headless` must be `proven` for the slice's acceptance bar.
