#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

mkdir -p agent/build

cc -std=c99 -Wall -Wextra -Werror -I agent/src -o agent/build/primitive-test \
  agent/tests/primitive_test.c \
  agent/src/crc32.c \
  agent/src/frame.c

./agent/build/primitive-test
