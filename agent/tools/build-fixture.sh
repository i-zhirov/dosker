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

if [ "$#" -lt 1 ] || [ -z "${1:-}" ]; then
  printf 'usage: build-fixture.sh OUTPUT_IMAGE\n' >&2
  exit 1
fi

OUTPUT="$1"
BASE="${DOSKER_BASE_IMAGE:-$ROOT/agent/fixtures/freedos-base.img}"
AGENT="${DOSKER_AGENT_EXE:-$ROOT/agent/build/DOSKER.EXE}"
EXITCODE="${DOSKER_EXITCODE_EXE:-$ROOT/agent/build/EXITCODE.EXE}"
OFFSET="${DOSKER_IMAGE_OFFSET:-1048576}"
SELF_TEST="${DOSKER_FIXTURE_TEST:-0}"

die() {
  printf 'build-fixture: %s\n' "$*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"
}

[ -f "$BASE" ] || \
  die "missing base image: $BASE (build it with agent/tools/build-base-image.sh)"
[ -f "$AGENT" ] || die "missing agent executable: $AGENT"
[ -f "$EXITCODE" ] || die "missing exit-code helper: $EXITCODE"

for tool in mcopy mmd mdir mtype mformat; do
  need "$tool"
done

case "$OFFSET" in
  ''|*[!0-9]*) die "DOSKER_IMAGE_OFFSET must be a non-negative integer" ;;
esac

TMP="$(mktemp -d "${TMPDIR:-/tmp}/dosker-fixture.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$(dirname "$OUTPUT")"
cp "$BASE" "$OUTPUT"

IMAGE="$OUTPUT"
if [ "$OFFSET" != "0" ]; then
  IMAGE="$OUTPUT@@$OFFSET"
fi

mtools_mcopy() {
  MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mcopy -o -i "$IMAGE" "$@"
}

mtools_mmd() {
  MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mmd -i "$IMAGE" "$@"
}

mtools_mdir() {
  MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mdir -i "$IMAGE" "$@"
}

if ! mtools_mdir ::/DOSKER >/dev/null 2>&1; then
  mtools_mmd ::/DOSKER
fi
if ! mtools_mdir ::/APP >/dev/null 2>&1; then
  mtools_mmd ::/APP
fi

# BIG.TXT is a known-size file for GET and STAT checks.
printf 'X%.0s' {1..2048} > "$TMP/BIG.TXT"

printf '%s\r\n' 'ONE' 'TWO' > "$TMP/TWO.TXT"

printf '%s\r\n' 'CONTROL_PORT=2' 'BAUD=115200' > "$TMP/CONFIG.INI"

if [ "$SELF_TEST" = "1" ]; then
  printf '%s\r\n' '@ECHO OFF' 'C:\DOSKER\DOSKER.EXE /TEST' \
    > "$TMP/AUTOEXEC.BAT"
else
  printf '%s\r\n' '@ECHO OFF' 'SET COMSPEC=C:\COMMAND.COM' \
    'C:\DOSKER\DOSKER.EXE' > "$TMP/AUTOEXEC.BAT"
fi

mtools_mcopy "$AGENT" ::/DOSKER/DOSKER.EXE
mtools_mcopy "$EXITCODE" ::/DOSKER/EXITCODE.EXE
mtools_mcopy "$TMP/BIG.TXT" ::/DOSKER/BIG.TXT
mtools_mcopy "$TMP/TWO.TXT" ::/DOSKER/TWO.TXT
mtools_mcopy "$TMP/CONFIG.INI" ::/DOSKER/CONFIG.INI
mtools_mcopy "$TMP/AUTOEXEC.BAT" ::/AUTOEXEC.BAT

CONFIG_SYS="$TMP/CONFIG.SYS"
if mtools_mdir ::/CONFIG.SYS >/dev/null 2>&1; then
  mtools_mcopy ::/CONFIG.SYS "$CONFIG_SYS"
else
  : > "$CONFIG_SYS"
fi

config_has() {
  grep -Eiq "^[[:space:]]*$1[[:space:]]*=" "$CONFIG_SYS"
}

config_ends_with_newline() {
  [ -s "$CONFIG_SYS" ] || return 0

  local last_byte
  last_byte="$(tail -c 1 "$CONFIG_SYS" | od -An -t u1 | tr -d '[:space:]')"

  [ "$last_byte" = "10" ]
}

needs_separator=0
if ! config_ends_with_newline; then
  needs_separator=1
fi

for setting in 'FILES=20' 'BUFFERS=10' 'LASTDRIVE=Z'; do
  key="${setting%%=*}"
  if ! config_has "$key"; then
    if [ "$needs_separator" -eq 1 ]; then
      printf '\r\n' >> "$CONFIG_SYS"
      needs_separator=0
    fi
    printf '%s\r\n' "$setting" >> "$CONFIG_SYS"
  fi
done

mtools_mcopy "$CONFIG_SYS" ::/CONFIG.SYS

mtools_mdir ::/DOSKER

printf 'built %s\n' "$OUTPUT"
