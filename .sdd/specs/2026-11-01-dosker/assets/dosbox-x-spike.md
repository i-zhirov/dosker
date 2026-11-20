# DOSBox-X Spike

- **Date**: 2026-11-19
- **Status**: Complete
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
| `boot` | proven | `IMGMOUNT C <image> -t hdd` then `BOOT C:` boots the copy; agent `HELLO` arrives. `assets/dosbox-x-spike/boot/` |
| `serial` | proven | `serial2 = nullmodem port:<port> sock:0 transparent:1`; `HELLO` in ~10.9 s with protocol 1, control port 2, `max_payload` 4096. `assets/dosbox-x-spike/boot/serial-transcript.txt` |
| `stop` | proven | Agent `SHUTDOWN` is acknowledged but does not exit DOSBox-X; `SIGTERM` exits with code 0 in ~2.9 s. `SIGINT` is ignored. `assets/dosbox-x-spike/boot/` |
| `headless` | proven | `[sdl] videodriver = dummy` (`-set "sdl videodriver=dummy"`): no window, process alive past 60 s, `EXEC` answers. `assets/dosbox-x-spike/headless/` |
| `host_dir_live` | unproven | `MOUNT D <host>` before `BOOT` converts the directory to a FAT snapshot. A host edit after mount is not visible (`TYPE` still returns `ONE`), guest writes do not reach the host, and `-nocachedir` does not change this. `assets/dosbox-x-spike/host-directory/` |
| `debug_native` | proven | `[dosbox] mcp_server = <port>`; DOSBox-X connects out to `127.0.0.1:<port>` and answers `REQ <id> EXEC CPU` with a register dump while detached (stdin null, no controlling terminal). The interactive `BREAK` UI is unavailable on macOS without a terminal; the command channel works. `assets/dosbox-x-spike/debugger/mcp-session.txt` |
| `gdbstub` | not tested; do not declare | — |
| `screenshot` | not tested; do not declare | — |
| `vnc` | not tested; do not declare | — |
| `nat` | unproven | `[ne2000]` + `[ethernet, slirp] tcp_port_forwards = 8080:80` reaches the guest with no root, but the forward binds `INADDR_ANY` (`*:8080`), and this build has no loopback-only option. Declaring it would bypass the design's `127.0.0.1` default for published ports. `assets/dosbox-x-spike/nat/` |

Also record the adapter decisions the plan consumes:

| Decision | Value |
| --- | --- |
| Working boot command | `IMGMOUNT C "<image>" -t hdd` then `BOOT C:`. The image path must be double-quoted: the macOS data root contains a space (`Application Support`), and an unquoted path with a space is rejected. The candidates in the procedure were wrong: `-hdd` is rejected and `BOOT -l c` is not a valid form (`-L` is ignored). `BOOT <image>` also fails for a copy under `/private/tmp`; the mount form works from the data root. |
| Working serial line | `serial2 = nullmodem port:<port> sock:0 transparent:1`. `port:` is the TCP **server**; `server:` is the TCP client and fails with `Connection failed.` without a listener. Without `transparent:1`, DOSBox-X injects `0xFF` modem-status bytes that corrupt the agent frame stream. |
| Serial endpoint kind (TCP or pipe) | TCP. The host connects to `127.0.0.1:<port>`; DOSBox-X listens. |
| Working headless flags | `-set "sdl videodriver=dummy"` (equivalent to `[sdl] videodriver = dummy`). `-silent` also runs without a window and stays alive while `BOOT` blocks, but it also switches audio to `dummy` and is documented to exit after `AUTOEXEC.BAT`; it is not the recorded form. |
| Stop mechanism and exit code | Send the agent `SHUTDOWN`, wait for `SHUTDOWN_ACK`, then `SIGTERM` the DOSBox-X process. Exit code 0, about 2.9 s after the signal. |
| Debug endpoint kind, if any | TCP MCP control channel, with the direction reversed: DOSBox-X is the TCP client and the host listens. Config key `[dosbox] mcp_server = <port>`. Requests are line-oriented: `REQ <id> PING`, `REQ <id> BREAK`, `REQ <id> EXEC <debugger command>`; responses are `BEGIN <id> OK` … `END <id>`. Sample: `REQ 2 EXEC CPU` returns the register dump. |
| Host-directory mechanism, if any | None live. `MOUNT D <host>` before `BOOT` creates a FAT snapshot (`Converting drive D: to FAT...`); `-nocachedir` does not make it live. |
| NAT backend and port-forward key, if any | Backend `slirp`; keys `[ne2000] ne2000 = true`, `nicbase = 300`, `nicirq = 9` (the fixture packet driver uses IRQ 9), `backend = slirp`; `[ethernet, slirp] tcp_port_forwards = host:guest ...`. Works with no root, but the listener binds all host interfaces. |
| Pinned DOSBox-X version and checksum | `2026.10.01` SDL2; `6934abe66fe55d71487ded37e097416de65981250c902a9cca2bfd1284e98c96`. |

## Evidence

Raw evidence is under `assets/dosbox-x-spike/`:

- `environment.md` — host, binary, fixture, and tooling record.
- `boot/` — working config, serial transcript, DOS CON log, `IMGMOUNT`/`BOOT` help.
- `headless/` — display-driver log lines for the dummy driver and `-silent`.
- `debugger/` — MCP session transcript and the DOSBox-X log.
- `host-directory/` — config, serial transcript, DOS CON log, `MOUNT` help.
- `nat/` — config, slirp log lines.

## Reporting

Copy the completed Results and Decisions tables into this file and keep the
raw evidence beside it. The plan is written only after `BOOT` is `proven`;
`serial` and `headless` must be `proven` for the slice's acceptance bar.
