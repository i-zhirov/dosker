# Bundled OpenWatcom FreeDOS Example Implementation Plan

> Execute this plan with the `executing-plans` skill. Each task is a
> checkbox; mark it done when its changes are implemented and its
> Verify command passes.

- **Date**: 2026-11-22
- **Status**: In Progress
- **Design**: This plan's Design section (self-contained imported-disk example)
- **Depends on**: —
- **Manual prerequisites**: QEMU, `dosker` 0.1.2+, `curl`, `unzip`, `mtools`, a FreeDOS 1.3 source image, network access, and about 2 GiB free disk.
- **Goal**: Add a separate example whose imported raw disk and image contain the OpenWatcom compiler, so `dosker save` and `dosker load` work without a named toolchain volume.

## Design

The current `examples/openwatcom-dos` example is volume-based. Its
`Doskerfile` only declares `VOLUME E:`, and `setup.sh` installs the
117.8 MiB OpenWatcom tree into the host-side `watcom-tools` volume. The
image alone does not contain the compiler.

`COPY` and `RUN` cannot fix that. They change build tree layers only.
`create` copies the image's annotated raw-disk blob to the container
disk, and the raw disk remains the one from the base. The runtime does
not project tree layers onto that disk. `docs/builds.md` and
`crates/dosker-cli/src/commands/create.rs` show this split.

Use the existing normal DOS disk import path instead:

```text
256 MiB bootable FreeDOS raw disk
  C:\WATCOM        full OpenWatcom DOS toolchain
  C:\APP           HELLO.C, BUILD.BAT
  C:\CONFIG.SYS    base configuration
  C:\AUTOEXEC.BAT  plain, returning batch
  no C:\DOSKER
        |
        | dosker image import
        v
openwatcom-bundled-base:1.3
        |
        | FROM + config-only Doskerfile
        v
openwatcom-bundled:latest  (raw disk contains the compiler)
```

The import path preserves the raw disk, injects the DOSker agent into
`C:\DOSKER`, and keeps `dosker exec` available. It is not the booter
path, which has no agent. The new image has no `VOLUME` and no host
source bind. The example builds from `C:\APP`, and the compiler is
inside the image.

This needs no runtime or base-builder changes. It uses the existing
import, build, run, and save/load paths.

### Why a larger disk is required

The current FreeDOS base C: is 32 MiB with 512-byte clusters. The full
OpenWatcom subtree is 123,529,176 bytes in 1,610 files, so it does not
fit.

The new base uses a 256 MiB disk with 8 KiB FAT16 clusters. That is
32,640 clusters, within FAT16's limit. The disk has 1,024 cylinders at
16 heads and 32 sectors, which the QEMU BIOS and Bochs geometry handle.

### Disk construction

Build the disk with the existing volume image format, then make it
bootable:

1. `dosker volume create --size 256M openwatcom-bundled-disk` creates an
   MBR-partitioned FAT16 raw disk at the DOSker data root.
2. Set the MBR active flag for partition 1. The volume image starts with
   `0x00` at MBR offset 446.
3. Boot a FreeDOS 1.3 floppy with the disk as IDE C:. Run
   `A:\FDISK.EXE /MBR` and `A:\SYS.COM C:`. Copy `COMMAND.COM`,
   `CONFIG.SYS`, and a plain `AUTOEXEC.BAT`.
4. Copy `BINW`, `H`, `RH`, `LIB286`, and `LIB386` to `::/WATCOM` with
   `mtools`.
5. Copy `src/HELLO.C` and `src/BUILD.BAT` to `::/APP`.
6. Boot the disk alone and require `OPENWATCOM BUNDLED READY` on COM1.
7. Import the disk with `dosker image import`. The importer injects the
   agent and wraps `AUTOEXEC.BAT`.

The volume is a temporary build device. After the import, remove the
volume and the raw disk file. The image store holds the raw disk.

### Image and build loop

The new example is self-contained. It does not need `--bind` or `-v`:

```sh
dosker run -d --name openwatcom-bundled-demo \
  --boot-timeout 120 openwatcom-bundled:latest

dosker exec openwatcom-bundled-demo C:\APP\BUILD.BAT
dosker exec openwatcom-bundled-demo C:\APP\HELLO.EXE
```

`C:\APP\BUILD.BAT` sets `WATCOM=C:\WATCOM` and uses the DOS-hosted
`WCC`/`WLINK` with `C:\WATCOM\BINW\DOS4GW.EXE`.

## Constraints

- New example directory: `examples/openwatcom-dos-bundled`.
- New image tag: `openwatcom-bundled:latest`.
- New base tag: `openwatcom-bundled-base:1.3`.
- Disk size: 268,435,456 bytes.
- Disk layout: MBR partition 1, FAT16, partition start LBA 2,048, 522,240 partition sectors.
- Disk geometry: 1,024 cylinders, 16 heads, 32 sectors.
- FAT16 cluster size: 8,192 bytes.
- MBR active flag: `0x80` at offset 446.
- FreeDOS source: FreeDOS 1.3 Floppy Edition.
- OpenWatcom package: `openwatcom-2_0-c-dos.exe` from `2026-11-01-Build`.
- OpenWatcom SHA-256: `21acc705be9fb04057c8f4ccddcfec659e3ea2aa3a6be463473c222b87a530d1`.
- OpenWatcom package size: 127,878,470 bytes.
- Toolchain directories: `BINW`, `H`, `RH`, `LIB286`, `LIB386`; 123,529,176 bytes.
- Toolchain destination: `C:\WATCOM`.
- Source destination: `C:\APP\HELLO.C` and `C:\APP\BUILD.BAT`.
- Build outputs: `C:\APP\HELLO.OBJ`, `C:\APP\HELLO.EXE`, `C:\APP\HELLO.MAP`.
- `Doskerfile` base: `FROM openwatcom-bundled-base:1.3`.
- `Doskerfile` machine: `MACHINE memory=16M`.
- `Doskerfile` engine: `ENGINE qemu`.
- `Doskerfile` requirements: `REQUIRES boot,serial,headless`.
- `Doskerfile` command: `CMD ECHO OPENWATCOM BUNDLED READY`.
- No `VOLUME` instruction and no image-provided data disk.
- No committed `Doskerfile.lock` for the local imported base; the setup builds with `--update` in a temporary context.
- Expected `save` archive size: about 380 MiB; do not commit the archive.
- Do not commit the OpenWatcom package, the 256 MiB disk, or the imported image.

## Tasks

- [x] **Task 1: Build and boot the 256 MiB FreeDOS disk**
    - **Implements:** Design §Disk construction; §Why a larger disk is required
    - **Create `examples/openwatcom-dos-bundled/build-disk.sh`**
        - `build-disk.sh <output.img>` — builds the bootable disk. Use the volume image path as the raw disk and a temporary `DOSKER_ROOT`.
        - Constants: `disk_size=268435456`, `volume_name=openwatcom-bundled-disk`, `partition_offset=1048576`, `active_flag_offset=446`, `boot_timeout=120`.
        - Set MBR active flag with `printf '\200' | dd of="$disk" bs=1 seek=446 conv=notrunc`.
        - Start QEMU with the FreeDOS source as floppy `index=0` and the disk as IDE `index=0`. The guest batch runs `FDISK /MBR`, `SYS C:`, and copies `COMMAND.COM`, `CONFIG.SYS`, and plain `AUTOEXEC.BAT`.
        - Emit `OPENWATCOM DISK READY` on COM1 from the guest batch.
        - Clear the build batch after success by overwriting `C:\AUTOEXEC.BAT` with a plain returning file.
        - Boot the disk alone and require `OPENWATCOM DISK READY`.
    - **Verify:**
      - `sh -n examples/openwatcom-dos-bundled/build-disk.sh`
      - `MTOOLS_SKIP_CHECK=1 mdir -i "$OUTPUT@@1048576" ::/KERNEL.SYS ::/COMMAND.COM ::/CONFIG.SYS ::/AUTOEXEC.BAT`
      - A second QEMU boot with only the disk prints `OPENWATCOM DISK READY`.

- [x] **Task 2: Install OpenWatcom and the example source on the disk**
    - **Implements:** Design §Disk construction steps 4-5
    - **Create `examples/openwatcom-dos-bundled/src/HELLO.C`**
        - `main` prints `HELLO FROM BUNDLED OPENWATCOM`.
    - **Create `examples/openwatcom-dos-bundled/src/BUILD.BAT`**
        - `C:` and `CD \APP`.
        - `SET WATCOM=C:\WATCOM`
        - `SET DOS4GPATH=%WATCOM%\BINW`
        - `SET PATH=%WATCOM%\BINW;%PATH%`
        - `SET INCLUDE=%WATCOM%\H;%WATCOM%\H\SYS`
        - `SET LIB=%WATCOM%\LIB286;%WATCOM%\LIB286\DOS;%WATCOM%\LIB386;%WATCOM%\LIB386\DOS`
        - `SET TMP=C:\TMP` and `SET TEMP=C:\TMP`
        - Compile `HELLO.C` with `WCC -0 -bt=dos -ml -zq -wx -we -i=%WATCOM%\H -fo=HELLO.OBJ`.
        - Link with `WLINK SYSTEM DOS NAME HELLO.EXE FILE HELLO.OBJ OPTION STACK=8192 OPTION MAP=HELLO.MAP`.
        - Print `BUILD OK` on success and `BUILD FAILED` on failure.
    - **Modify `examples/openwatcom-dos-bundled/build-disk.sh`**
        - Copy `BINW`, `H`, `RH`, `LIB286`, and `LIB386` from the toolchain directory to `::/WATCOM`.
        - Copy `HELLO.C` and `BUILD.BAT` from the source directory to `::/APP`.
        - Fail if `::/WATCOM/BINW/WCC.EXE`, `::/WATCOM/BINW/WLINK.EXE`, `::/WATCOM/BINW/DOS4GW.EXE`, or `::/APP/HELLO.C` is missing.
    - **Verify:**
      - `MTOOLS_SKIP_CHECK=1 mdir -i "$OUTPUT@@1048576" ::/WATCOM/BINW/WCC.EXE ::/WATCOM/BINW/WLINK.EXE ::/WATCOM/BINW/DOS4GW.EXE`
      - `MTOOLS_SKIP_CHECK=1 mdir -i "$OUTPUT@@1048576" ::/APP`
      - `MTOOLS_SKIP_CHECK=1 mtype -i "$OUTPUT@@1048576" ::/APP/BUILD.BAT` contains `WATCOM=C:\WATCOM`.
      - Boot a temporary copy of the disk with an `AUTOEXEC.BAT` that runs `C:\APP\BUILD.BAT`. Confirm the guest screen shows `BUILD OK` and `::/APP` contains `HELLO.OBJ`, `HELLO.MAP`, and `HELLO.EXE`.

- [x] **Task 3: Import the base disk and build the bundled image**
    - **Implements:** Design §Image and build loop
    - **Create `examples/openwatcom-dos-bundled/setup.sh`**
        - Source-only entry point like `examples/openwatcom-dos/setup.sh`.
        - Download and SHA-256 verify the OpenWatcom package; extract `BINW`, `H`, `RH`, `LIB286`, and `LIB386`.
        - Obtain the FreeDOS source and DOS agent using the existing `examples/openwatcom-dos/setup.sh` pattern.
        - Call `build-disk.sh "$disk" "$ow_dir" "$repo_root/examples/openwatcom-dos-bundled/src"`.
        - Run `dosker image import "$disk" openwatcom-bundled-base:1.3`.
        - Remove the temporary volume and disk after a successful import.
        - Build `dosker build --update -t openwatcom-bundled:latest "$build_context"`.
        - Print the `dosker run` and `dosker exec` commands.
    - **Create `examples/openwatcom-dos-bundled/Doskerfile`**
        - `FROM openwatcom-bundled-base:1.3`
        - `MACHINE memory=16M`
        - `ENGINE qemu`
        - `REQUIRES boot,serial,headless`
        - `CMD ECHO OPENWATCOM BUNDLED READY`
    - **Create `examples/openwatcom-dos-bundled/.gitignore`**
        - Ignore `*.img`, `*.tar`, `src/*.OBJ`, `src/*.obj`, `src/*.EXE`, `src/*.exe`, `src/*.MAP`, `src/*.map`, `src/*.ERR`, `src/*.err`.
    - **Verify:**
      - `dosker image import` prints the imported reference.
      - `dosker images` lists `openwatcom-bundled-base:1.3` and `openwatcom-bundled:latest`.
      - `dosker inspect` of a created container shows the volume list empty and a raw disk from the imported base.
      - `dosker volume ls` does not list a toolchain volume after cleanup.

- [x] **Task 4: Build and run from the bundled image**
    - **Implements:** Design §Image and build loop
    - **Modify `examples/openwatcom-dos-bundled/setup.sh`**
        - Print the exact run command:
          `dosker run -d --name openwatcom-bundled-demo --boot-timeout 120 openwatcom-bundled:latest`
        - Print:
          `dosker exec openwatcom-bundled-demo C:\APP\BUILD.BAT`
          `dosker exec openwatcom-bundled-demo C:\APP\HELLO.EXE`
    - **Verify:**
      - `dosker run -d --name openwatcom-bundled-demo --boot-timeout 120 openwatcom-bundled:latest`
      - `dosker ps` shows `openwatcom-bundled-demo` running.
      - `dosker exec openwatcom-bundled-demo 'C:\APP\BUILD.BAT'` exits 0 and creates `C:\APP\HELLO.EXE`.
      - `dosker exec openwatcom-bundled-demo 'C:\APP\HELLO.EXE'` prints `HELLO FROM BUNDLED OPENWATCOM`.
      - `dosker exec openwatcom-bundled-demo 'DIR C:\WATCOM\BINW\WCC.EXE'` succeeds (the compiler is in the image).
      - `dosker volume ls` shows no volume attached.

- [x] **Task 5: Prove save/load is self-contained**
    - **Implements:** Design §Image and build loop; Goal
    - **Create `examples/openwatcom-dos-bundled/VERIFY.md`**
        - Clean-checkout preparation, run, build, run, save/load, base-tag removal, and cleanup steps.
        - Record the archive size and the container/volume state.
    - **Verify:**
      - `dosker save -o "$DOSKER_ROOT/openwatcom-bundled.tar" openwatcom-bundled:latest`.
      - `dosker rmi openwatcom-bundled:latest` and `dosker rmi openwatcom-bundled-base:1.3`.
      - `dosker load "$DOSKER_ROOT/openwatcom-bundled.tar"`.
      - `dosker run -d --name openwatcom-bundled-loaded --boot-timeout 120 openwatcom-bundled:latest`.
      - `dosker exec openwatcom-bundled-loaded 'C:\APP\BUILD.BAT'` and `'C:\APP\HELLO.EXE'` pass with no volume and no base tag.
      - The save archive is under 500 MiB.

- [ ] **Task 6: Document the bundled example and keep the volume example separate**
    - **Implements:** Design §Design; §Constraints
    - **Modify `README.md`**
        - Add a bundled-example section after the existing OpenWatcom volume example.
        - State that the image contains OpenWatcom, has no `watcom-tools` volume, and uses `C:\APP` source.
        - State the 256 MiB disk, the save/load flow, the expected archive size, and the need to rebuild the image to change the source.
        - State the OpenWatcom redistribution/licensing requirement.
        - Add documentation links to the new example and `VERIFY.md`.
    - **Verify:**
      - `rg -n "openwatcom-dos-bundled|openwatcom-bundled" README.md examples/openwatcom-dos-bundled/VERIFY.md`
      - Every link in the new README section resolves.

## Risks

- **Disk bootstrap:** The volume-created MBR has no boot code and its active
  flag is clear. Task 1 sets the flag and uses FreeDOS `FDISK /MBR` and
  `SYS`. If the disk does not boot, stop and revise the disk-building
  path; do not substitute a booter image, because it has no agent.
- **FAT16 geometry:** 8 KiB clusters and 1,024 cylinders are valid but
  not covered by the current 32 MiB base tests. Task 1 and Task 2 must
  boot and read the disk on both macOS and Linux.
- **Agent injection:** The imported disk must keep a plain returning
  `AUTOEXEC.BAT` and no `C:\DOSKER` tree. Task 3 verifies that the agent
  is injected and `dosker exec` works.
- **Archive size:** The raw disk and the imported tree duplicate the
  toolchain. The save archive is expected near 380 MiB. Document it and
  do not commit it.
- **Licensing:** Publishing a saved image redistributes OpenWatcom.
  Confirm the OpenWatcom redistribution terms before publishing an
  artifact.
- **No runtime changes:** If the normal disk import path cannot preserve
  the large disk or inject the agent, stop and re-plan. Do not add an
  image-provided E: volume or a `COPY` runtime projection in this plan.

## Acceptance

- A fresh setup builds a 256 MiB disk with `C:\WATCOM` and `C:\APP`.
- `openwatcom-bundled:latest` runs without a named volume or bind.
- `C:\APP\BUILD.BAT` links `C:\APP\HELLO.EXE`, and the executable prints
  `HELLO FROM BUNDLED OPENWATCOM`.
- After removing the base tag, `dosker save` and `dosker load` still run
  and build the example.
- The existing `examples/openwatcom-dos` volume example is unchanged.
- No runtime or base-builder code changes are needed.

## Deviations

- Task 1 used `build-disk.sh <output.img>`. Task 2 added the required
  toolchain and source arguments and the copy step.
- Task 2 was additionally verified by booting a temporary disk copy and
  running `BUILD.BAT`; the guest screen showed `BUILD OK` and produced
  `HELLO.OBJ`, `HELLO.MAP`, and `HELLO.EXE`.
- Task 4 prints and uses quoted DOS paths because POSIX shells consume
  backslashes. It checks the compiler with `DIR` instead of `TYPE` to
  avoid dumping the executable to the terminal.
