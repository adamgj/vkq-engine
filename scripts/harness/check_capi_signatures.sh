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
/* Phase 2: the filesystem surface (the COM_, FS_ and LOC_ families in
 * common.h, the discovery half of steam.h, mem.h for the Mem_ boundary) */
#include "sys.h"
#include "mem.h"
#include "common.h"
#include "steam.h"
/* Phase 3 M2: the PCX/LMP decoders (image_decode.c). image.h forward-
 * declares enum srcformat (a Microsoft extension under -Werror clang-cl);
 * its real definition lives in gl_texmgr.h, which pulls Vulkan and cannot
 * be in this TU, so define a stand-in tag first (only Image_LoadImage,
 * which stays C, touches it) */
enum srcformat
{
	SRCFORMAT_SIG_CHECK_ONLY
};
#include "image.h"
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
/* Phase 4: the sound surface (q_sound.h needs cvar_t and qmutex_t) */
#include "cvar.h"
#include "q_thread.h"
#include "q_sound.h"
#include "snd_codec.h"
#include "snd_codeci.h"
#include "bgmusic.h"
/* Phase 6 M3: the progs interpreter shim. progs.h needs protocol.h for
 * entity_state_t (edict_t embeds it), progdefs.h for entvars_t, and
 * quakedef.h's per-level limits for freelist_t. */
#include "protocol.h"
#define MIN_EDICTS 256
#define MAX_EDICTS 32000
#include "progs.h"
#include "quake_rs.h"
EOF

"$CC" -fsyntax-only -Werror -IQuake -I"$header_dir" "$tmpdir/capi_sig_check.c"

# Phase 3 M3: the brush/BSP loader seam. gl_model.h pulls Vulkan (through
# q_render_types.h) and SDL (through atomics.h), so it cannot join the TU
# above -- it gets its own, with the same stand-in trick the ctest c_ref
# prelude uses: pre-define both include guards and supply 64-bit handle and
# atomic stand-ins, which is all the seam declarations need.
cat > "$tmpdir/capi_sig_check_model.c" <<'EOF'
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "q_types.h"
#define __Q_RENDER_TYPES_H
typedef struct VkAccelerationStructureKHR_T *VkAccelerationStructureKHR;
typedef struct VkBuffer_T *VkBuffer;
typedef struct VkDescriptorSet_T *VkDescriptorSet;
typedef uint64_t VkDeviceAddress;
#define __ATOMICS_H
typedef struct
{
	volatile uint32_t value;
} atomic_uint32_t;
/* quakedef.h slice gl_model.h needs; PSET_SCRIPT is unconditional there and
 * changes qmodel_t's layout, so it must be defined before gl_model.h */
#define PSET_SCRIPT
typedef struct efrag_s efrag_t;
#define MAX_DLIGHTS 64
#define MAX_LIGHTSTYLES 64
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
struct cvar_s; /* cvar.h only uses it in prototypes (-Wvisibility) */
#include "cvar.h"
#include "common.h"
#include "steam.h" /* quake_rs.h declares the Steam_ shims unguarded */
#include "wad.h"
#include "bspfile.h"
#include "gl_model.h"
/* Phase 3 M4: the sprite loader calls TexMgr_LoadImage. The real
 * gl_texmgr.h pulls Vulkan and tasks.h, so -- exactly as the ctest c_ref
 * prelude does -- hand-copy the slice the seam needs and claim the header's
 * guard, which is what quake_rs.h keys its declaration off. The residual
 * risk is drift between this slice and the real header; the engine build is
 * what catches that. */
#define _GL_TEXMAN_H
enum srcformat
{
	SRC_INDEXED,
	SRC_LIGHTMAP,
	SRC_RGBA,
	SRC_SURF_INDICES,
	SRC_RGBA_CUBEMAP,
	SRC_INDEXED_PALETTE,
};
typedef struct gltexture_s gltexture_t;
gltexture_t *TexMgr_LoadImage (
	qmodel_t *owner, const char *name, int width, int height, enum srcformat format, byte *data, const char *source_file, src_offset_t source_offset,
	unsigned flags);
/* Phase 3 M5: the MD3/MD5 loaders hand their buffers to gl_mesh.c. The real
 * glquake.h pulls Vulkan and tasks.h, so -- as for gl_texmgr.h above --
 * hand-copy the two declarations the seam needs and claim the header's
 * guard, which is what quake_rs.h keys them off. Residual risk: drift
 * between this slice and the real header, which only the engine build
 * catches. */
#define GLQUAKE_H
void GLMesh_UploadBuffers (qmodel_t *mod, aliashdr_t *hdr, unsigned short *indexes, byte *vertexes, aliasmesh_t *desc, jointpose_t *joints);
void GLMesh_DeleteMeshBuffers (aliashdr_t *mainhdr);
#include "mathlib.h"
#include "model_parse.h"
#include "quake_rs.h"
EOF

"$CC" -fsyntax-only -Werror -IQuake -I"$header_dir" "$tmpdir/capi_sig_check_model.c"
echo "OK: quake_rs.h declarations are compatible with the engine headers"
