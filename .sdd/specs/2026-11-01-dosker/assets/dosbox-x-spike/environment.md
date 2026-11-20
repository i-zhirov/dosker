# DOSBox-X Spike — Environment

- **Date**: 2026-11-20
- **Host**: macOS 15.5 (Darwin 24.5.0), arm64
- **DOSBox-X path**: `/opt/homebrew/bin/dosbox-x` →
  `../Cellar/dosbox-x/2026.10.01/bin/dosbox-x`
- **`dosbox-x --version` first line**:
  `DOSBox-X version 2026.10.01 SDL2, copyright 2011-2026 The DOSBox-X Team.`
- **Source**: Homebrew `dosbox-x 2026.10.01`
- **SHA-256 of the binary**:
  `6934abe66fe55d71487ded37e097416de65981250c902a9cca2bfd1284e98c96`
- **Debugger**: included (`Debugging with 16 bit debugger`, `DEBUG version
  1.25. Debugger.`). The interactive debugger UI is unavailable on macOS
  without a terminal; the MCP control channel works detached.
- **Reference config**:
  `/opt/homebrew/share/dosbox-x/dosbox-x.reference.full.conf`
- **Fixture `agent/fixtures/dosker-agent.img`**:
  `17a621ab7e3658091104f367e5b4bb23f81b608bbd8760c4be6b5cf46c1131df`
- **Fixture `agent/fixtures/dosker-net.img`**:
  `c9621effdc3746ea3aa630988057d8c397f25418e08ce0ce4ff9090fd077dfb3`
- **Host tooling**: `mcopy`, `mdir`, `mtype` (mtools) present; `curl` present;
  `qemu-system-i386` present; `socat` missing.
- **Scratch directory**: `$HOME/dosbox-x-spike` (copies of both fixtures).
  DOSBox-X refused to mount a copy under `/private/tmp` (`Image file is
  read-only!`, then `Cannot create drive from file.`); it mounted the same
  copy from `$HOME` without complaint. The data root on macOS is
  `~/Library/Application Support/dosker`, so this does not affect the adapter.

## Fixture preparation

```sh
cp agent/fixtures/dosker-agent.img "$HOME/dosbox-x-spike/agent.img"
printf 'CONTROL_PORT=2\r\nBAUD=115200\r\n' > "$HOME/dosbox-x-spike/CONFIG.INI"
mcopy -o -i "$HOME/dosbox-x-spike/agent.img@@1M" \
  "$HOME/dosbox-x-spike/CONFIG.INI" ::/DOSKER/CONFIG.INI
```

The copy contained exactly `CONTROL_PORT=2\r\nBAUD=115200\r\n`.
