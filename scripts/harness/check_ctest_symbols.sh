#!/bin/bash
# quake-ctest oracle-symbol gate (Rust migration, Phase 4).
#
# Every engine C file listed in rust/quake-ctest/build.rs's C_SOURCES is
# compiled as the differential oracle with its public symbols renamed to
# c_ref_* by include/c_ref_prelude.h, so the originals can link beside the
# Rust ports. A global that is missed by that rename block collides with the
# Rust-side definition -- but only on a linker that does not dead-strip:
# ld64 and rust-lld drop the unreferenced object and stay silent, while MSVC
# link.exe resolves everything in a pulled-in object and fails. That is
# exactly how `precache` (the one snd_dma.c global not in the rename list)
# reached CI and broke the Windows job alone.
#
# This gate closes that hole mechanically: it reads the defined symbols out
# of the compiled oracle objects and requires each to be either c_ref_*
# prefixed or an explicitly allowlisted shared symbol.
set -euo pipefail
cd "$(dirname "$0")/../.."

# Symbols the oracle sources deliberately export un-renamed. Anything not
# matched here (or c_ref_* prefixed) is a missing rename.
#
#   miniz     -- vendored zlib clone, #include'd into common_fs.c's TU; not
#                engine API, and the Rust side never defines these
#   stb       -- Image_DecodeSTBMem stays shared on purpose: the Rust
#                Image_DecodeSTB routes crate-undecoded formats through it
#                (ADR-012 seam), so both sides must call the same function
#   alias     -- Mod_SetExtraFlags and the alias scratch arrays gl_mesh.c
#                reads stay C until Phase 8 (documented in c_ref_prelude.h)
ALLOW_RE='^(mz_|miniz_|tdefl_|tinfl_)'
ALLOW_EXACT="
Image_DecodeSTBMem
Mod_LoadAliasFrame
Mod_LoadAliasGroup
Mod_SetExtraFlags
RadiusFromBounds
poseverts
stverts
triangles
triangles_size
"

echo "building quake-ctest (oracle objects) ..."
cargo build --manifest-path rust/Cargo.toml -p quake-ctest --quiet

out_dir=$(ls -dt rust/target/debug/build/quake-ctest-*/out 2>/dev/null | head -1)
if [ -z "$out_dir" ] || [ ! -d "$out_dir" ]; then
    echo "FAIL: could not locate the quake-ctest build output directory"
    exit 1
fi

# only the C_SOURCES array -- build.rs also names Quake/miniz.c and
# Quake/stb_image.h in rerun-if-changed lines, and those are #include'd into
# another TU rather than compiled on their own
sources=$(awk '/const C_SOURCES/, /\];/' rust/quake-ctest/build.rs \
    | grep -oE '"Quake/[A-Za-z0-9_]+\.c"' | sed 's|"Quake/||; s|\.c"||')
if [ -z "$sources" ]; then
    echo "FAIL: no C_SOURCES entries parsed out of rust/quake-ctest/build.rs"
    exit 1
fi

status=0
checked=0
for base in $sources; do
    obj=$(ls "$out_dir"/*-"$base".o 2>/dev/null | head -1 || true)
    if [ -z "$obj" ]; then
        echo "FAIL: no compiled object for Quake/$base.c in $out_dir"
        status=1
        continue
    fi
    checked=$((checked + 1))

    # defined (non-U) globals; macOS nm prefixes symbols with an underscore
    leaked=$(nm -g "$obj" 2>/dev/null \
        | awk 'NF >= 3 && $2 != "U" { print $3 }' \
        | sed 's/^_//' \
        | grep -v '^c_ref_' \
        | grep -Ev "$ALLOW_RE" \
        | grep -vxF "$(echo "$ALLOW_EXACT" | grep -v '^$')" || true)

    if [ -n "$leaked" ]; then
        echo "FAIL: Quake/$base.c exports un-renamed global(s):"
        echo "$leaked" | sed 's/^/        /'
        echo "       add each to the c_ref_* rename block in"
        echo "       rust/quake-ctest/include/c_ref_prelude.h, or to this script's"
        echo "       allowlist if it is deliberately shared with the Rust side"
        status=1
    else
        echo "ok:   Quake/$base.c"
    fi
done

if [ "$status" -eq 0 ]; then
    echo "OK: $checked oracle sources export only c_ref_* and allowlisted symbols"
fi
exit $status
