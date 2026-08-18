/* Force-included (-include / /FI) into every reference C file compiled by
 * quake-ctest's build.rs.
 *
 * 1. Pre-empts the real quakedef.h -- the Phase 1 C files start with
 *    #include "quakedef.h", whose quote-include resolves to the real,
 *    SDL-tainted header first; defining its include guard up front makes that
 *    include a no-op, and q_types.h supplies the little these files need.
 *
 * 2. Renames every public symbol of the reference C files to c_ref_<name> so
 *    they link into the same test binary as the Rust implementations
 *    (quake-capi rlib) without clashing.
 */

#ifndef C_REF_PRELUDE_H
#define C_REF_PRELUDE_H

#define QUAKEDEFS_H
#define _USE_MATH_DEFINES /* M_PI on MSVC */
#include "q_types.h"
#include <assert.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>
#include "mem.h"

size_t UTF8_WriteCodePoint (char *dst, size_t maxbytes, uint32_t codepoint);

/* the slice of quakedef.h the reference files actually need */
#define PITCH 0
#define YAW	  1
#define ROLL  2

typedef struct mplane_s
{
	vec3_t normal;
	float  dist;
	byte   type;
	byte   signbits;
	byte   pad[2];
} mplane_t;

void Sys_Error (const char *error, ...);

/* quakedef.h's bit-scan helper, needed by mathlib.h's Q_log2/Q_nextPow2 */
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

/* crc.c */
#define CRC_Init			  c_ref_CRC_Init
#define CRC_ProcessByte		  c_ref_CRC_ProcessByte
#define CRC_Value			  c_ref_CRC_Value
#define CRC_Block			  c_ref_CRC_Block

/* mdfour.c */
#define Com_BlockChecksum	  c_ref_Com_BlockChecksum
#define Com_BlockFullChecksum c_ref_Com_BlockFullChecksum

/* strlcpy.c / strlcat.c */
#define q_strlcpy			  c_ref_q_strlcpy
#define q_strlcat			  c_ref_q_strlcat

/* mathlib.c (incl. the data symbol, which the Rust staticlib also exports) */
#define vec3_origin				 c_ref_vec3_origin
#define ProjectPointOnPlane		 c_ref_ProjectPointOnPlane
#define PerpendicularVector		 c_ref_PerpendicularVector
#define RotatePointAroundVector	 c_ref_RotatePointAroundVector
#define anglemod				 c_ref_anglemod
#define BoxOnPlaneSide			 c_ref_BoxOnPlaneSide
#define VectorAngles			 c_ref_VectorAngles
#define AngleVectors			 c_ref_AngleVectors
#define VectorCompare			 c_ref_VectorCompare
#define VectorMA				 c_ref_VectorMA
#define _DotProduct				 c_ref_DotProduct_fn
#define _VectorSubtract			 c_ref_VectorSubtract_fn
#define _VectorAdd				 c_ref_VectorAdd_fn
#define _VectorCopy				 c_ref_VectorCopy_fn
#define CrossProduct			 c_ref_CrossProduct
#define VectorLength			 c_ref_VectorLength
#define VectorNormalize			 c_ref_VectorNormalize
#define VectorInverse			 c_ref_VectorInverse
#define VectorScale				 c_ref_VectorScale
#define R_ConcatRotations		 c_ref_R_ConcatRotations
#define R_ConcatTransforms		 c_ref_R_ConcatTransforms
#define FloorDivMod				 c_ref_FloorDivMod
#define GreatestCommonDivisor	 c_ref_GreatestCommonDivisor
#define Invert24To16			 c_ref_Invert24To16
#define MatrixMultiply			 c_ref_MatrixMultiply
#define RotationMatrix			 c_ref_RotationMatrix
#define TranslationMatrix		 c_ref_TranslationMatrix
#define ScaleMatrix				 c_ref_ScaleMatrix
#define IdentityMatrix			 c_ref_IdentityMatrix
#define IsOriginWithinMinMax	 c_ref_IsOriginWithinMinMax
#define IsAxisAlignedDeg		 c_ref_IsAxisAlignedDeg

#include "mathlib.h"

/* hash_map.c */
#define HashMap_CreateImpl	 c_ref_HashMap_CreateImpl
#define HashMap_Destroy		 c_ref_HashMap_Destroy
#define HashMap_Reserve		 c_ref_HashMap_Reserve
#define HashMap_Clear		 c_ref_HashMap_Clear
#define HashMap_InsertImpl	 c_ref_HashMap_InsertImpl
#define HashMap_EraseImpl	 c_ref_HashMap_EraseImpl
#define HashMap_LookupImpl	 c_ref_HashMap_LookupImpl
#define HashMap_Size		 c_ref_HashMap_Size
#define HashMap_GetKeyImpl	 c_ref_HashMap_GetKeyImpl
#define HashMap_GetValueImpl c_ref_HashMap_GetValueImpl

/* json.c */
#define JSON_Parse		 c_ref_JSON_Parse
#define JSON_Free		 c_ref_JSON_Free
#define JSON_Find		 c_ref_JSON_Find
#define JSON_FindString	 c_ref_JSON_FindString
#define JSON_FindNumber	 c_ref_JSON_FindNumber
#define JSON_FindBoolean c_ref_JSON_FindBoolean

/* wad.c (incl. its global data symbols, which the Rust staticlib also
 * exports) */
#define GAMENAME			"id1"
#define wad_numlumps		c_ref_wad_numlumps
#define wad_lumps			c_ref_wad_lumps
#define wad_base			c_ref_wad_base
#define W_LoadWadFile		c_ref_W_LoadWadFile
#define W_CleanupName		c_ref_W_CleanupName
#define W_GetLumpName		c_ref_W_GetLumpName
#define W_LoadWadList		c_ref_W_LoadWadList
#define W_FreeWadList		c_ref_W_FreeWadList
#define W_GetLumpinfoList	c_ref_W_GetLumpinfoList
#define SwapPic				c_ref_SwapPic

/* cfgfile.c */
#define CFG_OpenConfig		 c_ref_CFG_OpenConfig
#define CFG_CloseConfig		 c_ref_CFG_CloseConfig
#define CFG_ReadCvars		 c_ref_CFG_ReadCvars
#define CFG_ReadCvarOverrides c_ref_CFG_ReadCvarOverrides
#define CONFIG_NAME			 "vkQuake.cfg"

void Con_Printf (const char *fmt, ...);
void Cvar_Set (const char *var_name, const char *value);
void Con_Warning (const char *fmt, ...);
void Con_DPrintf (const char *fmt, ...);
void Con_DPrintf2 (const char *fmt, ...);

/* wad.c only includes quakedef.h; hand it wad.h, which pulls the real,
 * bindgen-clean common.h for the COM_ and FS_ APIs and fshandle_t */
#include "wad.h"

#endif /* C_REF_PRELUDE_H */
