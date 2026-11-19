# Volumes and network

This page describes named volumes and published ports. It also describes
the guest network and the integration fixture for the published-port
suite.

## Volumes

A volume is a raw FAT disk image. DOSker stores it under the data root:

```text
volumes/<name>/volume.img
volumes/<name>/meta.json
volumes/<name>/.lock
```

`meta.json` records the schema version, name, size, layout, FAT type,
creation time, and dirty flag. The `.lock` file holds the container ID,
the container name, and the supervisor PID while a container uses the
volume.

The default size is 64 MiB. `--size` accepts `K`, `M`, or `G` suffixes in
base 1024. A size without a suffix is in bytes.

| Size | Layout | FAT |
| --- | --- | --- |
| `1474560` (1.44 MB) | `superfloppy` | `fat12` |
| `2949120` (2.88 MB) | `superfloppy` | `fat16` |
| 16 MiB to 1 GiB | `mbr_partitioned` | `fat16` |

A volume size must be a multiple of 512 bytes. A name must match
`[a-z0-9][a-z0-9_.-]{0,62}`.

Mount a volume with `-v`:

```sh
dosker run -d -v data:D: freedos:latest
dosker run -d -v data:D:ro freedos:latest
```

The canonical form is `-v name:D:`. `-v name:D` is also accepted. A
literal `:ro` suffix mounts the volume read-only.

The mountable drives are `A:`, `B:`, `D:`, `E:`, and `F:`. `C:` is the
boot disk and is never a volume target. `A:` and `B:` need a `superfloppy`
volume and an IDE boot image. `D:` to `F:` need an `mbr_partitioned`
volume.

A read-only floppy rejects guest writes. QEMU cannot attach a read-only
IDE hard disk, so a read-only hard-disk volume uses a temporary QEMU
overlay. Guest writes go to the overlay and are discarded. The volume
image is never modified.

A volume attaches to at most one running container. A second attach fails
and names the container that holds the volume.

A container start warns on stderr when an attached volume was not shut
down cleanly:

```text
warning: volume data was not shut down cleanly
```

An image can declare a volume with `VOLUME D:`. `create` makes an
anonymous volume for each declared drive that has no `-v` mount. The name
is `anon-<container-id>-<lowercase drive letter>`. `A:` and `B:` anonymous
volumes are 1.44 MB. The other drives are 64 MiB.

A volume survives `dosker rm`. Only `dosker volume rm` removes a volume.

Manage volumes with these commands:

```sh
dosker volume create --size 64M data
dosker volume ls
dosker volume rm data
```

`dosker volume ls` prints the header `NAME SIZE LAYOUT ATTACHED`. The size
is in bytes. An unattached volume shows `-` in the `ATTACHED` column.
`dosker volume rm` refuses an attached volume and names the holder.

## Publishing ports

Publish a guest TCP port with `-p`:

```sh
dosker run -d -p 8080:80 freedos:latest
dosker run -d -p 0.0.0.0:8080:80 freedos:latest
```

The form is `-p [addr:]host:guest`. The default host address is
`127.0.0.1`. The ports are in the range 1 to 65535. The protocol is `tcp`.

A `(host_address, host_port)` pair must be unique across starting and
running containers. A second start fails with an `already published`
message and leaves the first container running. The CLI check is best
effort across processes. The QEMU NAT bind is the final guard.

`dosker port` prints one line per mapping:

```text
127.0.0.1:8080->80/tcp
```

A container with no published ports prints nothing.

NAT is a QEMU capability. An engine without NAT rejects `-p`. An image
with `MACHINE net=none` also rejects `-p`.

## Guest networking

QEMU uses user-mode NAT. The command line contains:

```text
-nic user,model=ne2k_isa,hostfwd=tcp:127.0.0.1:8080-:80
```

The guest sees an ISA NE2000 adapter. The guest needs a packet driver for
the NE2000 and a TCP/IP stack. For the QEMU ISA NE2000, load the driver
with software interrupt `0x60`, I/O port `0x300`, and IRQ `9`:

```text
NE2000.COM 0x60 0x9 0x300
```

The QEMU user network uses `10.0.2.15/24` for the guest, `10.0.2.2` for
the gateway, and `10.0.2.3` for DNS. A DHCP client can also configure the
guest.

`CONFIG.SYS` must set `LASTDRIVE` high enough for the mounted drives. The
FreeDOS base image sets `LASTDRIVE=Z`.

## Integration test fixture

The published-port suite needs a booter image with a guest TCP service.
Set `DOSKER_QEMU_NET_IMAGE` to the image path. A relative path resolves
against the workspace root.

The image must boot DOS and start a packet driver and a TCP server on
port 80 from `AUTOEXEC.BAT`. The fixture used by the repository has this
layout:

```text
C:\AUTOEXEC.BAT
C:\NET\NE2000.COM
C:\NET\HTTPSERV.EXE
C:\NET\MTCP.CFG
C:\HTTP\INDEX.HTM
```

`AUTOEXEC.BAT` sets `MTCPCFG`, loads `NE2000.COM`, and starts
`HTTPSERV.EXE`. `MTCP.CFG` sets the packet interrupt and the static
network values.

The suite fails when integration is enabled and the image is missing. It
does not skip. In CI, set the repository variable
`DOSKER_QEMU_NET_IMAGE_URL` to a raw `.img` URL. The workflow downloads it
to `agent/fixtures/dosker-net.img`.
