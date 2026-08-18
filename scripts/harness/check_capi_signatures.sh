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
#include <stdint.h>
#include <string.h>
#include "q_types.h"
/* engine headers first: quake_rs.h guards its typedef-dependent declarations
 * (wad, JSON_Find*) on these headers' include guards, so they only
 * materialize -- and only get diffed against the originals -- once the
 * engine declarations are in scope */
#include "crc.h"
#include "strl_fn.h"
#include "hash_map.h"
#include "json.h"
#include "cfgfile.h"
#include "wad.h"
/* mathlib.h needs quakedef.h's bit-scan inline */
#ifdef _MSC_VER
#include <intrin.h>
static inline int FindLastBitNonZero (const uint32_t mask)
{
	unsigned long result;
	_BitScanReverse (&result, mask);
	return (int)result;
}
#else
static inline int FindLastBitNonZero (const uint32_t mask)
{
	return 31 ^ __builtin_clz (mask);
}
#endif
#include "mathlib.h"
#include "quake_rs.h"
EOF

"$CC" -fsyntax-only -Werror -IQuake -I"$header_dir" "$tmpdir/capi_sig_check.c"
echo "OK: quake_rs.h declarations are compatible with the engine headers"
