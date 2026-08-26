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
/* q_stdinc.h pulls SDL.h; q_types.h + the libc includes below supply what
 * the reference files (net_loop.c is the first direct includer) need */
#define __QSTDINC_H
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

/* quakedef.h net message limits (Phase 5: net_defs.h's NET_DATAGRAMSIZE and
 * the abi_probe net table use them) */
#define MAX_MSGLEN	 64000
#define MAX_DATAGRAM 64000
#define DATAGRAM_MTU 1400

/* net_msg.c (Phase 5 M2): wire serialization oracle. The renames must sit
 * before the first common.h include so the declarations there rename too.
 * net_message itself lives in net_main.c (not compiled here); the c_ref
 * definition is stub-owned so tests can point its data at fixtures. */
#define MSG_WriteChar				 c_ref_MSG_WriteChar
#define MSG_WriteByte				 c_ref_MSG_WriteByte
#define MSG_WriteShort				 c_ref_MSG_WriteShort
#define MSG_WriteLong				 c_ref_MSG_WriteLong
#define MSG_WriteUInt64				 c_ref_MSG_WriteUInt64
#define MSG_WriteInt64				 c_ref_MSG_WriteInt64
#define MSG_WriteFloat				 c_ref_MSG_WriteFloat
#define MSG_WriteDouble				 c_ref_MSG_WriteDouble
#define MSG_WriteString				 c_ref_MSG_WriteString
#define MSG_WriteStringUnterminated	 c_ref_MSG_WriteStringUnterminated
#define MSG_WriteCoord16			 c_ref_MSG_WriteCoord16
#define MSG_WriteCoord24			 c_ref_MSG_WriteCoord24
#define MSG_WriteCoord32f			 c_ref_MSG_WriteCoord32f
#define MSG_WriteCoord				 c_ref_MSG_WriteCoord
#define MSG_WriteAngle				 c_ref_MSG_WriteAngle
#define MSG_WriteAngle16			 c_ref_MSG_WriteAngle16
#define MSG_WriteEntity				 c_ref_MSG_WriteEntity
#define MSG_BeginReading			 c_ref_MSG_BeginReading
#define MSG_ReadChar				 c_ref_MSG_ReadChar
#define MSG_ReadByte				 c_ref_MSG_ReadByte
#define MSG_ReadShort				 c_ref_MSG_ReadShort
#define MSG_ReadLong				 c_ref_MSG_ReadLong
#define MSG_ReadUInt64				 c_ref_MSG_ReadUInt64
#define MSG_ReadInt64				 c_ref_MSG_ReadInt64
#define MSG_ReadFloat				 c_ref_MSG_ReadFloat
#define MSG_ReadDouble				 c_ref_MSG_ReadDouble
#define MSG_ReadString				 c_ref_MSG_ReadString
#define MSG_ReadCoord16				 c_ref_MSG_ReadCoord16
#define MSG_ReadCoord24				 c_ref_MSG_ReadCoord24
#define MSG_ReadCoord32f			 c_ref_MSG_ReadCoord32f
#define MSG_ReadCoord				 c_ref_MSG_ReadCoord
#define MSG_ReadAngle				 c_ref_MSG_ReadAngle
#define MSG_ReadAngle16				 c_ref_MSG_ReadAngle16
#define MSG_ReadEntity				 c_ref_MSG_ReadEntity
#define SZ_Alloc					 c_ref_SZ_Alloc
#define SZ_Free						 c_ref_SZ_Free
#define SZ_Clear					 c_ref_SZ_Clear
#define SZ_GetSpace					 c_ref_SZ_GetSpace
#define SZ_Write					 c_ref_SZ_Write
#define SZ_Print					 c_ref_SZ_Print
#define msg_readcount				 c_ref_msg_readcount
/* net_loop.c (Phase 5 M5): the loopback oracle + the net_main.c globals it
 * references (stub-owned; the Rust shims' unrenamed stand-ins live in
 * rust/quake-ctest/src/net_stubs.rs) */
#define Loop_Init					 c_ref_Loop_Init
#define Loop_Shutdown				 c_ref_Loop_Shutdown
#define Loop_Listen					 c_ref_Loop_Listen
#define Loop_SearchForHosts			 c_ref_Loop_SearchForHosts
#define Loop_Connect				 c_ref_Loop_Connect
#define Loop_CheckNewConnections	 c_ref_Loop_CheckNewConnections
#define Loop_GetMessage				 c_ref_Loop_GetMessage
#define Loop_GetAnyMessage			 c_ref_Loop_GetAnyMessage
#define Loop_SendMessage			 c_ref_Loop_SendMessage
#define Loop_SendUnreliableMessage	 c_ref_Loop_SendUnreliableMessage
#define Loop_CanSendMessage			 c_ref_Loop_CanSendMessage
#define Loop_CanSendUnreliableMessage c_ref_Loop_CanSendUnreliableMessage
#define Loop_Close					 c_ref_Loop_Close
#define NET_NewQSocket				 c_ref_NET_NewQSocket
#define NET_FreeQSocket				 c_ref_NET_FreeQSocket
#define net_driverlevel				 c_ref_net_driverlevel
#define net_activeconnections		 c_ref_net_activeconnections
#define hostCacheCount				 c_ref_hostCacheCount
#define hostcache					 c_ref_hostcache
#define msg_badread					 c_ref_msg_badread
#define net_message					 c_ref_net_message
#define harness_badread_count		 c_ref_harness_badread_count
/* net_dgrm_rel.c (Phase 5 M6): the datagram reliable-layer oracle + the
 * ambient net globals it references (stub-owned) */
#define Datagram_SendMessage			  c_ref_Datagram_SendMessage
#define SendMessageNext					  c_ref_SendMessageNext
#define ReSendMessage					  c_ref_ReSendMessage
#define Datagram_CanSendMessage			  c_ref_Datagram_CanSendMessage
#define Datagram_CanSendUnreliableMessage c_ref_Datagram_CanSendUnreliableMessage
#define Datagram_SendUnreliableMessage	  c_ref_Datagram_SendUnreliableMessage
#define Datagram_ProcessPacket			  c_ref_Datagram_ProcessPacket
#define Datagram_GetMessage				  c_ref_Datagram_GetMessage
#define packetBuffer					  c_ref_packetBuffer
#define packetsSent						  c_ref_packetsSent
#define packetsReSent					  c_ref_packetsReSent
#define packetsReceived					  c_ref_packetsReceived
#define receivedDuplicateCount			  c_ref_receivedDuplicateCount
#define shortPacketCount				  c_ref_shortPacketCount
#define droppedDatagrams				  c_ref_droppedDatagrams
#define net_landrivers					  c_ref_net_landrivers
#define net_time						  c_ref_net_time
#define messagesReceived				  c_ref_messagesReceived
#define unreliableMessagesReceived		  c_ref_unreliableMessagesReceived

/* mplane_t comes from the real gl_model.h, included below (Phase 3) */

/* forward-declare at file scope so cvar.h's cvarcallback_t typedef (which
 * names struct cvar_s inside a parameter list) refers to this type, keeping
 * function pointers to real callbacks compatible (Phase 4: snd_dma.c) */
struct cvar_s;

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

/* server.h slice: model_parse.c reads sv.modelname; snd_mix.c (Phase 4)
 * reads sv.active */
typedef struct
{
	char	 modelname[64];
	qboolean active;
	char	 name[64]; /* Phase 5: net_loop.c's Loop_SearchForHosts */
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
 * stubs above. QMutex_* are no-op stubs (the differential suites are
 * single-threaded). shm / snd_mutex / loadas8bit were stub-owned through
 * M3-M5; from M6 on snd_dma.c *defines* them, so they are renamed with the
 * rest of that file's globals in the block below.
 *
 * Invariant for this whole header: every non-static global defined by a file
 * in build.rs's C_SOURCES is either renamed c_ref_* here or listed in
 * scripts/harness/check_ctest_symbols.sh's shared-symbol allowlist. That
 * script is the mechanical gate -- `precache` was the one snd_dma.c global
 * that never made it into the rename list, and nothing caught it until a
 * linker that does not dead-strip (MSVC) got hold of it. */
#define S_LoadSound c_ref_S_LoadSound
#define GetWavinfo	c_ref_GetWavinfo
#define ResampleSfx c_ref_ResampleSfx

/* snd_mix.c (Phase 4 M4): the mixer oracle. paintbuffer/scaletable/filters
 * stay file-static; snd_channels/total_channels/paintedtime/s_rawsamples and
 * the pause-state globals are stub-owned shared state. */
#define S_PaintChannels			 c_ref_S_PaintChannels
#define SND_InitScaletable		 c_ref_SND_InitScaletable
#define S_SetUnderwaterIntensity c_ref_S_SetUnderwaterIntensity

/* ---- snd_dma.c (Phase 4 M6): channel/spatialization oracle ----
 *
 * snd_dma.c *defines* the shared sound globals and cvars, so they are all
 * renamed c_ref_*; the c_ref sound subsystem (snd_mem/snd_mix/snd_dma) is
 * self-consistent through these renames, and the stubs' setter functions
 * (compiled under this same prelude) write the renamed symbols. */
#define S_Init				 c_ref_S_Init
#define S_Startup			 c_ref_S_Startup
#define S_Shutdown			 c_ref_S_Shutdown
#define S_StartSound		 c_ref_S_StartSound
#define S_StaticSound		 c_ref_S_StaticSound
#define S_StopSound			 c_ref_S_StopSound
#define S_StopAllSounds		 c_ref_S_StopAllSounds
#define S_ClearBuffer		 c_ref_S_ClearBuffer
#define S_Update			 c_ref_S_Update
#define S_ExtraUpdate		 c_ref_S_ExtraUpdate
#define S_ClearAll			 c_ref_S_ClearAll
#define S_BlockSound		 c_ref_S_BlockSound
#define S_UnblockSound		 c_ref_S_UnblockSound
#define S_PrecacheSound		 c_ref_S_PrecacheSound
#define S_TouchSound		 c_ref_S_TouchSound
#define S_LocalSound		 c_ref_S_LocalSound
#define S_RawSamples		 c_ref_S_RawSamples
#define SND_PickChannel		 c_ref_SND_PickChannel
#define SND_Spatialize		 c_ref_SND_Spatialize
#define S_ClearPrecache		 c_ref_S_ClearPrecache
#define S_BeginPrecaching	 c_ref_S_BeginPrecaching
#define S_EndPrecaching		 c_ref_S_EndPrecaching
#define snd_channels		 c_ref_snd_channels
#define total_channels		 c_ref_total_channels
#define shm					 c_ref_shm
#define snd_mutex			 c_ref_snd_mutex
#define soundtime			 c_ref_soundtime
#define paintedtime			 c_ref_paintedtime
#define s_rawend			 c_ref_s_rawend
#define s_rawsamples		 c_ref_s_rawsamples
#define listener_origin		 c_ref_listener_origin
#define listener_forward	 c_ref_listener_forward
#define listener_right		 c_ref_listener_right
#define listener_up			 c_ref_listener_up
#define bgmvolume			 c_ref_bgmvolume
#define sfxvolume			 c_ref_sfxvolume
#define precache			 c_ref_precache
#define loadas8bit			 c_ref_loadas8bit
#define sndspeed			 c_ref_sndspeed
#define snd_mixspeed		 c_ref_snd_mixspeed
#define snd_filterquality	 c_ref_snd_filterquality
#define snd_waterfx			 c_ref_snd_waterfx
#define snd_pauselooping	 c_ref_snd_pauselooping


#include <limits.h>
#include "common.h"
#include "q_thread.h"
#include "q_sound.h"

/* the quakedef.h slice snd_mix.c's pause_loops computation reads; the stub
 * definitions expose setters for the differential tests */
typedef struct
{
	qboolean  paused;
	int		  viewentity;
	qmodel_t *worldmodel;
} ctest_cl_t;
extern ctest_cl_t cl;
typedef struct
{
	int maxclients;
} ctest_svs_t;
extern ctest_svs_t svs;
typedef enum
{
	key_game,
	key_console,
	key_message,
	key_menu
} keydest_t;
extern keydest_t key_dest;
extern double	 host_frametime;

/* file-internal in the engine build; snd_mem.c un-statics it for this
 * oracle build (the rename above applies) */
void ResampleSfx (sfx_t *sfx, int inrate, int inwidth, byte *data);

/* declared file-locally in snd_dma.c/snd_mix.c; the renames above apply */
extern cvar_t snd_waterfx;
extern cvar_t snd_pauselooping;

/* ---- Phase 4 M7: codec framework oracle ----
 * snd_codec.c + the portable codecs (wav/umx/mp3tag). The mp3 *decoder*
 * wrapper is not compiled; c_ref_mp3_codec is a stub dummy vtable identical
 * to the Rust side's, so both frameworks register the same shape. */
#define USE_CODEC_WAVE
#define USE_CODEC_MP3
#define USE_CODEC_UMX
#define S_CodecInit			  c_ref_S_CodecInit
#define S_CodecShutdown		  c_ref_S_CodecShutdown
#define S_CodecOpenStreamType c_ref_S_CodecOpenStreamType
#define S_CodecOpenStreamExt  c_ref_S_CodecOpenStreamExt
#define S_CodecOpenStreamAny  c_ref_S_CodecOpenStreamAny
#define S_CodecForwardStream  c_ref_S_CodecForwardStream
#define S_CodecCloseStream	  c_ref_S_CodecCloseStream
#define S_CodecRewindStream	  c_ref_S_CodecRewindStream
#define S_CodecJumpToOrder	  c_ref_S_CodecJumpToOrder
#define S_CodecReadStream	  c_ref_S_CodecReadStream
#define S_CodecUtilOpen		  c_ref_S_CodecUtilOpen
#define S_CodecUtilClose	  c_ref_S_CodecUtilClose
#define S_CodecIsAvailable	  c_ref_S_CodecIsAvailable
#define wav_codec			  c_ref_wav_codec
#define umx_codec			  c_ref_umx_codec
#define mp3_codec			  c_ref_mp3_codec
#define mp3_skiptags		  c_ref_mp3_skiptags
#define S_WAV_CodecReadStream c_ref_S_WAV_CodecReadStream

#include "snd_codec.h"
#include "snd_codeci.h"

/* the quakedef.h slice snd_dma.c needs beyond the fs slice above */
#define MAX_SOUNDS 2048
#define SIGNONS	   4
typedef enum
{
	ca_dedicated,
	ca_disconnected,
	ca_connected
} cactive_t;
typedef struct
{
	cactive_t state;
	int		  signon;
	int		  demonum;
} ctest_cls_stub_t;
extern ctest_cls_stub_t cls;
/* ---- Phase 5 M2: net_msg.c wire serialization oracle ---- */
#include "protocol.h" /* PRFL_* / PEXT2_* flag sets (pulls q_minmax.h's Q_rint) */
/* Phase 5 M5: net_loop.c oracle needs the net headers (quakedef.h
 * normally supplies net.h; here the prelude does) */
#include "arch_def.h"
#include "net_sys.h"
#include "net.h"
#include "net_defs.h"
#include "net_dgrm.h"	  /* Phase 5 M6: the reliable-layer oracle decls */
#include "net_dgrm_int.h" /* dgrm_packet_t + the rel-layer shared statics */
/* stub-owned c_ref net_message (net_main.c is not compiled here); tests
 * point .data/.cursize at fixtures. The names expand through the renames. */
extern sizebuf_t	net_message;
extern unsigned int harness_badread_count;

mleaf_t				   *Mod_PointInLeaf (float *p, qmodel_t *model); /* stub-owned, settable */
void					S_CodecInit (void);							/* snd_codec.h; stub no-ops */
void					S_CodecShutdown (void);
int						Cmd_Argc (void);
const char			   *Cmd_Argv (int arg);
void					Con_SafePrintf (const char *fmt, ...);

#endif /* C_REF_PRELUDE_H */
