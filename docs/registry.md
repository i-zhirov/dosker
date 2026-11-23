# Registry

A registry checkout describes distributable images:

```text
<registry>/
  namespaces.toml
  images/<owner>/<name>/
    Doskerfile
    registry.toml
    NOTICE.txt        # when a notice is required
    ...
```

`namespaces.toml` proves an owner. `registry.toml` describes one image.

## Reference forms

| Form | Meaning |
| --- | --- |
| `name:tag` | A local image tag. |
| `name` | A local image tag, `latest`. |
| `owner/name` | A namespaced image, `latest`. |
| `owner/name:tag` | A namespaced image. |
| `host/owner/name:tag` | An image from an explicit registry host. |
| `owner/name@sha256:<hex>` | A namespaced image digest. |
| `sha256:<hex>` | A bare local image digest. |

A pulled or namespaced image is stored locally as `owner/name:tag`. `tag`,
`rmi`, `save`, and `load` accept that form. `save` writes `owner-name-tag.tar`
by default. `images` prints the full reference.

`DOSKER_REGISTRY` sets the default registry base URL. The default is
`https://i-zhirov.github.io/doskerfile-registry`. The network client that
fetches from it is a later slice; this release resolves a base from the local
image store only.

## Validation

```sh
dosker registry validate <path>
dosker registry validate <image-dir> --registry-root <registry>
```

`<path>` is either an image directory or a registry root that contains
`images/`. The command exits `0` when every image is valid, `1` when a check
fails, and `2` on a usage error. It never runs `RUN` or `RUN --host`.

It checks:

- `registry.toml`: `schema = 1`, a tag-shaped `version`, one to 32 unique
  tag-shaped tags, and a `description` of 1 to 280 characters.
- `namespaces.toml`: `schema = 1`, every key is a lowercase GitHub handle, the
  owner's `github` matches the key, and `proof` is an HTTP(S) URL.
- Every image owner is proven, and tags are unique across the whole registry.
- The Doskerfile parses, every image declares at least one `LICENSE` entry,
  and the entries pass the license rules, including the contradictory
  status/basis/SPDX check.
- The base licenses are inherited. A `COPY`-only replay checks coverage for
  the effective tree. The system `DOS` and `DOSKER` directories and the DOS
  kernel and startup files are exempt.
- A copyleft entry has `SOURCE_URL`. An image with any copyleft or
  `abandonware` entry contains `C:\DOSKER\NOTICE.TXT`. The notice is written
  with `COPY NOTICE.txt C:\DOSKER\NOTICE.TXT`; that one path is the only
  allowed write under `C:\DOSKER`.
- A booter image declares `COVERS=*`. A booter with a copyleft or
  `abandonware` entry fails because its tree cannot be inspected.

The base must be present in the local image store in this release. A missing
base reports a pull hint. The remote `DOSKER_REGISTRY` base fetch arrives with
the network client.

The registry repository runs the same command for every image in its
`validate` workflow.
