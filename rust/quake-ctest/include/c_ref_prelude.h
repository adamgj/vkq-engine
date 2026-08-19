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

/* common_fs.c / steam.c (Phase 2): rename every public symbol so the
 * reference C filesystem links next to the Rust fs shims (quake-capi's `fs`
 * feature exports the same names). com_filesize / file_from_pak / com_token
 * and the COM_Thread* accessors are NOT renamed: they stay stub-owned TLS
 * state shared by both sides, exactly like the engine keeps them in common.c.
 */
#define com_modified					 c_ref_com_modified
#define standard_quake					 c_ref_standard_quake
#define rogue							 c_ref_rogue
#define hipnotic						 c_ref_hipnotic
#define com_gamenames					 c_ref_com_gamenames
#define com_gamedir						 c_ref_com_gamedir
#define com_basedir						 c_ref_com_basedir
#define com_basedirs					 c_ref_com_basedirs
#define com_numbasedirs					 c_ref_com_numbasedirs
#define com_searchpaths					 c_ref_com_searchpaths
#define com_base_searchpaths			 c_ref_com_base_searchpaths
#define COM_AddBaseDir					 c_ref_COM_AddBaseDir
#define COM_WriteFile					 c_ref_COM_WriteFile
#define COM_FileExists					 c_ref_COM_FileExists
#define COM_OpenFile					 c_ref_COM_OpenFile
#define COM_FOpenFile					 c_ref_COM_FOpenFile
#define COM_CloseFile					 c_ref_COM_CloseFile
#define COM_LoadFile					 c_ref_COM_LoadFile
#define COM_LoadMallocFile_TextMode_OSPath c_ref_COM_LoadMallocFile_TextMode_OSPath
#define COM_GetGameNames				 c_ref_COM_GetGameNames
#define COM_GameDirMatches				 c_ref_COM_GameDirMatches
#define COM_ResetGameDirectories		 c_ref_COM_ResetGameDirectories
#define COM_ModForbiddenChars			 c_ref_COM_ModForbiddenChars
#define COM_FOpenPrefFile				 c_ref_COM_FOpenPrefFile
#define COM_WriteSelectedBaseDir		 c_ref_COM_WriteSelectedBaseDir
#define COM_InitFilesystem				 c_ref_COM_InitFilesystem
#define COM_Effectinfo_Enumerate		 c_ref_COM_Effectinfo_Enumerate
#define FS_fread						 c_ref_FS_fread
#define FS_fseek						 c_ref_FS_fseek
#define FS_fclose						 c_ref_FS_fclose
#define FS_ftell						 c_ref_FS_ftell
#define FS_rewind						 c_ref_FS_rewind
#define FS_feof							 c_ref_FS_feof
#define FS_ferror						 c_ref_FS_ferror
#define FS_fgetc						 c_ref_FS_fgetc
#define FS_fgets						 c_ref_FS_fgets
#define FS_filelength					 c_ref_FS_filelength
#define COM_HashString					 c_ref_COM_HashString
#define COM_HashBlock					 c_ref_COM_HashBlock
#define LOC_LoadFile					 c_ref_LOC_LoadFile
#define LOC_Init						 c_ref_LOC_Init
#define LOC_Shutdown					 c_ref_LOC_Shutdown
#define LOC_GetRawString				 c_ref_LOC_GetRawString
#define LOC_GetString					 c_ref_LOC_GetString
#define LOC_HasPlaceholders				 c_ref_LOC_HasPlaceholders
#define LOC_Format						 c_ref_LOC_Format
#define Steam_IsValidPath				 c_ref_Steam_IsValidPath
#define Steam_FindGame					 c_ref_Steam_FindGame
#define Steam_ResolvePath				 c_ref_Steam_ResolvePath
#define EGS_FindGame					 c_ref_EGS_FindGame

/* wad.c only includes quakedef.h; hand it wad.h, which pulls the real,
 * bindgen-clean common.h for the COM_ and FS_ APIs and fshandle_t */
#include "wad.h"

/* ---- the quakedef.h slice common_fs.c / steam.c need ---- */

/* scriptable particle system: quakedef.h defines it unconditionally, and
 * COM_Effectinfo_Enumerate is compiled only under it */
#define PSET_SCRIPT

#include "cvar.h"
#include "crc.h" /* common_fs.c CRCs pak directories (via the renames) */

/* quakedef.h host globals (definitions live in stubs.c) */
typedef struct
{
	const char *basedir;
	const char *userdir; // user's directory on UNIX platforms.
						 // if user directories are enabled, basedir
						 // and userdir will point to different
						 // memory locations, otherwise to the same.
	int			argc;
	char	  **argv;
	int			errstate;
} quakeparms_t;

extern quakeparms_t *host_parms;
extern qboolean		 isDedicated;
extern qboolean		 multiuser;
extern qboolean		 harness_active;
extern cvar_t		 developer;

/* cmd.h's registration macro over the real Cmd_AddCommand2 signature (the
 * stub logs the name); cmd.h itself drags in non-clean headers */
typedef void (*xcommand_t) (void);
typedef enum
{
	src_client,
	src_command,
	src_server
} cmd_source_t;
struct cmd_function_s *Cmd_AddCommand2 (const char *cmd_name, xcommand_t function, cmd_source_t srctype, qboolean qcinterceptable);
#define Cmd_AddCommand(cmdname, func) Cmd_AddCommand2 (cmdname, func, src_command, false)

#endif /* C_REF_PRELUDE_H */
