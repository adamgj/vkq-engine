#!/bin/sh
# Produce the compile_commands.json c2rust consumes. Meson emits it for free.
#
# The translation targets (pr_exec.c, mathlib.c, world.c, ...) left Quake/
# with the Phase 9 deletion PR; the C text now lives only at the tag
# c-reference/final (ADR-019). Check that tag out into a worktree (CREF, default
# <repo>/build-cref-src) and configure its C-only build there.
set -e
cd "$(dirname "$0")/../.."
CREF=${CREF:-$PWD/build-cref-src}
[ -d "$CREF" ] || git worktree add --detach "$CREF" c-reference/final
cd "$CREF"
meson setup builddir-cc -Duse_rust=disabled --reconfigure 2>/dev/null || meson setup builddir-cc -Duse_rust=disabled
cd - >/dev/null
cp "$CREF/builddir-cc/compile_commands.json" tools/c2rust-oracle/compile_commands.json
echo "wrote tools/c2rust-oracle/compile_commands.json"
