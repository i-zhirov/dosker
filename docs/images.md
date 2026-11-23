# Images

DOSker stores images in the local OCI image layout under the data root:

```text
<data-root>/images/
  oci-layout
  index.json
  blobs/sha256/<hex>
```

The store uses these media types:

- `application/vnd.dosker.layer.v1.tar`
- `application/vnd.dosker.image.config.v1+json`
- `application/vnd.dosker.booter.v1.raw`

Blobs are content-addressed by SHA-256 and are read-only. Container disks are
private writable copies; the stored image is never modified.

## Import

```sh
dosker image import [--booter] FILE name:tag
```

A DOS disk image is read into the canonical tree and boot metadata. A booter
blob is stored as one opaque blob and has no agent.

A disk import also injects the DOSker agent. The agent is written to
`C:\DOSKER\DOSKER.EXE`. If the image has an `AUTOEXEC.BAT`, the importer saves
it as `C:\DOSKER\AUTOEXEC.OLD` and replaces `AUTOEXEC.BAT` with a file that
calls the saved copy and then starts the agent. An image without
`AUTOEXEC.BAT` gets a new file with the agent line. A line that already starts
the agent is left alone, so importing an image with the agent already present
does not create a second `AUTOEXEC.OLD` or duplicate the agent line. Every
injected entry uses the fixed `1980-01-01T00:00:00Z` timestamp. Imported disks
get `dos_flavour=other` and agent version `1.0.0`. A booter import stays
opaque: it gets no agent and no annotations.

The manifest of a disk import records two annotations:

- `org.dosker.rawdisk.digest` is the digest of the stored raw disk after
  injection, which is the blob that `copy_disk_to` materializes.
- `org.dosker.source.digest` is the digest of the original source file before
  injection.

## Pull

```sh
dosker pull owner/name:tag
dosker pull owner/name@sha256:<hex>
dosker pull owner/name:tag --accept-license <image-digest>
```

`pull` resolves the reference through the Pages API, verifies the manifest,
the config, the asset, and every blob, and stores the image. A tag pull stores
`owner/name:<tag>`; a digest pull stores `owner/name:<version>`.

An image whose verified config license summary is not `redistributable`
refuses with exit code `3` and prints the exact digest to accept. Acceptance is
stored by digest under `<data-root>/accepted/`. The downloaded asset is cached
at `<data-root>/cache/registry/assets/<hex>.tar` and re-hashed on every pull.

`run`, `create`, and `build` pull a missing remote reference. Pass `--no-pull`
to keep the local-only behavior.

A takedown removes a version from the registry. It never deletes a local copy:
a local tag still starts offline, while the next pull of a revoked tag fails.

## Search and remote inspect

```sh
dosker search <term> [--json]
dosker image inspect owner/name:tag
```

`search` filters the catalog from `/index.json` by name, description, and tag.
`--json` prints the catalog object with the filtered images. `image inspect`
shows a local image or a remote image; the remote view shows the tag or digest,
version, digests, asset URL and size, license summary, and the license entries.

## List and tag

```sh
dosker images
dosker tag SOURCE TARGET
```

`images` lists local tags. `tag` adds or replaces a target tag without copying
blobs.

## Namespaced tags

A namespaced reference such as `owner/name:tag` is a valid local tag. `tag`,
`rmi`, `save`, and `load` accept it, and `images` prints the full reference.
`save` uses `owner-name-tag.tar` as the default output name, replacing each
`/` with `-`.

## Remove

`rmi` has two distinct operations:

- `dosker rmi name:tag` **untags only**. The manifest and its blobs remain
  reachable by digest. Removing the last tag does not delete the image.
- `dosker rmi sha256:<hex>` removes an untagged manifest and the config, layer,
  and raw-disk blobs that no other image references.

A digest that still has a tag returns `ImageInUse`; remove its tags first.
Image import, tag, and remove take an exclusive `images/.lock` while they
update the index, so concurrent mutations are serialized.

## Save and load

```sh
dosker save [-o FILE] name:tag
dosker load FILE
```

`save` writes one tagged image to an OCI image-layout tar. The default output
is `name-tag.tar` in the current directory. The tar has the members
`oci-layout`, `index.json`, and `blobs/sha256/<hex>`.

`load` restores every tag named by the archive index and replaces an existing
tag. A failed load writes no tag. A load may leave unreferenced
content-addressed blobs, and they do not affect a later load.

## Prune

`dosker system prune` removes containers in `created`, `stopped`, `exited`, or
`failed` state, untagged manifests that no remaining container references, and
cache records with a missing or corrupt layer blob or an unsupported schema. It
keeps running containers, tagged images, images used by a remaining container,
volumes, and valid cache records.

## Boot metadata

A layer may carry a boot override. The effective boot is the last non-null
`boot` in layer order, or the image config when no layer overrides it. A
`RUN` that changes the boot artifacts writes the override, and the next
projection uses it.
