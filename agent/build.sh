#!/bin/sh
set -eu

AGENT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
SRC_DIR="$AGENT_DIR/src"
TEST_DIR="$AGENT_DIR/test"
BUILD_DIR="$AGENT_DIR/build"
ARTIFACT="$BUILD_DIR/DOSKER.EXE"
EXITCODE_ARTIFACT="$BUILD_DIR/EXITCODE.EXE"

usage() {
    printf 'usage: build.sh [--clean|--print-artifact]\n' >&2
}

case "${1:-}" in
    "")
        ;;
    --clean)
        rm -rf "$BUILD_DIR"
        exit 0
        ;;
    --print-artifact)
        printf '%s\n' "$ARTIFACT"
        exit 0
        ;;
    *)
        usage
        exit 2
        ;;
esac

if [ -n "${WATCOM:-}" ] && [ -f "$WATCOM/owsetenv.sh" ]; then
    # shellcheck disable=SC1090
    . "$WATCOM/owsetenv.sh"
fi

if ! command -v wcc >/dev/null 2>&1; then
    printf 'build.sh: wcc is not installed or not on PATH\n' >&2
    exit 1
fi
if ! command -v wlink >/dev/null 2>&1; then
    printf 'build.sh: wlink is not installed or not on PATH\n' >&2
    exit 1
fi

mkdir -p "$BUILD_DIR"

OBJECTS=""
for source in "$SRC_DIR"/*.c; do
    name=$(basename "$source" .c)
    object="$BUILD_DIR/$name.obj"

    wcc -0 -bt=dos -ml -zq -wx -we -i="$SRC_DIR" -fo="$object" "$source"

    if [ -z "$OBJECTS" ]; then
        OBJECTS="$object"
    else
        OBJECTS="$OBJECTS,$object"
    fi
done

wlink system dos name "$ARTIFACT" file "$OBJECTS" \
    option stack=8192 option map="$BUILD_DIR/DOSKER.MAP"

wcc -0 -bt=dos -ml -zq -wx -we -i="$SRC_DIR" \
    -fo="$BUILD_DIR/exitcode.obj" "$TEST_DIR/exitcode.c"

wlink system dos name "$EXITCODE_ARTIFACT" file "$BUILD_DIR/exitcode.obj" \
    option stack=8192

printf 'built %s\n' "$ARTIFACT"
