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

OUTPUT="${1:-${DOSKER_BASE_IMAGE:-$ROOT/agent/fixtures/freedos-base.img}}"
SOURCE="${DOSKER_FREEDOS_SOURCE:-}"
QEMU="${DOSKER_QEMU:-qemu-system-i386}"
BOOT_TIMEOUT_MS="${DOSKER_QEMU_BOOT_TIMEOUT_MS:-60000}"

die() {
  printf 'build-base-image: %s\n' "$*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"
}

[ -n "$SOURCE" ] || die "DOSKER_FREEDOS_SOURCE is not set"
[ -f "$SOURCE" ] || die "DOSKER_FREEDOS_SOURCE is not a file: $SOURCE"
[ -n "$QEMU" ] || die "DOSKER_QEMU is empty"
command -v "$QEMU" >/dev/null 2>&1 || die "missing QEMU executable: $QEMU"

for tool in fdisk sfdisk mformat mcopy mdir dd; do
  need "$tool"
done

if command -v timeout >/dev/null 2>&1; then
  TIMEOUT=timeout
elif command -v gtimeout >/dev/null 2>&1; then
  TIMEOUT=gtimeout
else
  die "missing required tool: timeout (or gtimeout)"
fi

case "$BOOT_TIMEOUT_MS" in
  ''|*[!0-9]*) die "DOSKER_QEMU_BOOT_TIMEOUT_MS must be a non-negative integer" ;;
esac
TIMEOUT_SECONDS=$(( (BOOT_TIMEOUT_MS + 999) / 1000 ))
[ "$TIMEOUT_SECONDS" -gt 0 ] || TIMEOUT_SECONDS=1

TMP="$(mktemp -d "${TMPDIR:-/tmp}/dosker-base.XXXXXX")"
QEMU_PID=""

cleanup() {
  if [ -n "$QEMU_PID" ] && kill -0 "$QEMU_PID" 2>/dev/null; then
    kill "$QEMU_PID" 2>/dev/null || true
    wait "$QEMU_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

SOURCE_IMG="$TMP/source.img"
DISK_IMG="$TMP/disk.img"
SERIAL_LOG="$TMP/serial.log"

cp "$SOURCE" "$SOURCE_IMG"

mtools_mdir() {
  MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mdir -i "$1" "$2"
}

mtools_mcopy() {
  MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mcopy -o -i "$SOURCE_IMG" "$@"
}

file_present() {
  mtools_mdir "$SOURCE_IMG" "::$1" >/dev/null 2>&1
}

require_source_file() {
  file_present "$1" || die "FreeDOS source floppy lacks $1"
}

copy_to_source_root() {
  local source_path="$1"
  local root_name="$2"

  if file_present "$root_name"; then
    return 0
  fi

  if file_present "$source_path"; then
    mtools_mcopy "::$source_path" "::$root_name"
    return 0
  fi

  die "FreeDOS source floppy lacks $source_path (needed as $root_name)"
}

require_source_file "KERNEL.SYS"
copy_to_source_root "FREEDOS/BIN/FDISK.EXE" "FDISK.EXE"
copy_to_source_root "FREEDOS/BIN/SYS.COM" "SYS.COM"
copy_to_source_root "FREEDOS/BIN/COMMAND.COM" "COMMAND.COM"

# A minimal target configuration. The target gets its own CONFIG.SYS and a
# stub AUTOEXEC.BAT; Task 6 replaces the stub with the agent launcher.
{
  printf 'FILES=40\r\n'
  printf 'BUFFERS=20\r\n'
  printf 'LASTDRIVE=Z\r\n'
} > "$TMP/config.sys"

printf '@ECHO OFF\r\n' > "$TMP/autoexec.bat"

# The FreeDOS Floppy Edition boots FDCONFIG.SYS with \FDAUTO.BAT as its shell
# startup file. Write the build batch to both startup names so the source also
# works with a floppy that uses AUTOEXEC.BAT instead.
{
  printf '@ECHO OFF\r\n'
  printf 'ECHO Y|A:\\FDISK.EXE /MBR\r\n'
  printf 'IF ERRORLEVEL 1 GOTO FAILED\r\n'
  printf 'ECHO Y|A:\\SYS.COM C:\r\n'
  printf 'IF ERRORLEVEL 1 GOTO FAILED\r\n'
  printf 'COPY A:\\COMMAND.COM C:\\\r\n'
  printf 'IF ERRORLEVEL 1 GOTO FAILED\r\n'
  printf 'COPY A:\\CONFIG.SYS C:\\\r\n'
  printf 'IF ERRORLEVEL 1 GOTO FAILED\r\n'
  printf 'COPY A:\\AUTOEXEC.BAT C:\\\r\n'
  printf 'IF ERRORLEVEL 1 GOTO FAILED\r\n'
  printf 'ECHO DOSKER BASE READY > COM1\r\n'
  printf 'GOTO END\r\n'
  printf ':FAILED\r\n'
  printf 'ECHO DOSKER BASE FAILED > COM1\r\n'
  printf ':END\r\n'
} > "$TMP/fdauto.bat"

mtools_mcopy "$TMP/config.sys" "::/CONFIG.SYS"
mtools_mcopy "$TMP/fdauto.bat" "::/FDAUTO.BAT"
mtools_mcopy "$TMP/fdauto.bat" "::/AUTOEXEC.BAT"

truncate -s 32M "$DISK_IMG"

# Use fdisk rather than sfdisk for the geometry: current util-linux sfdisk no
# longer accepts -H/-S, while fdisk accepts -C/-H/-S and writes the CHS values
# that match the 16-head, 32-sector FreeDOS BPB.
fdisk -C 128 -H 16 -S 32 "$DISK_IMG" >/dev/null 2>&1 <<'EOF'
o
n
p
1
2048
65535
a
t
6
w
EOF

mformat -i "$DISK_IMG@@1048576" -t 124 -h 16 -s 32 -H 2048 -v DOSKER ::

: > "$SERIAL_LOG"
"$TIMEOUT" "$TIMEOUT_SECONDS" "$QEMU" \
  -machine pc \
  -cpu 486 \
  -m 16 \
  -no-reboot \
  -drive "file=$SOURCE_IMG,format=raw,if=floppy,index=0" \
  -drive "file=$DISK_IMG,format=raw,if=ide,index=0" \
  -boot a \
  -display none \
  -serial "file:$SERIAL_LOG" \
  -monitor none \
  -accel tcg \
  >"$TMP/qemu.log" 2>&1 &
QEMU_PID=$!

READY=0
DEADLINE=$((SECONDS + TIMEOUT_SECONDS))
while [ "$SECONDS" -lt "$DEADLINE" ]; do
  if grep -aq 'DOSKER BASE READY' "$SERIAL_LOG"; then
    READY=1
    break
  fi
  if grep -aq 'DOSKER BASE FAILED' "$SERIAL_LOG"; then
    break
  fi
  if ! kill -0 "$QEMU_PID" 2>/dev/null; then
    break
  fi
  sleep 0.1
done

if kill -0 "$QEMU_PID" 2>/dev/null; then
  kill "$QEMU_PID" 2>/dev/null || true
fi
wait "$QEMU_PID" 2>/dev/null || true
QEMU_PID=""

if [ "$READY" -ne 1 ]; then
  printf '%s\n' '--- serial log ---' >&2
  cat "$SERIAL_LOG" >&2 || true
  printf '%s\n' '--- QEMU log ---' >&2
  cat "$TMP/qemu.log" >&2 || true
  die "FreeDOS base image build did not report DOSKER BASE READY"
fi

for file in KERNEL.SYS COMMAND.COM CONFIG.SYS AUTOEXEC.BAT; do
  mtools_mdir "$DISK_IMG@@1048576" "::$file" >/dev/null 2>&1 || \
    die "generated image lacks $file"
done

# Do not rerun the build batch on every target boot.
MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mcopy -o -i "$DISK_IMG@@1048576" \
  "$TMP/autoexec.bat" "::/AUTOEXEC.BAT"

mkdir -p "$(dirname "$OUTPUT")"
cp "$DISK_IMG" "$OUTPUT"
printf 'built %s\n' "$OUTPUT"
