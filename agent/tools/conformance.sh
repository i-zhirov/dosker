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

if [ "$#" -gt 1 ]; then
  printf 'usage: conformance.sh [TEST_NAME]\n' >&2
  exit 2
fi

TEST_NAME="${1:-qemu_handshake}"
IMAGE="$ROOT/agent/fixtures/dosker-agent.img"

if [ -z "${DOSKER_SKIP_BUILD:-}" ]; then
  "$ROOT/agent/build.sh"
fi

export DOSKER_INTEGRATION=1
export DOSKER_AGENT_IMAGE="$IMAGE"
export DOSKER_QEMU="${DOSKER_QEMU:-qemu-system-i386}"
export DOSKER_QEMU_BOOT_TIMEOUT_MS="${DOSKER_QEMU_BOOT_TIMEOUT_MS:-60000}"

"$ROOT/agent/tools/build-fixture.sh" "$IMAGE"

cd "$ROOT"
cargo test -p dosker-agent-proto --test "$TEST_NAME" -- --nocapture
