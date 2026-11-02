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

BUILDER="$ROOT/agent/tools/build-fixture.sh"

die() {
  printf 'fixture-test: %s\n' "$*" >&2
  exit 1
}

for tool in dd mformat mdir mtype mcopy mmd od; do
  command -v "$tool" >/dev/null 2>&1 || die "missing required tool: $tool"
done

TMP="$(mktemp -d "${TMPDIR:-/tmp}/dosker-fixture-test.XXXXXX")"

cleanup() {
  local status=$?

  if [ "$status" -ne 0 ]; then
    printf 'fixture-test: diagnostics kept in %s\n' "$TMP" >&2
    return
  fi

  rm -rf "$TMP"
}
trap cleanup EXIT

BLANK="$TMP/blank.img"
FIXTURE="$TMP/fixture.img"
AGENT="$TMP/DOSKER.EXE"
EXITCODE="$TMP/EXITCODE.EXE"

dd if=/dev/zero of="$BLANK" bs=1048576 count=8 2>/dev/null
MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mformat -i "$BLANK" ::

printf 'dummy agent\n' > "$AGENT"
printf 'dummy exit-code helper\n' > "$EXITCODE"

mtools_mdir() {
  MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mdir "$@"
}

mtools_mtype() {
  MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mtype "$@"
}

mtools_mcopy() {
  MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mcopy "$@"
}

check_contains() {
  local content="$1"
  local needle="$2"
  local message="$3"

  printf '%s\n' "$content" | grep -Fq "$needle" || die "$message"
}

check_line() {
  local content="$1"
  local line="$2"
  local message="$3"

  printf '%s\n' "$content" | tr -d '\r' | grep -Fxq "$line" || die "$message"
}

run_expect_failure() {
  local log="$1"
  shift

  set +e
  "$@" >"$log" 2>&1
  local status=$?
  set -e

  printf '%s' "$status"
}

DOSKER_BASE_IMAGE="$BLANK" \
DOSKER_AGENT_EXE="$AGENT" \
DOSKER_EXITCODE_EXE="$EXITCODE" \
DOSKER_IMAGE_OFFSET=0 \
DOSKER_FIXTURE_TEST=0 \
  "$BUILDER" "$FIXTURE" >"$TMP/build.log"

[ -f "$FIXTURE" ] || die "builder did not create $FIXTURE"

mtools_mdir -i "$FIXTURE" ::/APP >/dev/null 2>&1 || \
  die "fixture lacks ::/APP"

LISTING="$(mtools_mdir -i "$FIXTURE" ::/DOSKER)"
for name in DOSKER.EXE EXITCODE.EXE BIG.TXT TWO.TXT CONFIG.INI; do
  printf '%s\n' "$LISTING" | grep -Fq "$name" || \
    die "fixture listing lacks $name"
done

mtools_mtype -i "$FIXTURE" ::/DOSKER/BIG.TXT > "$TMP/BIG.TXT"
BIG_SIZE="$(wc -c < "$TMP/BIG.TXT" | tr -d '[:space:]')"
[ "$BIG_SIZE" = "2048" ] || \
  die "BIG.TXT is $BIG_SIZE bytes, expected 2048"

TWO="$(mtools_mtype -i "$FIXTURE" ::/DOSKER/TWO.TXT)"
check_contains "$TWO" 'ONE' "TWO.TXT lacks ONE"
check_contains "$TWO" 'TWO' "TWO.TXT lacks TWO"

CONFIG="$(mtools_mtype -i "$FIXTURE" ::/DOSKER/CONFIG.INI)"
check_contains "$CONFIG" 'CONTROL_PORT=2' "CONFIG.INI lacks CONTROL_PORT=2"
check_contains "$CONFIG" 'BAUD=115200' "CONFIG.INI lacks BAUD=115200"

AUTOEXEC="$(mtools_mtype -i "$FIXTURE" ::/AUTOEXEC.BAT)"
check_contains "$AUTOEXEC" '@ECHO OFF' "AUTOEXEC.BAT lacks @ECHO OFF"
check_contains "$AUTOEXEC" 'SET COMSPEC=C:\COMMAND.COM' \
  "AUTOEXEC.BAT lacks SET COMSPEC=C:\COMMAND.COM"
check_contains "$AUTOEXEC" 'C:\DOSKER\DOSKER.EXE' \
  "AUTOEXEC.BAT lacks the agent launcher"
if printf '%s\n' "$AUTOEXEC" | grep -Fq '/TEST'; then
  die "normal AUTOEXEC.BAT contains the self-check flag"
fi

CONFIG_SYS="$(mtools_mtype -i "$FIXTURE" ::/CONFIG.SYS)"
for setting in 'FILES=20' 'BUFFERS=10' 'LASTDRIVE=Z'; do
  check_line "$CONFIG_SYS" "$setting" "CONFIG.SYS lacks $setting"
done

TEST_FIXTURE="$TMP/fixture-test.img"
DOSKER_BASE_IMAGE="$BLANK" \
DOSKER_AGENT_EXE="$AGENT" \
DOSKER_EXITCODE_EXE="$EXITCODE" \
DOSKER_IMAGE_OFFSET=0 \
DOSKER_FIXTURE_TEST=1 \
  "$BUILDER" "$TEST_FIXTURE" >"$TMP/build-test.log"

TEST_AUTOEXEC="$(mtools_mtype -i "$TEST_FIXTURE" ::/AUTOEXEC.BAT)"
check_contains "$TEST_AUTOEXEC" 'C:\DOSKER\DOSKER.EXE /TEST' \
  "self-check AUTOEXEC.BAT lacks the /TEST launcher"

CONFIG_BLANK="$TMP/blank-config.img"
CONFIG_FIXTURE="$TMP/fixture-config.img"

dd if=/dev/zero of="$CONFIG_BLANK" bs=1048576 count=8 2>/dev/null
MTOOLS_SKIP_CHECK=1 MTOOLS_NO_VFAT=1 mformat -i "$CONFIG_BLANK" ::

printf '%s' 'SHELL=C:\COMMAND.COM' > "$TMP/CONFIG.SYS.source"
mtools_mcopy -i "$CONFIG_BLANK" "$TMP/CONFIG.SYS.source" ::/CONFIG.SYS

DOSKER_BASE_IMAGE="$CONFIG_BLANK" \
DOSKER_AGENT_EXE="$AGENT" \
DOSKER_EXITCODE_EXE="$EXITCODE" \
DOSKER_IMAGE_OFFSET=0 \
DOSKER_FIXTURE_TEST=0 \
  "$BUILDER" "$CONFIG_FIXTURE" >"$TMP/build-config.log"

CONFIG_SYS_CONTENT="$(mtools_mtype -i "$CONFIG_FIXTURE" ::/CONFIG.SYS)"
check_line "$CONFIG_SYS_CONTENT" 'SHELL=C:\COMMAND.COM' \
  "existing CONFIG.SYS lost its first line"
for setting in 'FILES=20' 'BUFFERS=10' 'LASTDRIVE=Z'; do
  check_line "$CONFIG_SYS_CONTENT" "$setting" \
    "existing CONFIG.SYS lacks $setting on its own line"
done
if printf '%s\n' "$CONFIG_SYS_CONTENT" | \
  grep -Fq 'SHELL=C:\COMMAND.COMFILES=20'; then
  die "existing CONFIG.SYS settings were joined to the previous line"
fi

BASE_MISSING="$TMP/absent-base.img"
BASE_LOG="$TMP/missing-base.log"
BASE_STATUS="$(run_expect_failure "$BASE_LOG" \
  env \
  DOSKER_BASE_IMAGE="$BASE_MISSING" \
  DOSKER_AGENT_EXE="$AGENT" \
  DOSKER_EXITCODE_EXE="$EXITCODE" \
  DOSKER_IMAGE_OFFSET=0 \
  "$BUILDER" "$TMP/missing-base.img")"
[ "$BASE_STATUS" -eq 1 ] || \
  die "missing base image: expected exit code 1, got $BASE_STATUS"
grep -Fq "$BASE_MISSING" "$BASE_LOG" || \
  die "missing base image message does not name $BASE_MISSING"
grep -Fq 'agent/tools/build-base-image.sh' "$BASE_LOG" || \
  die "missing base image message does not name agent/tools/build-base-image.sh"

AGENT_MISSING="$TMP/absent-agent.exe"
AGENT_LOG="$TMP/missing-agent.log"
AGENT_STATUS="$(run_expect_failure "$AGENT_LOG" \
  env \
  DOSKER_BASE_IMAGE="$BLANK" \
  DOSKER_AGENT_EXE="$AGENT_MISSING" \
  DOSKER_EXITCODE_EXE="$EXITCODE" \
  DOSKER_IMAGE_OFFSET=0 \
  "$BUILDER" "$TMP/missing-agent.img")"
[ "$AGENT_STATUS" -eq 1 ] || \
  die "missing agent: expected exit code 1, got $AGENT_STATUS"
grep -Fq "$AGENT_MISSING" "$AGENT_LOG" || \
  die "missing agent message does not name $AGENT_MISSING"

EXITCODE_MISSING="$TMP/absent-exitcode.exe"
EXITCODE_LOG="$TMP/missing-exitcode.log"
EXITCODE_STATUS="$(run_expect_failure "$EXITCODE_LOG" \
  env \
  DOSKER_BASE_IMAGE="$BLANK" \
  DOSKER_AGENT_EXE="$AGENT" \
  DOSKER_EXITCODE_EXE="$EXITCODE_MISSING" \
  DOSKER_IMAGE_OFFSET=0 \
  "$BUILDER" "$TMP/missing-exitcode.img")"
[ "$EXITCODE_STATUS" -eq 1 ] || \
  die "missing exit-code helper: expected exit code 1, got $EXITCODE_STATUS"
grep -Fq "$EXITCODE_MISSING" "$EXITCODE_LOG" || \
  die "missing exit-code helper message does not name $EXITCODE_MISSING"

printf 'fixture builder OK\n'
