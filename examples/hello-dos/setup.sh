#!/bin/sh
#
# Prepare the hello-dos example environment.
#
# Source this script from the repository:
#   . examples/hello-dos/setup.sh
#
# The script prepares the DOS agent, FreeDOS source, QEMU path, data root,
# and dosker binary. It does not run the example.

setup_error() {
    printf 'setup: %s\n' "$*" >&2
    return 1
}

setup_is_sourced() {
    if [ -n "${BASH_SOURCE:-}" ] && [ "${BASH_SOURCE[0]:-}" != "$0" ]; then
        return 0
    fi
    case "${ZSH_EVAL_CONTEXT:-}" in
        *:file*) return 0 ;;
    esac
    case "$0" in
        *setup.sh) return 1 ;;
        *) return 0 ;;
    esac
}

setup_repo_root() {
    dir=$PWD
    while [ "$dir" != "/" ]; do
        if [ -f "$dir/Cargo.toml" ] && [ -f "$dir/examples/hello-dos/Doskerfile" ]; then
            printf '%s\n' "$dir"
            return 0
        fi
        dir=$(dirname "$dir")
    done
    return 1
}

if ! setup_is_sourced; then
    printf 'setup: source this script instead of executing it:\n' >&2
    printf '  . examples/hello-dos/setup.sh\n' >&2
    exit 1
fi

repo_root=$(setup_repo_root) || setup_error "run this script from the repository" || return 1
cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/dosker-hello-dos"
mkdir -p "$cache_root" || setup_error "cannot create $cache_root" || return 1

freedos_url='https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/distributions/1.3/official/FD13-FloppyEdition.zip'
freedos_image="$cache_root/freedos/x86BOOT.img"
if [ -f "$freedos_image" ]; then
    export DOSKER_FREEDOS_SOURCE="$freedos_image"
elif [ -n "${DOSKER_FREEDOS_SOURCE:-}" ]; then
    resolved=$(DOSKER_FREEDOS_SOURCE="$DOSKER_FREEDOS_SOURCE" \
        bash "$repo_root/.github/scripts/fetch-freedos-source.sh") \
        || setup_error "cannot resolve DOSKER_FREEDOS_SOURCE" || return 1
    export DOSKER_FREEDOS_SOURCE="$resolved"
else
    printf 'setup: fetching the FreeDOS source\n'
    resolved=$(DOSKER_FREEDOS_SOURCE="$freedos_url" \
        bash "$repo_root/.github/scripts/fetch-freedos-source.sh") \
        || setup_error "cannot fetch the FreeDOS source" || return 1
    mkdir -p "$(dirname "$freedos_image")" \
        || setup_error "cannot create $(dirname "$freedos_image")" || return 1
    cp "$resolved" "$freedos_image" \
        || setup_error "cannot cache the FreeDOS source" || return 1
    export DOSKER_FREEDOS_SOURCE="$freedos_image"
fi

if [ -n "${DOSKER_AGENT_EXE:-}" ]; then
    if [ ! -f "$DOSKER_AGENT_EXE" ]; then
        setup_error "DOSKER_AGENT_EXE does not exist: $DOSKER_AGENT_EXE" || return 1
    fi
elif [ -f "$repo_root/agent/build/DOSKER.EXE" ]; then
    export DOSKER_AGENT_EXE="$repo_root/agent/build/DOSKER.EXE"
elif [ -f "$cache_root/agent/DOSKER.EXE" ]; then
    export DOSKER_AGENT_EXE="$cache_root/agent/DOSKER.EXE"
else
    mkdir -p "$cache_root/agent" \
        || setup_error "cannot create $cache_root/agent" || return 1
    if command -v gh >/dev/null 2>&1; then
        printf 'setup: downloading the DOS agent from CI\n'
        run_id=$(gh run list --repo i-zhirov/dosker --workflow CI \
            --status success --limit 1 --json databaseId \
            --jq '.[0].databaseId' 2>/dev/null) || run_id=
        if [ -n "$run_id" ] && [ "$run_id" != "null" ]; then
            tmp=$(mktemp -d "${TMPDIR:-/tmp}/dosker-agent.XXXXXX") \
                || setup_error "cannot create a temporary directory" || return 1
            if gh run download "$run_id" --repo i-zhirov/dosker \
                --name dosker-agent-binaries --dir "$tmp" >/dev/null 2>&1; then
                cp "$tmp/DOSKER.EXE" "$cache_root/agent/DOSKER.EXE" \
                    || setup_error "cannot cache DOSKER.EXE" || return 1
                export DOSKER_AGENT_EXE="$cache_root/agent/DOSKER.EXE"
            fi
            rm -rf "$tmp"
        fi
    fi
    if [ -z "${DOSKER_AGENT_EXE:-}" ] \
        && command -v wcc >/dev/null 2>&1 \
        && command -v wlink >/dev/null 2>&1; then
        printf 'setup: building the DOS agent\n'
        (cd "$repo_root" && ./agent/build.sh) \
            || setup_error "cannot build the DOS agent" || return 1
        export DOSKER_AGENT_EXE="$repo_root/agent/build/DOSKER.EXE"
    fi
    if [ -z "${DOSKER_AGENT_EXE:-}" ]; then
        setup_error "cannot obtain DOSKER.EXE; run gh auth login, build the agent, or set DOSKER_AGENT_EXE" || return 1
    fi
fi

if [ -n "${DOSKER_QEMU:-}" ]; then
    if [ ! -x "$DOSKER_QEMU" ] && ! command -v "$DOSKER_QEMU" >/dev/null 2>&1; then
        setup_error "DOSKER_QEMU is not executable: $DOSKER_QEMU" || return 1
    fi
elif command -v qemu-system-i386 >/dev/null 2>&1; then
    export DOSKER_QEMU="$(command -v qemu-system-i386)"
else
    setup_error "qemu-system-i386 is not on PATH; install QEMU or set DOSKER_QEMU" || return 1
fi

if [ -n "${DOSKER_ROOT:-}" ]; then
    mkdir -p "$DOSKER_ROOT" || setup_error "cannot create DOSKER_ROOT" || return 1
else
    export DOSKER_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/dosker-hello-dos.XXXXXX")" \
        || setup_error "cannot create a temporary data root" || return 1
fi

if command -v dosker >/dev/null 2>&1; then
    :
elif [ -x "$repo_root/target/debug/dosker" ]; then
    export PATH="$repo_root/target/debug:$PATH"
elif [ -x "$repo_root/target/release/dosker" ]; then
    export PATH="$repo_root/target/release:$PATH"
elif command -v cargo >/dev/null 2>&1; then
    printf 'setup: building dosker\n'
    (
        if [ "$(uname -s)" = "Darwin" ] && command -v xcrun >/dev/null 2>&1; then
            sdkroot=$(xcrun --show-sdk-path)
            export SDKROOT="$sdkroot"
            export LIBRARY_PATH="$sdkroot/usr/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
        fi
        cd "$repo_root" && cargo build -q -p dosker-cli \
            --features qemu-engine --bin dosker
    ) || setup_error "cannot build dosker" || return 1
    export PATH="$repo_root/target/debug:$PATH"
else
    setup_error "dosker is not on PATH and cargo is not available; install dosker or Rust" || return 1
fi

build_context="$DOSKER_ROOT/hello-dos"
mkdir -p "$build_context" || setup_error "cannot create $build_context" || return 1
cp "$repo_root/examples/hello-dos/Doskerfile" "$build_context/Doskerfile" \
    || setup_error "cannot copy the Doskerfile" || return 1
if [ -f "$repo_root/examples/hello-dos/Doskerfile.lock" ]; then
    cp "$repo_root/examples/hello-dos/Doskerfile.lock" "$build_context/Doskerfile.lock" \
        || setup_error "cannot copy the lock file" || return 1
fi

printf 'setup: building hello:latest\n'
(cd "$repo_root" && dosker build --update -t hello:latest "$build_context") \
    || setup_error "cannot build hello:latest" || return 1

printf 'setup: creating the hello-data volume\n'
if ! dosker volume create --size 64M hello-data; then
    if dosker volume ls | grep -q '^hello-data '; then
        printf 'setup: using the existing hello-data volume\n'
    else
        setup_error "cannot create the hello-data volume" || return 1
    fi
fi

printf '\nsetup complete\n'
printf '  DOSKER_ROOT=%s\n' "$DOSKER_ROOT"
printf '  DOSKER_AGENT_EXE=%s\n' "$DOSKER_AGENT_EXE"
printf '  DOSKER_FREEDOS_SOURCE=%s\n' "$DOSKER_FREEDOS_SOURCE"
printf '  DOSKER_QEMU=%s\n' "$DOSKER_QEMU"
printf '  dosker=%s\n' "$(command -v dosker)"
printf '\nReady to run:\n'
printf '  dosker run -d --name hello-demo --boot-timeout 120 -v hello-data:D: -p 8080:80 hello:latest\n'
printf '\nThen use dosker ps, dosker logs hello-demo, and dosker exec.\n'
