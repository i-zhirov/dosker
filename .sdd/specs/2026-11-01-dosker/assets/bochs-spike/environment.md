# Bochs Spike — Environment

- **Date**: 2026-11-20
- **Host**: macOS 15.5 (Darwin 24.5.0), arm64
- **Reference Bochs path**: `/opt/homebrew/bin/bochs` →
  `../Cellar/bochs/3.1/bin/bochs`
- **Reference version**: Bochs 3.1 (Homebrew). The banner reads
  `Bochs x86 Emulator 3.1` / `Built from GitHub snapshot on August 23, 2026`.
- **Reference build flags**: Homebrew builds with `--enable-debugger`,
  `--enable-debugger-gui`, `--with-nogui`, `--with-sdl2`, and `--with-term`.
- **Reference SHA-256**:
  `8c1d38ac5fe8dba981fc9c7321ec46723324fdaa6cb39f01e86d4afab08c447c`
- **Reference share directory**: `/opt/homebrew/share/bochs` (the compile-time
  path is `/opt/homebrew/Cellar/bochs/3.1/share/bochs`).
- **gdbstub build path**: `$HOME/bochs-gdbstub/bin/bochs`
- **gdbstub build source**: Bochs `REL_3_1_FINAL` from the upstream GitHub
  archive.
- **gdbstub build flags**: `--enable-gdb-stub --with-nogui
  --enable-cpu-level=6 --prefix=$HOME/bochs-gdbstub`.
- **gdbstub SHA-256**:
  `56aa939652578a81bb2e0f915fc48b80643053e24a5e046ba7af86c65643ae95`
- **gdbstub share directory**: `$HOME/bochs-gdbstub/share/bochs`.
- **Internal debugger**: yes in the Homebrew build (`-dbg`, prompt `bochs:1>`).
- **gdbstub**: yes in the local gdbstub build; not in the Homebrew build
  (`--enable-debugger` and `--enable-gdb-stub` are mutually exclusive).
- **Fixture `agent/fixtures/dosker-agent.img`**:
  `17a621ab7e3658091104f367e5b4bb23f81b608bbd8760c4be6b5cf46c1131df`
- **Fixture copy after the CONFIG.INI write**:
  `029966b260bd8667b2890a5356f7d3f7a24349a2847ddcd81c5c7bb8444f365f`
- **Fixture `agent/fixtures/dosker-net.img`**:
  `c9621effdc3746ea3aa630988057d8c397f25418e08ce0ce4ff9090fd077dfb3`
- **Host tooling**: `gdb` at `$HOME/.nix-profile/bin/gdb`, `lldb` at
  `/usr/bin/lldb`, `mcopy`/`mdir`/`mtype` (mtools), and `curl` present;
  `socat` missing.
- **Scratch directory**: `$HOME/bochs-spike`.

## Fixture preparation

```sh
cp agent/fixtures/dosker-agent.img "$HOME/bochs-spike/agent.img"
printf 'CONTROL_PORT=2\r\nBAUD=115200\r\n' > "$HOME/bochs-spike/CONFIG.INI"
mcopy -o -i "$HOME/bochs-spike/agent.img@@1M" \
  "$HOME/bochs-spike/CONFIG.INI" ::/DOSKER/CONFIG.INI
```

The copy contained exactly `CONTROL_PORT=2\r\nBAUD=115200\r\n`.

## Disk geometry

The 32 MiB fixture is `128` cylinders, `16` heads, and `32` sectors per track
(`128*16*32*512 = 33 554 432`). Bochs accepted `cylinders=128, heads=16,
spt=32` and reported `using specified geometry: CHS=128/16/32`.

## Notes

- The CPU model is `i486dx4`; Bochs rejects `486` as a model name.
- The serial socket server blocks startup until a client connects. A debugger
  spike that does not need the serial link uses `com2: enabled=1, mode=null`.
- Bochs leaves a lock on a disk image when the process is killed. A fresh copy
  per run avoided it; the adapter uses a private disk per container, so a stale
  lock is not expected.
- The gdbstub listens on all interfaces (`*:1235` in `lsof`), not only
  loopback.
