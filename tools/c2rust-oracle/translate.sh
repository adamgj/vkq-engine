#!/bin/sh
# Translate the Phase 0 oracle targets (pr_exec.c, mathlib.c, world.c).
# Requires c2rust on PATH (or run via docker, see README.md).
set -e
cd "$(dirname "$0")"
[ -f compile_commands.json ] || { echo "run gen_compile_commands.sh first" >&2; exit 1; }
c2rust transpile compile_commands.json --filter 'pr_exec|mathlib|world' --output-dir translated --emit-no-std no 2>/dev/null \
    || c2rust transpile compile_commands.json --filter 'pr_exec|mathlib|world' -o translated
echo "translations written to translated/ (check c2rust transpile --help if flags changed)"
