# Manual verification for the bundled OpenWatcom example

Use this checklist to confirm that the image is self-contained. Run the
commands from the repository root.

## Clean checkout

```sh
git clone https://github.com/i-zhirov/dosker.git dosker-clean
cd dosker-clean
```

The setup builds a 256 MiB FreeDOS disk, installs OpenWatcom and the
source on it, imports the disk as a base image, and builds the bundled
image. It needs QEMU, `curl`, `unzip`, `mtools`, and a FreeDOS 1.3
source image.

The setup downloads the OpenWatcom package to
`${XDG_CACHE_HOME:-$HOME/.cache}/dosker-openwatcom-dos`. The package is
about 122 MiB.

## Prepare

Start a new shell, or unset `DOSKER_ROOT`, `DOSKER_AGENT_EXE`,
`DOSKER_FREEDOS_SOURCE`, and `DOSKER_QEMU`. Then source the setup script:

```sh
. examples/openwatcom-dos-bundled/setup.sh
```

The script builds `openwatcom-bundled-base:1.3` from the raw disk and
then builds `openwatcom-bundled:latest`. It removes the temporary disk
and volume. The image has no toolchain volume and no source bind.

## Steps

- [ ] Confirm setup printed `built openwatcom-bundled:latest sha256:`.
- [ ] Run `dosker images` and confirm both the base and bundled images.
- [ ] Run `dosker volume ls` and confirm it is empty.
- [ ] Run the printed command:

  ```sh
  dosker run -d --name openwatcom-bundled-demo \
    --boot-timeout 120 openwatcom-bundled:latest
  ```
- [ ] Run `dosker ps` and confirm `openwatcom-bundled-demo` is `running`.
- [ ] Run:

  ```sh
  dosker exec openwatcom-bundled-demo 'C:\APP\BUILD.BAT'
  ```

  Confirm exit status 0. The DOS4GW banner appears. The guest creates
  `C:\APP\HELLO.EXE`.
- [ ] Run:

  ```sh
  dosker exec openwatcom-bundled-demo 'C:\APP\HELLO.EXE'
  ```

  Confirm `HELLO FROM BUNDLED OPENWATCOM`.
- [ ] Run:

  ```sh
  dosker exec openwatcom-bundled-demo 'DIR C:\WATCOM\BINW\WCC.EXE'
  ```

  Confirm the compiler file is present.
- [ ] Run `dosker volume ls` and confirm no volume is attached.
- [ ] Save the image:

  ```sh
  dosker save -o "$DOSKER_ROOT/openwatcom-bundled.tar" \
    openwatcom-bundled:latest
  ```

  Confirm the archive is under 500 MiB.
- [ ] Stop and remove the demo container:

  ```sh
  dosker stop openwatcom-bundled-demo
  dosker rm openwatcom-bundled-demo
  ```
- [ ] Remove both image tags:

  ```sh
  dosker rmi openwatcom-bundled:latest
  dosker rmi openwatcom-bundled-base:1.3
  ```

  Run `dosker images` and confirm it is empty.
- [ ] Load the archive:

  ```sh
  dosker load "$DOSKER_ROOT/openwatcom-bundled.tar"
  ```

  Run `dosker images` and confirm only `openwatcom-bundled:latest` is
  present. The base tag must not be restored.
- [ ] Run the loaded image without a volume or bind:

  ```sh
  dosker run -d --name openwatcom-bundled-loaded \
    --boot-timeout 120 openwatcom-bundled:latest
  ```
- [ ] Run:

  ```sh
  dosker exec openwatcom-bundled-loaded 'C:\APP\BUILD.BAT'
  dosker exec openwatcom-bundled-loaded 'C:\APP\HELLO.EXE'
  ```

  Confirm the build exits 0 and the executable prints
  `HELLO FROM BUNDLED OPENWATCOM`.
- [ ] Run `dosker volume ls` and confirm no volume is attached.
- [ ] Run `dosker images` and confirm the base tag is still absent.
- [ ] Stop and remove the loaded container:

  ```sh
  dosker stop openwatcom-bundled-loaded
  dosker rm openwatcom-bundled-loaded
  ```
- [ ] Run `dosker rmi openwatcom-bundled:latest` if you no longer need
  the image.
- [ ] Run `rm -rf "$DOSKER_ROOT"` and remove the clean checkout.

## Result

Verified on 2026-11-22 on macOS arm64 (Darwin).

- DOSker: 0.1.2 Homebrew binary.
- QEMU: 11.1.3.
- OpenWatcom: `2026-11-01-Build`.
- Base image: `openwatcom-bundled-base:1.3`,
  `sha256:e21f30ebd50de924b4bb4819e5c85a59040f01cda043b0e600cc7553e4252ab7`.
- Bundled image: `openwatcom-bundled:latest`,
  `sha256:6fd219cfccc96d3cefcf225793b0e5765aae80cdcc29acb455d3e1c9394f1665`.
- The image size is 268,435,456 bytes. It contains OpenWatcom and
  `C:\APP` source.
- The build exits 0 and creates `HELLO.OBJ`, `HELLO.MAP`, and
  `HELLO.EXE`.
- `HELLO.EXE` prints `HELLO FROM BUNDLED OPENWATCOM`.
- `C:\WATCOM\BINW\WCC.EXE` is present.
- `dosker volume ls` is empty before and after save/load.
- The save archive is 375 MiB, under the 500 MiB limit.
- Both image tags were removed, leaving `dosker images` empty.
- Loading the archive restored only `openwatcom-bundled:latest`.
- The loaded image built and ran without the base tag and without a
  volume or bind.
- The loaded container was left running after this check.
