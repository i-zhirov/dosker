# Manual verification for hello-dos-bundled

Use this checklist to confirm that the self-contained example works from
a clean checkout. Run the commands from the repository root.

## Clean checkout

```sh
git clone https://github.com/i-zhirov/dosker.git dosker-clean
cd dosker-clean
```

The checkout has no `target`, no `agent/build`, and no FreeDOS source.
The setup script prepares all three through the shared hello-dos setup.

## Prepare

Start a new shell, or unset `DOSKER_ROOT`, `DOSKER_AGENT_EXE`,
`DOSKER_FREEDOS_SOURCE`, and `DOSKER_QEMU`. Then source the setup script:

```sh
. examples/hello-dos-bundled/setup.sh
```

The script prepares the DOS agent, FreeDOS source, QEMU, data root, and
workspace `dosker`. It builds `hello-bundled:latest` from the
`COPY`/`RUN` Doskerfile and creates no volume. It caches the agent and
FreeDOS source in `${XDG_CACHE_HOME:-$HOME/.cache}/dosker-hello-dos`.

## Steps

- [ ] Confirm setup printed `built hello-bundled:latest sha256:`.
- [ ] Run `dosker images` and confirm `freedos:1.3` and
  `hello-bundled:latest`.
- [ ] Run `dosker volume ls` and confirm it is empty.
- [ ] Start the container:

  ```sh
  dosker run -d --name hello-bundled-demo \
    --boot-timeout 120 hello-bundled:latest
  ```
- [ ] Run `dosker ps` and confirm `hello-bundled-demo` is `running`.
- [ ] Run:

  ```sh
  dosker exec hello-bundled-demo 'TYPE C:\APP\HELLO.TXT'
  ```

  Confirm `HELLO FROM DOSKER`.
- [ ] Run:

  ```sh
  dosker exec hello-bundled-demo 'TYPE C:\APP\NOTES.TXT'
  ```

  Confirm `This file was copied with COPY.`
- [ ] Run:

  ```sh
  dosker exec hello-bundled-demo 'TYPE C:\APP\DATA.TXT'
  ```

  Confirm `UPLOADED FROM THE HOST`. The `RUN COPY` step created this file
  during the image build.
- [ ] Run:

  ```sh
  dosker exec hello-bundled-demo 'DIR C:\APP'
  ```

  Confirm `HELLO.TXT`, `NOTES.TXT`, and `DATA.TXT` are present.
- [ ] Run `dosker volume ls` and confirm no volume is attached.
- [ ] Save the image:

  ```sh
  dosker save -o "$DOSKER_ROOT/hello-bundled.tar" hello-bundled:latest
  ```

  Confirm the archive is about 32 MiB.
- [ ] Stop and remove the demo container:

  ```sh
  dosker stop hello-bundled-demo
  dosker rm hello-bundled-demo
  ```
- [ ] Remove and load the image:

  ```sh
  dosker rmi hello-bundled:latest
  dosker load "$DOSKER_ROOT/hello-bundled.tar"
  dosker images
  ```

  Confirm `hello-bundled:latest` is present.
- [ ] Run the loaded image without a volume:

  ```sh
  dosker run -d --name hello-bundled-loaded \
    --boot-timeout 120 hello-bundled:latest
  ```
- [ ] Run:

  ```sh
  dosker exec hello-bundled-loaded 'TYPE C:\APP\HELLO.TXT'
  dosker exec hello-bundled-loaded 'TYPE C:\APP\DATA.TXT'
  ```

  Confirm `HELLO FROM DOSKER` and `UPLOADED FROM THE HOST`.
- [ ] Run `dosker volume ls` and confirm no volume is attached.
- [ ] Stop and remove the loaded container:

  ```sh
  dosker stop hello-bundled-loaded
  dosker rm hello-bundled-loaded
  ```
- [ ] Run `rm -rf "$DOSKER_ROOT"` and remove the clean checkout.

## Result

Verified on 2026-11-23 on macOS arm64 (Darwin).

- DOSker: workspace `target/debug/dosker` 0.1.2.
- QEMU: 11.1.3.
- Agent: 1.0.0.
- Base: `freedos:1.3`,
  `sha256:640eb8a69e594c7b37d07e68e42e23931d03e7da04666ff83115516810d71268`.
- Bundled image: `hello-bundled:latest`,
  `sha256:ab805e6093e87557485c6e5be282a937f41a59206b70ea3d93acb97ebc8ebd2e`.
- The image size is 33,554,432 bytes.
- The `COPY` steps created `C:\APP\HELLO.TXT`, `C:\APP\NOTES.TXT`, and
  `C:\UPLOAD.TXT`.
- The `RUN COPY` step created `C:\APP\DATA.TXT` and reported
  `C:\UPLOAD.TXT =>> C:\APP\DATA.TXT`.
- `HELLO.TXT`, `NOTES.TXT`, and `DATA.TXT` read back correctly.
- `dosker volume ls` is empty before and after save/load.
- The save archive is 32 MiB. Loading it restored only
  `hello-bundled:latest`, and the loaded image ran without a volume.
- The image config contains `CMD ECHO HELLO FROM BUNDLED DOSKER`.
  `dosker logs` did not show the start command for this projected image,
  so this checklist verifies the image tree with `dosker exec`.
