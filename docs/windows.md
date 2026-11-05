# Windows runtime

The Windows runtime supports Windows x86_64. Windows arm64 is not supported.

## Host integration

- The data root is `%LOCALAPPDATA%\dosker`. Set `DOSKER_ROOT` to override it.
- The supervisor control endpoint is the named pipe
  `\\.\pipe\dosker-<id>-control`. Its DACL allows only the current user.
- State writes use `ReplaceFileW`. A missing target falls back to
  `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`.
- QEMU runs in a Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. If the supervisor exits, the Job
  Object closes and terminates the engine.

## Engine networking

The serial agent, QMP, monitor, and gdbstub endpoints bind to
`127.0.0.1` on separate TCP ports. VNC also uses a loopback address and a
port at or above `5900`.

`--display window` is not supported on Windows. Use `--display none` or
`--display vnc`.

NAT is not yet supported.

## QEMU discovery

Discovery checks the following locations in order:

1. `DOSKER_QEMU`
2. `PATH`
3. `%ProgramFiles%\qemu`
4. `%ProgramFiles(x86)%\qemu`
5. `%LOCALAPPDATA%\Programs\qemu`

The program name is `qemu-system-i386.exe`. A bare name is resolved with the
`PATHEXT` suffixes.

The minimum QEMU version is `7.1.0`.

## Environment variables

- `DOSKER_QEMU` selects the QEMU executable.
- `DOSKER_QEMU_SHA256` pins the executable SHA-256. The value is the
  lowercase hexadecimal digest.
- `DOSKER_QEMU_VERSION` pins the exact version. The value is a bare
  `X.Y.Z` string.
- `DOSKER_QEMU_IMAGE` selects the prepared FreeDOS image for integration
  tests.
- `DOSKER_INTEGRATION` enables the real-engine integration tests. Set it to
  `1`.

## CI variables

The Windows QEMU CI job requires these repository variables:

- `DOSKER_WINDOWS_QEMU_URL` is the QEMU installer `.exe` URL.
- `DOSKER_WINDOWS_QEMU_SHA256` is the installer SHA-256.
- `DOSKER_WINDOWS_QEMU_VERSION` is the QEMU version in `X.Y.Z`
  form.
- `DOSKER_QEMU_IMAGE_URL` is the prepared image URL. It can be a ZIP archive
  that contains a `.img` file or a raw image URL.

Set them in *Settings > Secrets and variables > Actions > Variables*. They
are not secrets. The job fails when any value is empty.

The smoke command is:

```sh
DOSKER_INTEGRATION=1 \
DOSKER_QEMU=qemu-system-i386.exe \
DOSKER_QEMU_IMAGE=agent/fixtures/dosker-agent.img \
cargo test -p dosker-cli --features qemu-engine \
  --test windows_qemu_runtime -- --nocapture
```

## Security

DOSker does not sandbox untrusted images. Run untrusted images in an outer
virtual machine.
