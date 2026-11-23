#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

for extra in /opt/homebrew/opt/util-linux/sbin /opt/homebrew/opt/coreutils/libexec/gnubin; do
    if [ -d "$extra" ]; then
        PATH="$extra:$PATH"
    fi
done
export PATH

OUTPUT="${1:-}"
SOURCE="${DOSKER_FREEDOS_SOURCE:-}"
QEMU="${DOSKER_QEMU:-qemu-system-i386}"
BOOT_TIMEOUT_SECONDS="${DOSKER_BUNDLED_BOOT_TIMEOUT_SECONDS:-120}"
VOLUME_NAME="openwatcom-bundled-disk"
PARTITION_OFFSET=1048576

die() {
    printf 'build-disk: %s\n' "$*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"
}

[ -n "$OUTPUT" ] || die "usage: build-disk.sh <output.img>"
[ -n "$SOURCE" ] || die "DOSKER_FREEDOS_SOURCE is not set"
[ -f "$SOURCE" ] || die "DOSKER_FREEDOS_SOURCE is not a file: $SOURCE"
[ -n "$QEMU" ] || die "DOSKER_QEMU is empty"
command -v "$QEMU" >/dev/null 2>&1 || die "missing QEMU executable: $QEMU"
command -v dosker >/dev/null 2>&1 || die "dosker is not on PATH"

for tool in mcopy mdir dd; do
    need "$tool"
done

case "$BOOT_TIMEOUT_SECONDS" in
    ''|*[!0-9]*) die "DOSKER_BUNDLED_BOOT_TIMEOUT_SECONDS must be a non-negative integer" ;;
esac
[ "$BOOT_TIMEOUT_SECONDS" -gt 0 ] || die "boot timeout must be greater than zero"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/dosker-bundled-disk.XXXXXX")"
OWN_ROOT=0
QEMU_PID=""

cleanup() {
    if [ -n "$QEMU_PID" ] && kill -0 "$QEMU_PID" 2>/dev/null; then
        kill "$QEMU_PID" 2>/dev/null || true
        wait "$QEMU_PID" 2>/dev/null || true
    fi
    if [ "${DOSKER_ROOT:-}" != "" ]; then
        dosker volume rm "$VOLUME_NAME" >/dev/null 2>&1 || true
    fi
    if [ "$OWN_ROOT" -eq 1 ] && [ "${DOSKER_ROOT:-}" != "" ]; then
        rm -rf "$DOSKER_ROOT"
    fi
    rm -rf "$TMP"
}
trap cleanup EXIT

if [ -z "${DOSKER_ROOT:-}" ]; then
    DOSKER_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/dosker-bundled-root.XXXXXX")"
    export DOSKER_ROOT
    OWN_ROOT=1
fi

SOURCE_IMG="$TMP/source.img"
DISK_IMG="$DOSKER_ROOT/volumes/$VOLUME_NAME/volume.img"
SERIAL_LOG="$TMP/serial.log"

cp "$SOURCE" "$SOURCE_IMG"

mtools_source_mdir() {
    MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mdir -i "$SOURCE_IMG" "$1"
}

mtools_source_mcopy() {
    MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mcopy -o -i "$SOURCE_IMG" "$@"
}

mtools_disk_mdir() {
    MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mdir \
        -i "$DISK_IMG@@$PARTITION_OFFSET" "$1"
}

mtools_disk_mcopy() {
    MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mcopy -o \
        -i "$DISK_IMG@@$PARTITION_OFFSET" "$@"
}

source_file_present() {
    mtools_source_mdir "::$1" >/dev/null 2>&1
}

require_source_file() {
    source_file_present "$1" || die "FreeDOS source floppy lacks $1"
}

copy_to_source_root() {
    local source_path="$1"
    local root_name="$2"

    if source_file_present "$root_name"; then
        return 0
    fi
    if source_file_present "$source_path"; then
        mtools_source_mcopy "::$source_path" "::$root_name"
        return 0
    fi
    die "FreeDOS source floppy lacks $source_path (needed as $root_name)"
}

require_source_file "KERNEL.SYS"
copy_to_source_root "FREEDOS/BIN/FDISK.EXE" "FDISK.EXE"
copy_to_source_root "FREEDOS/BIN/SYS.COM" "SYS.COM"
copy_to_source_root "FREEDOS/BIN/COMMAND.COM" "COMMAND.COM"

{
    printf 'FILES=40\r\n'
    printf 'BUFFERS=20\r\n'
    printf 'LASTDRIVE=Z\r\n'
} > "$TMP/config.sys"

{
    printf '@ECHO OFF\r\n'
    printf 'ECHO OPENWATCOM DISK READY > COM1\r\n'
} > "$TMP/autoexec.bat"

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
    printf 'ECHO OPENWATCOM DISK READY > COM1\r\n'
    printf 'GOTO END\r\n'
    printf ':FAILED\r\n'
    printf 'ECHO OPENWATCOM DISK FAILED > COM1\r\n'
    printf ':END\r\n'
} > "$TMP/fdauto.bat"

mtools_source_mcopy "$TMP/config.sys" "::/CONFIG.SYS"
mtools_source_mcopy "$TMP/autoexec.bat" "::/AUTOEXEC.BAT"
mtools_source_mcopy "$TMP/fdauto.bat" "::/FDAUTO.BAT"
mtools_source_mcopy "$TMP/fdauto.bat" "::/AUTOEXEC.BAT"

printf 'build-disk: creating %s\n' "$VOLUME_NAME"
dosker volume rm "$VOLUME_NAME" >/dev/null 2>&1 || true
dosker volume create --size 256M "$VOLUME_NAME" >/dev/null

[ -f "$DISK_IMG" ] || die "volume image was not created at $DISK_IMG"

printf '\200' | dd of="$DISK_IMG" bs=1 seek=446 conv=notrunc 2>/dev/null

wait_for_marker() {
    local marker="$1"
    shift
    local deadline=$((SECONDS + BOOT_TIMEOUT_SECONDS))

    : > "$SERIAL_LOG"
    "$QEMU" \
        -machine pc \
        -cpu 486 \
        -m 16 \
        -no-reboot \
        -display none \
        -serial "file:$SERIAL_LOG" \
        -monitor none \
        -accel tcg \
        "$@" >"$TMP/qemu.log" 2>&1 &
    QEMU_PID=$!

    while [ "$SECONDS" -lt "$deadline" ]; do
        if grep -aq "$marker" "$SERIAL_LOG"; then
            if kill -0 "$QEMU_PID" 2>/dev/null; then
                kill "$QEMU_PID" 2>/dev/null || true
            fi
            wait "$QEMU_PID" 2>/dev/null || true
            QEMU_PID=""
            return 0
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

    printf '%s\n' '--- serial log ---' >&2
    cat "$SERIAL_LOG" >&2 || true
    printf '%s\n' '--- QEMU log ---' >&2
    cat "$TMP/qemu.log" >&2 || true
    die "guest did not report $marker"
}

printf 'build-disk: installing FreeDOS boot code\n'
wait_for_marker 'OPENWATCOM DISK READY' \
    -drive "file=$SOURCE_IMG,format=raw,if=floppy,index=0" \
    -drive "file=$DISK_IMG,format=raw,if=ide,index=0" \
    -boot a

for file in KERNEL.SYS COMMAND.COM CONFIG.SYS AUTOEXEC.BAT; do
    mtools_disk_mdir "::$file" >/dev/null 2>&1 \
        || die "generated disk lacks $file"
done

printf '\200' | dd of="$DISK_IMG" bs=1 seek=446 conv=notrunc 2>/dev/null
mtools_disk_mcopy "$TMP/autoexec.bat" "::/AUTOEXEC.BAT"

printf 'build-disk: booting the disk alone\n'
wait_for_marker 'OPENWATCOM DISK READY' \
    -drive "file=$DISK_IMG,format=raw,if=ide,index=0" \
    -boot c

mkdir -p "$(dirname "$OUTPUT")"
cp "$DISK_IMG" "$OUTPUT"
printf 'build-disk: wrote %s\n' "$OUTPUT"
