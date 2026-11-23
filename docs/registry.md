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
`https://i-zhirov.github.io/doskerfile-registry`.

## Pull, search, and inspect

```sh
dosker pull owner/name:tag
dosker pull owner/name@sha256:<hex>
dosker pull owner/name:tag --accept-license <image-digest>
dosker search <term> [--json]
dosker image inspect <ref>
```

`pull` fetches the per-image index, `manifest.json`, and `config.json`. It
verifies the manifest digest, the config digest, and the config, then reads
`licenses` from the verified config. It never uses the catalog summary for the
gate. It downloads the tar from `asset_url`, verifies the asset digest and
size, verifies the tar's manifest and config, and stores the image.

A tag pull stores `owner/name:<tag>`. A digest pull stores
`owner/name:<version>`.

An image whose verified license summary is not `redistributable` needs
acceptance. `pull` prints the image digest and exits with code `3` until the
user passes `--accept-license <image-digest>`. Acceptance is stored by digest
under `<data-root>/accepted/`. A different digest needs a new acceptance.

`search` reads `/index.json` and filters the image name, the description, and
the tags. `--json` prints the catalog object with the filtered images.

`image inspect` shows a local image or a remote image. A remote view shows the
tag or digest, version, digests, asset URL and size, license summary, and the
license entries.

The client uses HTTPS. Plain HTTP is accepted only for `localhost` and
`127.0.0.1`. A redirect from HTTPS to a non-HTTPS URL is refused. An asset is
capped at 2 GiB.

The asset is cached at `<data-root>/cache/registry/assets/<hex>.tar`. Every
pull re-hashes the cached bytes. Site documents are fetched on every pull.

`run`, `create`, and `build` pull a missing namespaced or host-qualified
reference. `run --no-pull`, `create --no-pull`, and `build --no-pull` keep the
local-only behavior.

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

A missing remote base is pulled before the build. Use `build --no-pull` to
keep the local-only check. The validator still reads the base from the local
image store; a missing base reports a pull hint.

The registry repository runs the same command for every image in its
`validate` workflow.

## Final tree check

```sh
dosker registry check-final <ref>
```

`registry check-final` reads the stored config and final tree of a built image
and repeats the license, coverage, and notice checks on that tree. It sees the
files after every `RUN` step and never reads the Doskerfile. `publish` runs it
after the build and before it uploads the tar.

It exits `0` on success, `1` on a check failure, and `2` on a usage error. A
missing image reports a build hint. A booter image uses the booter rule.
