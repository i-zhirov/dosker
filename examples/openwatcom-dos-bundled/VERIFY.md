# Manual verification for the bundled OpenWatcom example

Use this checklist to confirm that the image is self-contained. Run the
commands from the repository root.

## Clean checkout

```sh
git clone https://github.com/i-zhirov/dosker.git dosker-clean
cd dosker-clean
```

The setup builds a 256 MiB FreeDOS base disk, imports it, and builds the
bundled image from a `Doskerfile` that copies OpenWatcom and the source
and runs `BUILD.BAT` during the build. It uses the workspace binary from
`target/debug` or `target/release`, builds it if necessary, and falls back
to an installed `dosker`. It needs QEMU, `curl`, `unzip`, `mtools`, a
FreeDOS 1.3 source image, and Rust if no binary is built.

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
then builds `openwatcom-bundled:latest` from the `COPY`/`RUN`
`Doskerfile`. The `RUN` step executes `C:\APP\BUILD.BAT` under QEMU. The
script removes the temporary base disk and the copied toolchain/source
build context. The image tree contains OpenWatcom, the source, and the
build outputs; there is no toolchain volume and no source bind.

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
  dosker exec openwatcom-bundled-demo 'C:\APP\HELLO.EXE'
  ```

  Confirm `HELLO FROM BUNDLED OPENWATCOM`. The executable was built
  during `dosker build`, not at run time.
- [ ] Run:

  ```sh
  dosker exec openwatcom-bundled-demo 'DIR C:\APP'
  ```

  Confirm `HELLO.EXE`, `HELLO.OBJ`, `HELLO.MAP`, `HELLO.C`, and
  `BUILD.BAT` are present.
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
  dosker exec openwatcom-bundled-loaded 'C:\APP\HELLO.EXE'
  ```

  Confirm the executable prints `HELLO FROM BUNDLED OPENWATCOM` without
  rebuilding.
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

Verified on 2026-11-23 on macOS arm64 (Darwin).

- DOSker: workspace `target/debug/dosker` 0.1.2.
- QEMU: 11.1.3.
- OpenWatcom: `2026-11-01-Build`.
- Base image: `openwatcom-bundled-base:1.3`,
  `sha256:c019204e576b5b36dcfa869423cdc680544072667e1417cd565cadc974988a61`.
- Bundled image: `openwatcom-bundled:latest`,
  `sha256:93e8b3043ca91e6cbc205b522268be3c4bd71981d76b6a8abe7bb77d0d525527`.
- The image size is 268,435,456 bytes. The image tree contains
  OpenWatcom, `C:\APP` source, and the build outputs.
- The `RUN C:\APP\BUILD.BAT` step during `dosker build` exits 0 and
  creates `HELLO.OBJ`, `HELLO.MAP`, and `HELLO.EXE`.
- `HELLO.EXE` prints `HELLO FROM BUNDLED OPENWATCOM`.
- `C:\WATCOM\BINW\WCC.EXE` is present.
- `dosker volume ls` is empty.
- The save/load steps above were verified for the earlier imported
  variant. The current conversion was verified for setup, `COPY`/`RUN`,
  and the QEMU boot.
