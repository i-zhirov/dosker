#!/usr/bin/env bash
set -euo pipefail

# Report every missing required repository variable in one message. The
# expected format follows the variable name so the reader can fix the
# configuration without a second round trip.

formats=(
  "DOSKER_FREEDOS_SOURCE|FreeDOS Floppy Edition ZIP URL, raw boot floppy image URL, or local path"
  "DOSKER_QEMU_IMAGE_URL|ZIP archive containing a .img file, or a raw image URL"
  "DOSKER_WINDOWS_QEMU_URL|Windows QEMU ZIP archive URL"
  "DOSKER_WINDOWS_QEMU_SHA256|64-character hexadecimal SHA-256 of the ZIP"
  "DOSKER_WINDOWS_QEMU_VERSION|bare X.Y.Z version"
)

selected=("$@")
if ((${#selected[@]} == 0)); then
  for entry in "${formats[@]}"; do
    selected+=("${entry%%|*}")
  done
fi

format_for() {
  local name="$1"
  local entry
  for entry in "${formats[@]}"; do
    if [[ "${entry%%|*}" == "$name" ]]; then
      printf '%s\n' "${entry#*|}"
      return 0
    fi
  done
  return 1
}

missing=()
for name in "${selected[@]}"; do
  value="${!name-}"
  if [[ -z "${value//[[:space:]]/}" ]]; then
    if ! format="$(format_for "$name")"; then
      printf '::error::unknown required repository variable: %s\n' "$name" >&2
      exit 1
    fi
    missing+=("${name}: ${format}")
  fi
done

if ((${#missing[@]} == 0)); then
  exit 0
fi

message="Missing required repository variables:"
for item in "${missing[@]}"; do
  message+="%0A- ${item}"
done

printf '::error::%s\n' "$message"
printf 'Missing required repository variables:\n' >&2
for item in "${missing[@]}"; do
  printf '  - %s\n' "$item" >&2
done
exit 1
