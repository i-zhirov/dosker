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

## List and tag

```sh
dosker images
dosker tag SOURCE TARGET
```

`images` lists local tags. `tag` adds or replaces a target tag without copying
blobs.

## Remove

`rmi` has two distinct operations:

- `dosker rmi name:tag` **untags only**. The manifest and its blobs remain
  reachable by digest. Removing the last tag does not delete the image.
- `dosker rmi sha256:<hex>` removes an untagged manifest and the config, layer,
  and raw-disk blobs that no other image references.

A digest that still has a tag returns `ImageInUse`; remove its tags first.
Image import, tag, and remove take an exclusive `images/.lock` while they
update the index, so concurrent mutations are serialized.
