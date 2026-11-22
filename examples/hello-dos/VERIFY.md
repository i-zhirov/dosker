# Manual verification for hello-dos

Use this checklist to confirm that the README example works from a clean
checkout. Run the commands from the repository root.

## Clean checkout

```sh
git clone https://github.com/i-zhirov/dosker.git dosker-clean
cd dosker-clean
```

The checkout has no `target`, no `agent/build`, and no FreeDOS source. The
setup script prepares all three.

## Prepare

Start a new shell, or unset `DOSKER_ROOT`, `DOSKER_AGENT_EXE`,
`DOSKER_FREEDOS_SOURCE`, and `DOSKER_QEMU`. Then source the setup script:

```sh
. examples/hello-dos/setup.sh
```

The script prints the paths. It downloads the DOS agent from CI, fetches and
verifies the FreeDOS source, finds QEMU, creates a temporary `DOSKER_ROOT`,
and builds `dosker` if needed. It then builds `hello:latest`, creates the
`hello-data` volume, and prints the `dosker run` command. It caches the agent
and FreeDOS source in `${XDG_CACHE_HOME:-$HOME/.cache}/dosker-hello-dos`.

If you used the setup script, skip the build and volume steps below. To
verify those commands exactly as documented, use the manual export block in
the README instead of the script.

The checkout includes `examples/hello-dos/Doskerfile.lock`. If the build
reports a lock mismatch, run the first build with `--update` as shown in the
README.

## Steps

- [ ] (Manual setup only) Build the image:
  `dosker build -t hello:latest examples/hello-dos`
  and confirm `built hello:latest sha256:`.
- [ ] (Manual setup only) Create the volume:
  `dosker volume create --size 64M hello-data`
  and confirm `hello-data`.
- [ ] Start the container and confirm a 12-character id:

  ```sh
  dosker run -d --name hello-demo --boot-timeout 120 \
    -v hello-data:D: -p 8080:80 hello:latest
  ```
- [ ] Run `dosker ps` and confirm `hello-demo` is `running`.
- [ ] Run `dosker logs hello-demo` and confirm a `cmd exit=0` line for
  `ECHO HELLO FROM DOSKER`.
- [ ] Run `dosker port hello-demo` and confirm
  `127.0.0.1:8080->80/tcp`.
- [ ] Run `dosker exec hello-demo ECHO HELLO` and confirm `HELLO`.
- [ ] Run `dosker exec hello-demo MD 'C:\APP'` and confirm it succeeds.
- [ ] Run `dosker exec hello-demo DIR 'C:\APP'` and confirm the directory
  listing.
- [ ] Run `dosker exec -w 'C:\APP' hello-demo DIR` and confirm the same
  listing.
- [ ] Run
  `dosker cp examples/hello-dos/HELLO.TXT 'hello-demo:C:\APP\HELLO.TXT'`
  and confirm `copied 18 bytes`.
- [ ] Run
  `dosker cp examples/hello-dos/NOTES.TXT 'hello-demo:C:\APP\NOTES.TXT'`
  and confirm `copied 37 bytes`.
- [ ] Run `dosker exec hello-demo TYPE 'C:\APP\HELLO.TXT'` and confirm
  `HELLO FROM DOSKER`.
- [ ] Run
  `dosker cp 'hello-demo:C:\APP\HELLO.TXT' "$DOSKER_ROOT/HELLO-FROM-DOS.TXT"`
  and confirm `copied 18 bytes`.
- [ ] Run `cat "$DOSKER_ROOT/HELLO-FROM-DOS.TXT"` and confirm
  `HELLO FROM DOSKER`.
- [ ] Run
  `dosker cp examples/hello-dos/UPLOAD.TXT 'hello-demo:C:\UPLOAD.TXT'`
  and confirm `copied 23 bytes`.
- [ ] Run
  `dosker exec hello-demo COPY 'C:\UPLOAD.TXT' 'D:\DATA.TXT'`
  and confirm `C:\UPLOAD.TXT =>> D:\DATA.TXT`.
- [ ] Run `dosker exec hello-demo TYPE 'D:\DATA.TXT'` and confirm
  `UPLOADED FROM THE HOST`.
- [ ] Run `dosker volume ls` and confirm `hello-data` with `hello-demo`.
- [ ] Run `dosker stop hello-demo` and confirm `hello-demo stopped`.
- [ ] Run `dosker start hello-demo --boot-timeout 120` and confirm
  `hello-demo running`.
- [ ] Run `dosker exec hello-demo TYPE 'D:\DATA.TXT'` and confirm
  `UPLOADED FROM THE HOST`.
- [ ] Run `dosker attach hello-demo` in a terminal. Type
  `TYPE C:\APP\HELLO.TXT` and confirm `HELLO FROM DOSKER`. Type `EXIT` and
  confirm the container is still `running`.
- [ ] Run
  `dosker save -o "$DOSKER_ROOT/hello.tar" hello:latest`
  and confirm `saved hello:latest to`.
- [ ] Run `dosker rmi hello:latest` and confirm `untagged hello:latest`.
- [ ] Run `dosker load "$DOSKER_ROOT/hello.tar"` and confirm
  `loaded hello:latest`.
- [ ] Run `dosker images` and confirm `freedos:1.3` and `hello:latest`.
- [ ] Run `dosker stop hello-demo` and `dosker rm hello-demo`.
- [ ] Run `dosker volume rm hello-data` and `dosker system prune`.
- [ ] Run `dosker ps -a` and confirm no containers.
- [ ] Run `dosker volume ls` and confirm no volumes.
- [ ] Run `rm -rf "$DOSKER_ROOT"` and remove the clean checkout.

## Result

Verified on 2026-11-22 from a clean archive with no `target`,
`agent/build`, FreeDOS source, or data root.

- Host: macOS arm64 (Darwin).
- DOSker: 0.1.0 release binary. The source-build fallback also built
  0.1.0.
- QEMU: 11.1.3.
- Agent: 1.0.0 from the `dosker-agent-binaries` CI artifact.
- Base: `freedos:1.3` built from the fetched FreeDOS source.
- Full README check: all checklist steps passed. The `attach` step was
  checked with piped input.
- Ready-to-run setup: `setup.sh` built `hello:latest`, created
  `hello-data`, and the printed `dosker run` command passed the run, exec,
  copy, volume, and cleanup checks.
