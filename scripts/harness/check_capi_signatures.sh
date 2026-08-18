#!/bin/bash
# Phase 1 FFI drift gate (ADR-011): compile one TU that includes the
# cbindgen-generated quake_rs.h together with the original engine headers
# whose functions the Rust staticlib now provides. If a quake-capi shim's
# signature drifts from the C original's declaration, the redeclaration is a
# conflicting-types compile error.
#
# Usage: check_capi_signatures.sh [path/to/quake_rs.h]
# Without an argument, quake_rs.h is generated with cbindgen (must be on PATH).
set -euo pipefail
cd "$(dirname "$0")/../.."

CC="${CC:-cc}"
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

if [ $# -ge 1 ]; then
    header_dir=$(cd "$(dirname "$1")" && pwd)
else
    cbindgen --config rust/quake-capi/cbindgen.toml --output "$tmpdir/quake_rs.h" rust/quake-capi
    header_dir="$tmpdir"
fi

# headers of every module ported so far; grows with each Phase 1 module
cat > "$tmpdir/capi_sig_check.c" <<'EOF'
#include <stddef.h>
#include "q_types.h"
#include "quake_rs.h"
#include "crc.h"
#include "strl_fn.h"
EOF

"$CC" -fsyntax-only -Werror -IQuake -I"$header_dir" "$tmpdir/capi_sig_check.c"
echo "OK: quake_rs.h declarations are compatible with the engine headers"
