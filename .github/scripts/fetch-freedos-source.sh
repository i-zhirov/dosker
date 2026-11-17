#!/usr/bin/env bash
set -euo pipefail

# Resolve DOSKER_FREEDOS_SOURCE to a local FreeDOS boot floppy image and print
# its path on stdout. The CLI requires a local image; CI accepts a ZIP URL, a
# raw image URL, or a local path and resolves it here first.

VARIABLE='DOSKER_FREEDOS_SOURCE'
PINNED_ZIP_SHA256='75a4e11a7fce6f124e20927b3022b4b715a2a3f7324c5f5bfea42d90d80eb072'
ZIP_MEMBER='144m/x86BOOT.img'

input="${!VARIABLE:-}"

die() {
  printf '%s (%s): %s\n' "$VARIABLE" "${input:-<unset>}" "$*" >&2
  exit 1
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "cannot verify the archive: neither sha256sum nor shasum is available"
  fi
}

[ -n "$input" ] || die "is not set; set it to a FreeDOS 1.3 Floppy Edition ZIP URL, a raw boot floppy image URL, or a local path"

work="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/dosker-freedos.XXXXXX")" \
  || die "cannot create a temporary directory"

case "$input" in
  http://*|https://*)
    command -v curl >/dev/null 2>&1 || die "curl is required to download the source"
    case "$input" in
      *.[zZ][iI][pP])
        archive="$work/freedos-floppy.zip"
        if ! curl -fsSL --retry 3 --retry-delay 2 -o "$archive" "$input"; then
          die "ZIP download failed"
        fi

        if ! actual="$(sha256_of "$archive")"; then
          die "cannot compute the ZIP SHA-256"
        fi
        if [ "$actual" != "$PINNED_ZIP_SHA256" ]; then
          die "ZIP SHA-256 mismatch: expected $PINNED_ZIP_SHA256, got $actual"
        fi

        command -v unzip >/dev/null 2>&1 || die "unzip is required to extract the ZIP"
        if ! unzip -q -o "$archive" "$ZIP_MEMBER" -d "$work"; then
          die "ZIP does not contain $ZIP_MEMBER"
        fi

        source_path="$work/$ZIP_MEMBER"
        ;;
      *)
        source_path="$work/x86BOOT.img"
        if ! curl -fsSL --retry 3 --retry-delay 2 -o "$source_path" "$input"; then
          die "image download failed"
        fi
        ;;
    esac
    ;;
  *)
    source_path="$input"
    ;;
esac

[ -f "$source_path" ] || die "does not name a regular file: $source_path"

printf '%s\n' "$source_path"
