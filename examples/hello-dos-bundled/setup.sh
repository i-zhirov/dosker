#!/bin/sh
#
# Prepare the hello-dos-bundled example.
#
# Source this script from the repository:
#   . examples/hello-dos-bundled/setup.sh
#
# The script prepares the shared DOS environment, builds a self-contained
# image from COPY and RUN, and does not run the example.

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
        if [ -f "$dir/Cargo.toml" ] \
            && [ -f "$dir/examples/hello-dos-bundled/Doskerfile" ]; then
            printf '%s\n' "$dir"
            return 0
        fi
        dir=$(dirname "$dir")
    done
    return 1
}

if ! setup_is_sourced; then
    printf 'setup: source this script instead of executing it:\n' >&2
    printf '  . examples/hello-dos-bundled/setup.sh\n' >&2
    exit 1
fi

repo_root=$(setup_repo_root) || setup_error "run this script from the repository" || return 1

export DOSKER_HELLO_SETUP_ONLY=1
. "$repo_root/examples/hello-dos/setup.sh" \
    || {
        unset DOSKER_HELLO_SETUP_ONLY
        setup_error "cannot prepare the shared DOS environment"
        return 1
    }
unset DOSKER_HELLO_SETUP_ONLY

build_context="$DOSKER_ROOT/hello-dos-bundled"
rm -rf "$build_context" || setup_error "cannot clean $build_context" || return 1
mkdir -p "$build_context" || setup_error "cannot create $build_context" || return 1
for file in Doskerfile Doskerfile.lock HELLO.TXT NOTES.TXT UPLOAD.TXT; do
    cp "$repo_root/examples/hello-dos-bundled/$file" "$build_context/$file" \
        || setup_error "cannot copy $file" || return 1
done

printf 'setup: building hello-bundled:latest with COPY and RUN\n'
(cd "$repo_root" && dosker build --update -t hello-bundled:latest "$build_context") \
    || setup_error "cannot build hello-bundled:latest" || return 1

printf '\nsetup complete\n'
printf '  DOSKER_ROOT=%s\n' "$DOSKER_ROOT"
printf '  DOSKER_AGENT_EXE=%s\n' "$DOSKER_AGENT_EXE"
printf '  DOSKER_FREEDOS_SOURCE=%s\n' "$DOSKER_FREEDOS_SOURCE"
printf '  DOSKER_QEMU=%s\n' "$DOSKER_QEMU"
printf '  dosker=%s\n' "$(command -v dosker)"
printf '\nReady to run:\n'
printf '  dosker run -d --name hello-bundled-demo --boot-timeout 120 hello-bundled:latest\n'
printf '\nThen verify the image tree:\n'
printf "  dosker exec hello-bundled-demo 'TYPE C:\\\\APP\\\\HELLO.TXT'\n"
printf "  dosker exec hello-bundled-demo 'TYPE C:\\\\APP\\\\DATA.TXT'\n"
printf '\nThe image has no volume and no runtime file copy.\n'
