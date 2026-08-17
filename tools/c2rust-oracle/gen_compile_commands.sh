#!/bin/sh
# Produce the compile_commands.json c2rust consumes. Meson emits it for free.
set -e
cd "$(dirname "$0")/../.."
meson setup builddir-cc -Duse_rust=disabled --reconfigure 2>/dev/null || meson setup builddir-cc -Duse_rust=disabled
cp builddir-cc/compile_commands.json tools/c2rust-oracle/compile_commands.json
echo "wrote tools/c2rust-oracle/compile_commands.json"
