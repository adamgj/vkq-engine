#!/bin/sh
# Verify the Rust-migration "core" headers compile standalone, with no
# SDL or Vulkan on the include path (PLAN.md 4.1 exit criterion; these are
# the future bindgen roots).
set -e

cd "$(dirname "$0")/../.."

HEADERS="q_types.h q_minmax.h protocol.h modelgen.h spritegn.h bspfile.h pakfile.h wad.h common.h sys.h mem.h steam.h"

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

status=0
for h in $HEADERS; do
    echo "#include \"$h\"" > "$tmpdir/check.c"
    if cc -std=gnu11 -IQuake -fsyntax-only -Wall "$tmpdir/check.c" 2> "$tmpdir/err.txt"; then
        echo "ok:   $h"
    else
        echo "FAIL: $h"
        cat "$tmpdir/err.txt"
        status=1
    fi
    # a core header must not drag in SDL or Vulkan transitively
    if cc -std=gnu11 -IQuake -E "$tmpdir/check.c" 2>/dev/null | grep -q -m1 -e 'SDL' -e 'vulkan_core'; then
        echo "FAIL: $h pulls in SDL/Vulkan"
        status=1
    fi
done

# the actual Phase 0 exit criterion: bindgen processes the core headers standalone
if command -v bindgen > /dev/null 2>&1; then
    for h in $HEADERS; do
        if bindgen "Quake/$h" --allowlist-file ".*$h" -- -IQuake > /dev/null 2>&1; then
            echo "bindgen ok:   $h"
        else
            echo "bindgen FAIL: $h"
            status=1
        fi
    done
else
    echo "note: bindgen not on PATH; skipped the bindgen smoke (cargo install bindgen-cli)"
fi
exit $status
