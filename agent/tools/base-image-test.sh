#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

for extra in /opt/homebrew/opt/util-linux/sbin; do
  if [ -d "$extra" ]; then
    PATH="$extra:$PATH"
  fi
done
export PATH

IMAGE="${1:-${DOSKER_BASE_IMAGE:-$ROOT/agent/fixtures/freedos-base.img}}"

die() {
  printf 'base-image-test: %s\n' "$*" >&2
  exit 1
}

[ -f "$IMAGE" ] || die "base image not found: $IMAGE"
command -v sfdisk >/dev/null 2>&1 || die "missing required tool: sfdisk"
command -v mdir >/dev/null 2>&1 || die "missing required tool: mdir"
command -v minfo >/dev/null 2>&1 || die "missing required tool: minfo"

SIZE="$(wc -c < "$IMAGE" | tr -d '[:space:]')"
[ "$SIZE" = "33554432" ] || die "expected 33554432 bytes, found $SIZE"

PARTITIONS="$(sfdisk --dump "$IMAGE")"
COUNT="$(printf '%s\n' "$PARTITIONS" | grep -c '^/.*:')"
[ "$COUNT" = "1" ] || die "expected exactly one partition, found $COUNT"
printf '%s\n' "$PARTITIONS" | \
  grep -Eq 'start=[[:space:]]*2048, size=[[:space:]]*63488, type=6, bootable' || \
  die "partition is not a bootable type 6 partition at sector 2048 with size 63488"

MINFO="$(MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 minfo -i "$IMAGE@@1048576")"
printf '%s\n' "$MINFO" | grep -Eq 'disk type="FAT16' || \
  die "partition is not FAT16"
printf '%s\n' "$MINFO" | grep -Eq 'hidden sectors: 2048' || \
  die "FAT16 partition does not report 2048 hidden sectors"

for file in KERNEL.SYS COMMAND.COM CONFIG.SYS AUTOEXEC.BAT; do
  MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mdir -i "$IMAGE@@1048576" "::$file" \
    >/dev/null 2>&1 || die "image lacks $file"
done

printf 'base image OK: %s\n' "$IMAGE"
