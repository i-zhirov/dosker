# Manual verification for the OpenWatcom example

Use this checklist to confirm that the README example works from a clean
checkout. Run the commands from the repository root.

## Clean checkout

```sh
git clone https://github.com/i-zhirov/dosker.git dosker-clean
cd dosker-clean
```

The checkout has no `target`, no `agent/build`, and no FreeDOS source. The
setup script prepares the inputs. It also downloads the OpenWatcom DOS
package to the cache.

## Prepare

Start a new shell, or unset `DOSKER_ROOT`, `DOSKER_AGENT_EXE`,
`DOSKER_FREEDOS_SOURCE`, and `DOSKER_QEMU`. Then source the setup script:

```sh
. examples/openwatcom-dos/setup.sh
```

The script checks `curl`, `unzip`, and `mtools`, downloads the DOS agent
from CI, fetches the FreeDOS source, finds QEMU, creates a temporary
`DOSKER_ROOT`, and builds `dosker` if needed. It downloads
`open-watcom-2_0-c-dos.exe`, verifies its SHA-256, extracts the DOS
toolchain, creates the `watcom-tools` volume, installs the toolchain, and
builds `openwatcom:latest`.

The cache is `${XDG_CACHE_HOME:-$HOME/.cache}/dosker-openwatcom-dos`.
The OpenWatcom package is about 122 MiB. The extracted tree is about
118 MiB. The volume is 256 MiB.

The script prints `DOSKER_ROOT`, the toolchain paths, the source bind, and
the `dosker run` command.

## Steps

- [ ] Confirm the setup printed `setup complete` and `built
  openwatcom:latest sha256:`.
- [ ] Confirm the toolchain cache has `BINW/WCC.EXE`, `BINW/WLINK.EXE`,
  and `BINW/DOS4GW.EXE`.
- [ ] Run the printed command. It has this shape:

  ```sh
  dosker run -d --name openwatcom-demo --boot-timeout 120 \
    --bind D:"$PWD/examples/openwatcom-dos/src" \
    -v watcom-tools:E:ro openwatcom:latest
  ```
- [ ] Run `dosker ps` and confirm `openwatcom-demo` is `running`.
- [ ] Run `dosker volume ls` and confirm `watcom-tools` with size
  `268435456` and layout `mbr_partitioned`.
- [ ] Run `dosker exec openwatcom-demo D:\BUILD.BAT`. Confirm exit status
  0. The DOS4GW banner appears for `WCC` and `WLINK`.
- [ ] Run `ls -l examples/openwatcom-dos/src` and confirm
  `hello.obj`, `hello.exe`, and `hello.map`. QEMU vvfat may write the
  names in lowercase.
- [ ] Run `dosker exec openwatcom-demo D:\HELLO.EXE` and confirm
  `HELLO FROM OPENWATCOM`.
- [ ] Edit the message in `examples/openwatcom-dos/src/HELLO.C` on the
  host.
- [ ] Stop and remove the container, then run the printed command again:

  ```sh
  dosker stop openwatcom-demo
  dosker rm openwatcom-demo
  # Run the printed command again.
  ```

  QEMU vvfat caches host directory entries. A fresh container is the
  reliable way to see a host edit. The `watcom-tools` volume and
  `openwatcom:latest` image are reused.
- [ ] Run `dosker exec openwatcom-demo D:\BUILD.BAT` and then
  `dosker exec openwatcom-demo D:\HELLO.EXE`. Confirm the new message.
- [ ] Restore the original message in `HELLO.C`.
- [ ] Stop and remove the container. Run the printed command again.
  Build and run. Confirm the restored message. The toolchain is not
  downloaded or installed again. The volume is reused.
- [ ] Confirm the source bind refuses a non-8.3 name. Create
  `examples/openwatcom-dos/src/.DS_Store`, then run `dosker run` with a
  different container name. Confirm it fails before boot and names the
  path. Remove the file.
- [ ] Stop and remove the container and the volume:

  ```sh
  dosker stop openwatcom-demo
  dosker rm openwatcom-demo
  dosker volume rm watcom-tools
  dosker system prune
  rm -rf "$DOSKER_ROOT"
  ```
- [ ] Run `dosker ps -a` and confirm no containers. Run
  `dosker volume ls` and confirm no volumes. The image is in the removed
  data root.

## Result

Verified on 2026-11-22 on macOS arm64 (Darwin).

- DOSker: 0.1.2 Homebrew binary.
- QEMU: 11.1.3.
- Agent: 1.0.0 from `agent/build/DOSKER.EXE`.
- Base: `freedos:1.3`.
- OpenWatcom: `2026-11-01-Build`, package SHA-256
  `21acc705be9fb04057c8f4ccddcfec659e3ea2aa3a6be463473c222b87a530d1`.
- Setup, volume install, build, run, host output, and the edit/rebuild
  loop passed. The edit was checked with `HELLO FROM THE HOST`, then
  restored to `HELLO FROM OPENWATCOM`.
- The fresh-container workflow was used after the host edit. A live
  rebuild in the same container saw cached vvfat content.
- The `watcom-tools` volume and `openwatcom:latest` image were reused
  between containers during the edit check.
- The non-8.3 bind check passed. The run rejected
  `src/.DS_Store` before boot with `invalid host path` and
  `invalid DOS path`.
- Final cleanup removed the container, the `watcom-tools` volume, the
  `openwatcom:latest` image, and the temporary data root. `dosker ps -a`
  and `dosker volume ls` were empty after the volume removal.
- The final pass reused the cached OpenWatcom package and FreeDOS source.
  The agent came from `agent/build/DOSKER.EXE`. The cache-miss download
  path was exercised when the package was first fetched.
