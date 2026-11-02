#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Homebrew installs mtools normally, but keeps util-linux and coreutils out of
# the default PATH. Add them when present so this script also runs on macOS.
for extra in /opt/homebrew/opt/util-linux/sbin /opt/homebrew/opt/coreutils/libexec/gnubin; do
  if [ -d "$extra" ]; then
    PATH="$extra:$PATH"
  fi
done
export PATH

IMAGE="${DOSKER_AGENT_TEST_IMAGE:-$ROOT/agent/fixtures/dosker-agent-test.img}"
OFFSET="${DOSKER_IMAGE_OFFSET:-1048576}"
QEMU="${DOSKER_QEMU:-qemu-system-i386}"
TIMEOUT_MS="${DOSKER_QEMU_BOOT_TIMEOUT_MS:-60000}"
LOG="$ROOT/agent/fixtures/dosker-agent-test-qemu.log"

die() {
  printf 'self-check: %s\n' "$*" >&2
  exit 1
}

case "$TIMEOUT_MS" in
  ''|*[!0-9]*) die "DOSKER_QEMU_BOOT_TIMEOUT_MS must be a non-negative integer" ;;
esac

if [ -z "${DOSKER_SKIP_BUILD:-}" ]; then
  "$ROOT/agent/build.sh"
fi

mkdir -p "$(dirname "$IMAGE")" "$(dirname "$LOG")"

DOSKER_FIXTURE_TEST=1 "$ROOT/agent/tools/build-fixture.sh" "$IMAGE"

if [ "$OFFSET" = "0" ]; then
  MOUNT="$IMAGE"
else
  MOUNT="$IMAGE@@$OFFSET"
fi

TIMEOUT_SECONDS=$(( (TIMEOUT_MS + 999) / 1000 ))

QEMU_ARGS=(
  -machine pc
  -cpu 486
  -m 16
  -display none
  -no-reboot
  -drive "file=$IMAGE,format=raw,if=ide,index=0"
  -serial none
  -serial none
  -device isa-debug-exit,iobase=0x501,iosize=0x04
)

set +e
if command -v timeout >/dev/null 2>&1; then
  timeout "$TIMEOUT_SECONDS" "$QEMU" "${QEMU_ARGS[@]}" >"$LOG" 2>&1
else
  "$QEMU" "${QEMU_ARGS[@]}" >"$LOG" 2>&1
fi
STATUS=$?
set -e

if [ "$STATUS" -eq 1 ]; then
  printf 'DOSKER SELF-TEST PASS\n'
  exit 0
fi

if [ "$STATUS" -eq 3 ]; then
  if ! MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 \
    mtype -i "$MOUNT" ::/DOSKER/TEST.LOG; then
    printf 'self-check: could not read TEST.LOG from %s\n' "$IMAGE" >&2
  fi
  exit 1
fi

printf 'self-check: unexpected QEMU exit code %s\n' "$STATUS" >&2
cat "$LOG" >&2
exit 1
