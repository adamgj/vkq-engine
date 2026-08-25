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
#define BSP29_VALVE		  /* quakedef.h defines it unconditionally; bspfile.h/model_parse.c gate on it */
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

/* mplane_t comes from the real gl_model.h, included below (Phase 3) */

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

/* mathlib.h needs mplane_t, which now comes from gl_model.h; both are
 * included in the Phase 3 model slice below (after the renames) */

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

/* image_decode.c (Phase 3): PCX/LMP decoders; the fs plumbing they call
 * (COM_CloseFile, com_filesize, Sys_File*) resolves through the renames and
 * stubs above/below like the rest of the reference fs */
#define Image_DecodePCX c_ref_Image_DecodePCX
#define Image_DecodeLMP c_ref_Image_DecodeLMP

/* image_stb.c (Phase 3 M8): the streaming stb decoder is the oracle.
 * Image_DecodeSTBMem is NOT renamed: it stays shared — the Rust shim under
 * test routes crate-undecoded formats through it (same convention as
 * Mod_FindName above; stb's failure-reason string is thread-local, so the
 * two users cannot race). */
#define Image_DecodeSTB c_ref_Image_DecodeSTB

/* model_parse.c (Phase 3): rename every seam symbol from model_parse.h.
 * Mod_FindName / Mod_LoadWadTexture / Mod_LoadAllSkins (gl_model.h) are NOT
 * renamed: they stay stub-owned and shared by both sides. */
#define Mod_DecompressVis		   c_ref_Mod_DecompressVis
#define Mod_ParseTextures		   c_ref_Mod_ParseTextures
#define Mod_LoadLighting		   c_ref_Mod_LoadLighting
#define Mod_LoadVisibility		   c_ref_Mod_LoadVisibility
#define Mod_LoadEntities		   c_ref_Mod_LoadEntities
#define Mod_LoadVertexes		   c_ref_Mod_LoadVertexes
#define Mod_LoadEdges			   c_ref_Mod_LoadEdges
#define Mod_LoadTexinfo			   c_ref_Mod_LoadTexinfo
#define CalcSurfaceExtents		   c_ref_CalcSurfaceExtents
#define Mod_ParseFaces			   c_ref_Mod_ParseFaces
#define Mod_LoadNodes			   c_ref_Mod_LoadNodes
#define Mod_LoadLeafs			   c_ref_Mod_LoadLeafs
#define Mod_LoadClipnodes		   c_ref_Mod_LoadClipnodes
#define Mod_MakeHull0			   c_ref_Mod_MakeHull0
#define Mod_LoadMarksurfaces	   c_ref_Mod_LoadMarksurfaces
#define Mod_LoadSurfedges		   c_ref_Mod_LoadSurfedges
#define Mod_LoadPlanes			   c_ref_Mod_LoadPlanes
#define Mod_LoadSubmodels		   c_ref_Mod_LoadSubmodels
#define Mod_SetupSubmodels		   c_ref_Mod_SetupSubmodels
#define Mod_FindVisibilityExternal c_ref_Mod_FindVisibilityExternal
#define Mod_LoadVisibilityExternal c_ref_Mod_LoadVisibilityExternal
#define Mod_LoadLeafsExternal	   c_ref_Mod_LoadLeafsExternal
#define Mod_ParseAliasModel		   c_ref_Mod_ParseAliasModel
#define Mod_CalcAliasBounds		   c_ref_Mod_CalcAliasBounds
#define Mod_LoadSpriteModel		   c_ref_Mod_LoadSpriteModel
#define Mod_LoadMD5MeshModel	   c_ref_Mod_LoadMD5MeshModel
#define Mod_LoadMD3Model		   c_ref_Mod_LoadMD3Model

/* ---- the Phase 3 model slice: real bspfile.h + gl_model.h ----
 *
 * gl_model.h pulls Vulkan solely through q_render_types.h, and the only
 * Vk-typed fields are the qmodel_t ray-tracing tail, which parsing never
 * touches. Pre-defining the include guard and 64-bit handle stand-ins keeps
 * the ctest build Vulkan-SDK-free (same trick as the capi signature gate). */
#define __Q_RENDER_TYPES_H
typedef struct VkAccelerationStructureKHR_T *VkAccelerationStructureKHR;
typedef struct VkBuffer_T					*VkBuffer;
typedef struct VkDescriptorSet_T			*VkDescriptorSet; /* aliashdr_t */
typedef uint64_t							 VkDeviceAddress;

/* atomics.h drags in q_stdinc.h -> SDL.h; stand in for the one type/op the
 * model headers and model_parse.c use. Both real variants (MSVC volatile
 * struct, C11 _Atomic uint32_t) are a 4-byte u32, so the layout matches. */
#define __ATOMICS_H
typedef struct
{
	volatile uint32_t value;
} atomic_uint32_t;
static inline void Atomic_StoreUInt32 (volatile atomic_uint32_t *atomic, uint32_t desired)
{
	atomic->value = desired;
}

/* quakedef.h slice gl_model.h / model_parse.c need. PSET_SCRIPT is
 * unconditional in quakedef.h and changes qmodel_t's layout, so it must be
 * defined before gl_model.h. */
#define PSET_SCRIPT
typedef struct efrag_s efrag_t;
#define MAX_DLIGHTS			  64
#define MAX_LBM_HEIGHT		  480
#define MAX_LIGHTSTYLES		  64
#define ALIAS_BASE_SIZE_RATIO (1.0 / 11.0) /* glquake.h */

#include "cvar.h"
#include "wad.h"
#include "bspfile.h"
#include "gl_model.h"
#include "mathlib.h"
#include "hash_map.h" /* MD5_ComputeNormals; the HashMap_ renames are above */
#include "model_parse.h"

FUNC_NORETURN void Host_Error (const char *error, ...);
void			   Con_DWarning (const char *fmt, ...);

/* glquake.h declares this and drags in Vulkan; PSET_SCRIPT is on above, so
 * Mod_SetExtraFlags calls it. Apple clang errors on the implicit declaration
 * where gcc/cl only warn. */
void PScript_UpdateModelEffects (qmodel_t *mod);

/* gl_texmgr.h slice for the sprite loader (the real header drags in tasks.h
 * and Vk-typed structs); values match Quake/gl_texmgr.h */
enum srcformat
{
	SRC_INDEXED,
	SRC_LIGHTMAP,
	SRC_RGBA,
	SRC_SURF_INDICES,
	SRC_RGBA_CUBEMAP,
	SRC_INDEXED_PALETTE,
};
#define TEXPREF_ALPHA	 0x0008
#define TEXPREF_PAD		 0x0010
#define TEXPREF_NOPICMIP 0x0080
typedef struct gltexture_s gltexture_t;
gltexture_t *TexMgr_LoadImage (
	qmodel_t *owner, const char *name, int width, int height, enum srcformat format, byte *data, const char *source_file, src_offset_t source_offset,
	unsigned flags);

/* glquake.h slice: the MD3/MD5 loaders hand their parsed index/vertex/joint
 * buffers to the mesh uploader and release them on the MD5 error path. The
 * real header pulls Vulkan; the stubs record the arguments instead. */
void GLMesh_UploadBuffers (qmodel_t *mod, aliashdr_t *hdr, unsigned short *indexes, byte *vertexes, aliasmesh_t *desc, jointpose_t *joints);
void GLMesh_DeleteMeshBuffers (aliashdr_t *mainhdr);

/* server.h slice: model_parse.c reads only sv.modelname */
typedef struct
{
	char modelname[64];
} ctest_server_stub_t;
extern ctest_server_stub_t sv;

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

/* ---- Phase 4 sound slice: snd_mem.c as the sfx loader/resampler oracle ----
 *
 * COM_LoadFile / com_filesize resolve through the Phase 2 renames and fs
 * stubs above. shm / snd_mutex / loadas8bit are NOT renamed: they stay
 * stub-owned state shared by the c_ref side and (from M3 on) the Rust shims,
 * exactly like the engine's own globals. QMutex_* are no-op stubs (the
 * differential suites are single-threaded). */
#define S_LoadSound c_ref_S_LoadSound
#define GetWavinfo	c_ref_GetWavinfo
#define ResampleSfx c_ref_ResampleSfx

#include "common.h"
#include "q_thread.h"
#include "q_sound.h"

/* file-internal in the engine build; snd_mem.c un-statics it for this
 * oracle build (the rename above applies) */
void ResampleSfx (sfx_t *sfx, int inrate, int inwidth, byte *data);

#endif /* C_REF_PRELUDE_H */
