# DOSker — Engine Research

This asset records the evidence behind the capability model. Items marked **unverified** are settled by an M0 spike, not by assumption.

## Capability Matrix

| Capability | QEMU | DOSBox-X | Bochs |
| --- | --- | --- | --- |
| Boot a DOS disk image | Yes, full machine | Runs its own DOS. `BOOT` can boot an image, not the normal path | Yes, full machine |
| Long-running headless | Yes, `-display none` | Unclear. `-silent` runs AUTOEXEC and exits. Dummy video driver may work | Yes, `nogui` or `term` |
| Control channel | QMP, monitor, serial on Unix sockets | Limited `monitor sock`, debugger MCP channel | stdin and `-rc` script. No control socket |
| Serial to host | Yes | Yes, TCP and Unix socket | Yes, TCP |
| Native debugger | gdbstub built in | ncurses debugger, needs a terminal. MCP needs a debugger build | Internal debugger, stdin or `-rc`. gdbstub needs a custom build |
| Host directory drive | `vvfat`, marked beta and unsafe | `MOUNT`, live | `vvfat` with redo log |
| macOS arm64 | Homebrew only | Official arm64 zip | Homebrew only |
| User-mode NAT | Yes | Unverified | Unverified |

## QEMU

- Boot: QEMU passes a raw image to SeaBIOS as an IDE disk. A bare FAT filesystem with no boot code does not boot. The image needs a FreeDOS boot sector and, for a hard disk, a partition table. Sources: [Disk Images](https://www.qemu.org/docs/master/system/images.html), [Invocation](https://www.qemu.org/docs/master/system/invocation.html).
- Machine: `-M pc -cpu 486`. The `486` CPU model exists in [target/i386/cpu.c](https://github.com/qemu/qemu/blob/master/target/i386/cpu.c). The `isapc` machine is deprecated since QEMU 9.0 ([Deprecated features](https://www.qemu.org/docs/master/about/deprecated.html)).
- Headless: `-display none` disables video only. `-nographic` redirects serial to stdio. Verified: `-qmp unix:/tmp/q.sock,server=on,wait=off`, `-serial unix:/tmp/s.sock,server=on,wait=off`, and `-monitor unix:/tmp/m.sock,server=on,wait=off` with QEMU 11.1.1. Sources: [Invocation](https://www.qemu.org/docs/master/system/invocation.html), [QMP spec](https://www.qemu.org/docs/master/interop/qmp-spec.html).
- Debugging: `-s -S` opens a gdbstub on TCP 1234. For a Unix socket, use a chardev and `-gdb chardev:...`. For 16-bit real mode, use `set architecture i8086` and `x/10i $cs*16+$eip`. Sources: [GDB usage](https://www.qemu.org/docs/master/system/gdb.html). HMP `screendump` writes PPM, and QMP `screendump` supports PNG or PPM since 7.1. Sources: [Monitor](https://www.qemu.org/docs/master/system/monitor.html), [QMP reference](https://www.qemu.org/docs/master/interop/qemu-qmp-ref.html).
- Networking: `-nic user` or `-netdev user,id=n1`. Host forwarding uses `hostfwd=tcp:127.0.0.1:6001-:6000`. The ISA NE2000 model is `ne2k_isa`. Sources: [Invocation](https://www.qemu.org/docs/master/system/invocation.html), [ne2000-isa.h](https://github.com/qemu/qemu/blob/master/include/hw/net/ne2000-isa.h). DOS guests need a Crynwr packet driver ([Crynwr drivers](http://crynwr.com/drivers/)).
- Host directory: `-drive file=fat:rw:<dir>,format=raw` or `-hdb fat:<dir>`. The official limits say read/write is "beta stage", do not use non-ASCII names, do not use `-snapshot`, do not expect `loadvm`, and do not write to the host directory while the guest uses it. Sources: [Disk Images](https://www.qemu.org/docs/master/system/images.html). The issue tracker records directory traversal, assertion failures, and heap overflows ([QEMU issue 272](https://gitlab.com/qemu-project/qemu/-/issues/272)). Write corruption for large files existed until August 2024 ([pTOS issue 828](https://github.com/kelihlodversson/pTOS/issues/828)).
- macOS arm64: no official installer. Homebrew provides QEMU 11.1.3 with arm64 bottles ([Homebrew qemu](https://formulae.brew.sh/formula/qemu)). On Apple Silicon, HVF exposes Arm vCPU interfaces ([Apple documentation](https://developer.apple.com/documentation/hypervisor/apple-silicon)). The x86 HVF backend uses Intel VMX ([target/i386/hvf/vmx.h](https://github.com/qemu/qemu/blob/master/target/i386/hvf/vmx.h)). Therefore x86 DOS guests on macOS arm64 run under TCG. Performance is an estimate, not a measured DOS benchmark.

## DOSBox-X

- Command line: `-conf <file>`, `-c "<command>"`, `-set <section property=value>`, `-exit`, and `-time-limit <n>`. Source: [Command-Line Options](https://dosbox-x.com/wiki/DOSBox%E2%80%90X%E2%80%99s-Command%E2%80%90Line-Options).
- Control: `[dosbox] monitor sock` opens a TCP or Unix socket and supports some CONFIG.COM commands. `[dosbox] mcp_server` starts a debugger control client. It is line-oriented text, not JSON. Sources: [reference config](https://github.com/joncampbell123/dosbox-x/blob/master/dosbox-x.reference.full.conf), [README.debugger](https://github.com/joncampbell123/dosbox-x/blob/master/README.debugger). There is no documented stable lifecycle API.
- Headless: `-silent` runs without a window, executes AUTOEXEC.BAT, then exits. The SDL `videodriver` accepts `dummy`. There is no built-in VNC. Source: [Command-Line Options](https://dosbox-x.com/wiki/DOSBox%E2%80%90X%E2%80%99s-Command%E2%80%90Line-Options), [reference config](https://github.com/joncampbell123/dosbox-x/blob/master/dosbox-x.reference.full.conf).
- Serial: `serialX` accepts `nullmodem` with `server`, `port`, `telnet`, `transparent`, `usedtr`, `socks`, `pipe`, and `sock`. `sock:0` is TCP, `sock:1` is ENet, and `sock:2` is a named pipe or Unix socket. `pipe:<path>` makes DOSBox-X a server. Source: [reference config](https://github.com/joncampbell123/dosbox-x/blob/master/dosbox-x.reference.full.conf).
- Host directory: `MOUNT C /host/path` maps a host directory as drive C. It is write-through. DOSBox-X adds LFN support, controlled by `lfn`. Source: [DOSBox manual, MOUNT](https://www.dosbox.com/DOSBoxManual.html), [reference config](https://github.com/joncampbell123/dosbox-x/blob/master/dosbox-x.reference.full.conf).
- Debugger: ncurses-based. On Linux and macOS it must start from a terminal. `-break-start` breaks in at startup. The MCP control channel requires a debugger-enabled build. Source: [README.debugger](https://github.com/joncampbell123/dosbox-x/blob/master/README.debugger), [Command-Line Options](https://dosbox-x.com/wiki/DOSBox%E2%80%90X%E2%80%99s-Command%E2%80%90Line-Options). Whether official release builds include the debugger is **unverified**.
- macOS arm64: the 2026.10.01 release includes `dosbox-x-macosx-arm64-2026.10.01.zip`. Source: [Releases](https://github.com/joncampbell123/dosbox-x/releases).

## Bochs

- Display: `x`, `win32`, `carbon`, `sdl`, `sdl2`, `term`, `rfb`, `vncsrv`, `wx`, and `nogui`. `nogui`, `term`, `rfb`, and `vncsrv` are cross-platform. The Homebrew build uses `--with-nogui --with-sdl2 --with-term` and does not include `rfb` or `vncsrv`. Sources: [bochsrc, display_library](https://bochs.sourceforge.io/doc/docbook/user/bochsrc.html), [Homebrew bochs](https://formulae.brew.sh/formula/bochs).
- Serial: `com1` supports `null`, `term`, `file`, `raw`, `mouse`, `socket-client`, and `socket-server`. Socket modes use plain TCP. Source: [bochsrc, com1-4](https://bochs.sourceforge.io/doc/docbook/user/bochsrc.html).
- Internal debugger: build with `--enable-debugger`, start with `-dbg`, and feed commands on stdin. `-rc <file>` executes commands at startup. There is no documented remote socket. Sources: [internal debugger](https://bochs.sourceforge.io/doc/docbook/user/internal-debugger.html), [bochs/main.cc](https://github.com/bochs-emu/Bochs/blob/master/bochs/main.cc).
- gdbstub: build with `--enable-gdb-stub`. The stub is not active in standard packages. It waits on TCP 1234. `--enable-debugger` and `--enable-gdb-stub` are mutually exclusive. Source: [debugging with gdb](https://bochs.sourceforge.io/doc/docbook/user/debugging-with-gdb.html), [configure.ac](https://github.com/bochs-emu/Bochs/blob/master/bochs/configure.ac). Real-mode support is **unverified**.
- Host directory: `vvfat` as a disk mode or floppy source, with a volatile redo log and optional commit. Source: [bochsrc, ata devices](https://bochs.sourceforge.io/doc/docbook/user/bochsrc.html). Long-file-name behavior and DOS write safety are **unverified**.
- macOS arm64: Homebrew provides Bochs 3.1 with arm64 bottles. Source: [Homebrew bochs](https://formulae.brew.sh/formula/bochs).

## DOS-Side Automation

- `CTTY COM1` moves the system console to a serial device. The effect is system-wide. Programs that write directly to video memory ignore it. FreeDOS has a recorded input bug after `CTTY COM1`. Sources: [FreeDOS CTTY](http://help.fdos.org/en/hhstndrd/command/ctty.htm), [86Duino](https://www.86duino.com/?p=11966), [vmdos PR 137](https://github.com/jasonbrianhall/vmdos/pull/137). CTTY alone is not a reliable control channel.
- Existing tools: DSZ does X/Y/ZMODEM file transfer ([DSZ manual](https://www.arxvaldex.com/pb/files/manuals/modem/DSZBG.pdf)). C-Kermit provides serial sessions, file transfer, and scripting ([C-Kermit](https://www.kermitproject.org/ckermit.html)). Kermit server mode gives remote file operations, not a DOS command shell. FastLynx and LapLink are proprietary. No documented command channel was found.
- No small general-purpose DOS serial shell or agent was found. SSHDOS is a client, not a server ([SSHDOS](https://sshdos.sourceforge.net/)). NetFoss is a telnet FOSSIL driver for BBS software ([NetFoss](http://pcmicro.com/netfoss/)).
- Exit status: DOS programs return one byte with INT 21h/AH=4Ch. INT 21h/AH=4Dh reads the return code. COMMAND.COM stores the last external command code as ERRORLEVEL. The value is 0..255. Reading clears the stored word. `IF ERRORLEVEL n` is true when the value is greater than or equal to n. Sources: [Ralf Brown INT 21/AH=4Dh](https://www.ctyme.com/intr/rb-2976.htm), [FreeDOS IF](http://help.fdos.org/en/hhstndrd/batch/if.htm), [FreeDOS ERRORLEVEL](http://help.fdos.org/en/hhstndrd/batch/errorlvl.htm). A guest cannot return this value through the emulator process. The supervisor must read it from the DOS side.

## Non-Interactive FreeDOS Image

- `makebootfat` creates a bootable FAT filesystem, copies files, and can write an MBR. It accepts FreeDOS boot sectors with `-1 fat12com.bin -2 fat16com.bin -3 fat32lba.bin`. Source: [makebootfat(1)](https://manpages.debian.org/bookworm/makebootfat/makebootfat.1.en.html). Whether it writes to a regular image file as well as a block device is **unverified**.
- FreeDOS `SYS` writes the boot sector and copies `KERNEL.SYS` and `COMMAND.COM`. It supports `/FORCE:LBA`, `/FORCE:CHS`, `/NOBAKBS`, and `/B <btdrv>`. Source: [FreeDOS SYS](https://help.fdos.org/en/hhstndrd/base/sys.htm).
- Syslinux can load FreeDOS through `chain.c32`. `makebootfat -Y` supports syslinux 3.x.
- FAT type: FAT12 for floppy images, FAT16 for small hard disks, FAT32 only with a FAT32-capable kernel. Bochs requires the image size to equal `C*H*S*512` exactly ([bochsrc, ata devices](https://bochs.sourceforge.io/doc/docbook/user/bochsrc.html)). QEMU derives geometry from the image size.
- Rust: `fatfs` supports FAT12, FAT16, FAT32, LFN, and `format_volume` ([fatfs](https://docs.rs/fatfs/latest/fatfs/), [format_volume](https://docs.rs/fatfs/latest/fatfs/fn.format_volume.html)). It formats a volume, not a partitioned disk. It does not create an MBR or a boot sector. Wrap a partition region with `fscommon::StreamSlice`. `hadris-fat` is an alternative with explicit formatting ([hadris-fat](https://crates.io/crates/hadris-fat)).
- OCI: `oci-spec` provides Rust types for the image, runtime, and distribution specifications ([oci-spec](https://docs.rs/oci-spec/latest/oci_spec/)). `oci-client` is the registry client, formerly `oci-distribution` ([oci-client](https://docs.rs/oci-client/latest/oci_client/), [README](https://github.com/krustlet/oci-distribution)). Neither crate is a complete local image-layout store. Blob storage and `index.json` handling are our work.

## Container-Like Supervisors

- Podman has no central daemon. For each container it starts `conmon`, a small C monitor that double-forks, launches the OCI runtime, holds a socket for attach, copies stdio to a log, and records the exit code. The manager process can exit while `conmon` continues. Source: [conmon README](https://github.com/containers/conmon).
- `runq` is a Docker runtime that runs Docker images inside a KVM/QEMU VM ([runq](https://github.com/gotoz/runq)). Kata Containers runs each pod or container in a lightweight VM ([Kata architecture](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/README.md)). Both target Linux guests and use Linux kernel isolation. Neither models a DOS emulator.
- DOS-specific prior art: Docker images that wrap DOSBox or a DOS BBS, for example [an old DOS BBS in a Docker container](https://changelog.complete.org/archives/9836-an-old-dos-bbs-in-a-docker-container). These are ordinary containers running an emulator, not a Docker-like supervisor for emulated DOS machines.
- No project was found that treats QEMU, DOSBox-X, or Bochs as a container runtime with layered images and per-instance supervision. The search was web-based and could miss a private or small project.

## Windows Hosts

Windows x86_64 is a first-release host in the design. This research pass did not verify the Windows engine artifacts. These items are **unverified** and belong to the M1-W spike.

- QEMU publishes Windows builds. The exact source, version, and installer need confirmation.
- DOSBox-X publishes Windows builds. The serial and `BOOT` behavior should match the Unix build, but this is not proven.
- Bochs publishes Windows builds. The `nogui` display library and `com2` socket modes need confirmation.
- Windows has no Unix domain sockets in the Rust standard library. The control channel needs a named pipe or localhost TCP.
- QEMU on Windows can expose QMP, serial, monitor, gdbstub, and VNC over localhost TCP. The exact option forms need confirmation.
- A Windows Job Object with kill-on-close can replace the Unix process group and parent-death signal.
- Windows console behavior for `attach`, ANSI output, and Ctrl-C needs confirmation.

## Unverified Items

- QEMU TCG speed for a DOS guest on macOS arm64. No DOS benchmark was found.
- DOSBox-X process exit code and guest status propagation.
- Whether official DOSBox-X release builds include the debugger and MCP channel.
- Bochs gdbstub support for 16-bit real mode.
- Bochs `vvfat` long-file-name behavior and DOS write safety.
- FastLynx and LapLink command channels.
- Exact display libraries in Linux distribution Bochs packages.
- Whether a bare FAT "superfloppy" image boots as a QEMU hard disk without an MBR. Use a partitioned image or a standard floppy size.
- Whether `makebootfat` writes to a regular image file as well as a block device.
- User-mode NAT on DOSBox-X and Bochs.
- Windows engine artifacts, versions, and package sources.
- Windows named-pipe control and Windows console behavior.
- Windows QEMU QMP, serial, gdbstub, VNC, and NAT over localhost TCP.
- Windows Job Object cleanup after a supervisor crash.
