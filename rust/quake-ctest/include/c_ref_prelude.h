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

/* quakedef.h per-level limits the Phase 6 progs oracles need: freelist_t
 * sizes its circular buffer on MAX_EDICTS, and ED_Alloc's reuse policy is
 * written in terms of the two age thresholds */
#define MIN_EDICTS						256
#define MAX_EDICTS						32000
#define MIN_EDICT_AGE_FOR_REUSE			2.0
#define MAX_EDICT_FREETIME_ALWAYS_REUSE 2.0

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
/* net_udp.c (Phase 5 M7b): the pure-address-function oracle (unix only;
 * the socket entry points are compiled but never invoked by tests) */
#define UDP4_Init				 c_ref_UDP4_Init
#define UDP4_Shutdown			 c_ref_UDP4_Shutdown
#define UDP4_Listen				 c_ref_UDP4_Listen
#define UDP4_OpenSocket			 c_ref_UDP4_OpenSocket
#define UDP_CloseSocket			 c_ref_UDP_CloseSocket
#define UDP_Connect				 c_ref_UDP_Connect
#define UDP4_CheckNewConnections c_ref_UDP4_CheckNewConnections
#define UDP_Read				 c_ref_UDP_Read
#define UDP_Write				 c_ref_UDP_Write
#define UDP4_Broadcast			 c_ref_UDP4_Broadcast
#define UDP_AddrToString		 c_ref_UDP_AddrToString
#define UDP4_StringToAddr		 c_ref_UDP4_StringToAddr
#define UDP_GetSocketAddr		 c_ref_UDP_GetSocketAddr
#define UDP_GetNameFromAddr		 c_ref_UDP_GetNameFromAddr
#define UDP4_GetAddrFromName	 c_ref_UDP4_GetAddrFromName
#define UDP_AddrCompare			 c_ref_UDP_AddrCompare
#define UDP_GetSocketPort		 c_ref_UDP_GetSocketPort
#define UDP_SetSocketPort		 c_ref_UDP_SetSocketPort
#define UDP4_GetAddresses		 c_ref_UDP4_GetAddresses
#define UDP6_Init				 c_ref_UDP6_Init
#define UDP6_Shutdown			 c_ref_UDP6_Shutdown
#define UDP6_Listen				 c_ref_UDP6_Listen
#define UDP6_OpenSocket			 c_ref_UDP6_OpenSocket
#define UDP6_CheckNewConnections c_ref_UDP6_CheckNewConnections
#define UDP6_Broadcast			 c_ref_UDP6_Broadcast
#define UDP6_StringToAddr		 c_ref_UDP6_StringToAddr
#define UDP6_GetAddrFromName	 c_ref_UDP6_GetAddrFromName
#define UDP6_GetAddresses		 c_ref_UDP6_GetAddresses
#define net_hostport			 c_ref_net_hostport
#define my_ipv4_address			 c_ref_my_ipv4_address
#define my_ipv6_address			 c_ref_my_ipv6_address
#define ipv4Available			 c_ref_ipv4Available
#define ipv6Available			 c_ref_ipv6Available

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

/* cvar.c / cmd.c (Phase 7 M2): rename every public symbol so the reference
 * registries link next to the Rust cvar/cmd shims (quake-capi's `cvar`
 * feature exports the same plain names). Placed before cfgfile.c's block
 * below because cfgfile.c calls plain Cvar_Set, and the shared prelude's
 * object-like macros must be defined before that text is read for the
 * rename to apply there too -- the same reasoning holds for common_fs.c's
 * Cvar_RegisterVariable/Cmd_AddCommand and snd_dma.c's Cvar_SetQuick /
 * Cvar_SetCallback call sites: they are rewritten by these same defines. */
#define CMDLINE_LENGTH 256 /* quakedef.h; cmd.c's Cmd_StuffCmds_f needs it */

#define Cvar_List_f			 c_ref_Cvar_List_f
#define Cvar_Inc_f			 c_ref_Cvar_Inc_f
#define Cvar_Set_f			 c_ref_Cvar_Set_f
#define Cvar_Toggle_f		 c_ref_Cvar_Toggle_f
#define Cvar_Cycle_f		 c_ref_Cvar_Cycle_f
#define Cvar_Reset_f		 c_ref_Cvar_Reset_f
#define Cvar_ResetAll_f		 c_ref_Cvar_ResetAll_f
#define Cvar_ResetCfg_f		 c_ref_Cvar_ResetCfg_f
#define Cvar_Init			 c_ref_Cvar_Init
#define Cvar_FindVar		 c_ref_Cvar_FindVar
#define Cvar_FindVarAfter	 c_ref_Cvar_FindVarAfter
#define Cvar_LockVar		 c_ref_Cvar_LockVar
#define Cvar_UnlockVar		 c_ref_Cvar_UnlockVar
#define Cvar_UnlockAll		 c_ref_Cvar_UnlockAll
#define Cvar_VariableValue	 c_ref_Cvar_VariableValue
#define Cvar_VariableString c_ref_Cvar_VariableString
#define Cvar_CompleteVariable c_ref_Cvar_CompleteVariable
#define Cvar_Reset			 c_ref_Cvar_Reset
#define Cvar_SetQuick		 c_ref_Cvar_SetQuick
#define Cvar_SetValueQuick	 c_ref_Cvar_SetValueQuick
#define Cvar_Set			 c_ref_Cvar_Set
#define Cvar_SetValue		 c_ref_Cvar_SetValue
#define Cvar_SetROM			 c_ref_Cvar_SetROM
#define Cvar_SetValueROM	 c_ref_Cvar_SetValueROM
#define Cvar_RegisterVariable c_ref_Cvar_RegisterVariable
#define Cvar_Create			 c_ref_Cvar_Create
#define Cvar_SetCallback	 c_ref_Cvar_SetCallback
#define Cvar_SetCompletion	 c_ref_Cvar_SetCompletion
#define Cvar_Command		 c_ref_Cvar_Command
#define Cvar_WriteVariables	 c_ref_Cvar_WriteVariables

#define cl_nopext	 c_ref_cl_nopext
#define cmd_warncmd	 c_ref_cmd_warncmd
#define cmd_text	 c_ref_cmd_text
#define cmd_alias	 c_ref_cmd_alias
#define cmd_source	 c_ref_cmd_source
#define cmd_functions c_ref_cmd_functions
#define Cmd_Wait_f			 c_ref_Cmd_Wait_f
#define Cbuf_Init			 c_ref_Cbuf_Init
#define Cbuf_AddText		 c_ref_Cbuf_AddText
#define Cbuf_AddTextLen		 c_ref_Cbuf_AddTextLen
#define Cbuf_InsertText		 c_ref_Cbuf_InsertText
#define Cbuf_Waited			 c_ref_Cbuf_Waited
#define Cbuf_Execute		 c_ref_Cbuf_Execute
#define Cmd_StuffCmds_f		 c_ref_Cmd_StuffCmds_f
#define Cmd_Exec_f			 c_ref_Cmd_Exec_f
#define Cmd_Echo_f			 c_ref_Cmd_Echo_f
#define Cmd_Alias_f			 c_ref_Cmd_Alias_f
#define Cmd_Unalias_f		 c_ref_Cmd_Unalias_f
#define Cmd_AliasExists		 c_ref_Cmd_AliasExists
#define Cmd_Unaliasall_f	 c_ref_Cmd_Unaliasall_f
#define Cmd_List_f			 c_ref_Cmd_List_f
#define Cmd_Apropos_f		 c_ref_Cmd_Apropos_f
#define Cmd_Init			 c_ref_Cmd_Init
#define Cmd_Argc			 c_ref_Cmd_Argc
#define Cmd_Argv			 c_ref_Cmd_Argv
#define Cmd_Args			 c_ref_Cmd_Args
#define Cmd_TokenizeString	 c_ref_Cmd_TokenizeString
#define Cmd_AddArg			 c_ref_Cmd_AddArg
#define Cmd_AddCommand2		 c_ref_Cmd_AddCommand2
#define Cmd_RemoveCommand	 c_ref_Cmd_RemoveCommand
#define Cmd_FindCommand		 c_ref_Cmd_FindCommand
#define Cmd_IsReservedName	 c_ref_Cmd_IsReservedName
#define Cmd_Exists			 c_ref_Cmd_Exists
#define Cmd_CompleteCommand c_ref_Cmd_CompleteCommand
#define Cmd_ExecuteString	 c_ref_Cmd_ExecuteString
#define Cmd_ForwardToServer c_ref_Cmd_ForwardToServer
#define Cmd_CheckParm		 c_ref_Cmd_CheckParm

/* cvar.h/cmd.h are clean (only depend on q_types.h's qboolean, already
 * included above); pulling them in this early means every later forward
 * declaration of a renamed name (cfgfile.c's Cvar_Set below, common_fs.c's
 * Cvar_RegisterVariable/Cmd_AddCommand, snd_dma.c's Cvar_SetQuick /
 * Cvar_SetCallback) type-checks against the real prototype instead of a
 * hand-rolled stand-in. */
#include "cvar.h"
#include "cmd.h"

/* quakedef.h globals cvar.c/cmd.c read directly (definitions in stubs.c) */
extern qboolean host_initialized; // true once command execution is live;
								   // gates Cvar_RegisterVariable's dynamic
								   // vs static name-copy strategy and
								   // Cvar_SetQuick's default_string update
extern void PR_AutoCvarChanged (cvar_t *var);				  // pr_ext.c
extern void Info_SetKey (char *info, size_t infosize, const char *key, const char *val); // common.c

/* cfgfile.c */
#define CFG_OpenConfig		 c_ref_CFG_OpenConfig
#define CFG_CloseConfig		 c_ref_CFG_CloseConfig
#define CFG_ReadCvars		 c_ref_CFG_ReadCvars
#define CFG_ReadCvarOverrides c_ref_CFG_ReadCvarOverrides
#define CONFIG_NAME			 "vkQuake.cfg"

void Con_Printf (const char *fmt, ...);
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
/* render.h's efrag_t, transcribed (render.h itself #includes tasks.h ->
 * q_stdinc.h -> SDL.h). quake_types::host::Efrag mirrors it field-by-field,
 * and abi_probe.c's host probe takes its offsetof()s from this definition;
 * it reaches entity_t only through a pointer, so it can sit here, ahead of
 * the entity_t transcription further down (which needs protocol.h's
 * entity_state_t). */
typedef struct efrag_s
{
	struct efrag_s	*leafnext;
	struct entity_s *entity;
} efrag_t;
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

/* ---- Phase 6: the progs VM oracles (pr_edict_arena.c, pr_exec.c) ----
 * pr_edict_arena.c was split verbatim out of pr_edict.c so the differential
 * suites get a small stub surface. Only symbols those two files *define* are
 * renamed: an object-like macro for an ambient like `qcvm` would also rewrite
 * `sv.qcvm`, the struct field PR_Profile_f dereferences. The ambients below
 * are stub-owned and keep their real names, which is also what the quake-capi
 * progs shims will import. */
#define ED_AllocSetHook       c_ref_ED_AllocSetHook
#define ED_Alloc              c_ref_ED_Alloc
#define ED_Free               c_ref_ED_Free
#define ED_RemoveFromFreeList c_ref_ED_RemoveFromFreeList
#define ED_CheckFreeList      c_ref_ED_CheckFreeList
#define ED_RebuildFreeList    c_ref_ED_RebuildFreeList
#define PR_GetString          c_ref_PR_GetString
#define PR_ClearEngineString  c_ref_PR_ClearEngineString
#define PR_SetEngineString    c_ref_PR_SetEngineString
#define PR_AllocString        c_ref_PR_AllocString
#define PR_ClearEdictStrings  c_ref_PR_ClearEdictStrings
#define PR_ExecuteProgram     c_ref_PR_ExecuteProgram
#define PR_RunError           c_ref_PR_RunError
#define PR_RunWarning         c_ref_PR_RunWarning
#define PR_Profile_f          c_ref_PR_Profile_f
#define PR_UglyValueString    c_ref_PR_UglyValueString
#define ED_Write              c_ref_ED_Write
#define ED_WriteGlobals       c_ref_ED_WriteGlobals
#define ED_FieldAtOfs         c_ref_ED_FieldAtOfs
#define type_size             c_ref_type_size
#define ED_NewString          c_ref_ED_NewString
#define ED_ParseEpair         c_ref_ED_ParseEpair
#define ED_FindField          c_ref_ED_FindField
#define ED_FindFunction       c_ref_ED_FindFunction

/* ---- Phase 7 M3: world.c, the world-query oracle ----
 *
 * Every public symbol world.c defines is renamed c_ref_*; the Rust port
 * (quake-capi's `host` feature) exports the same plain names beside it. The
 * block sits HERE, above the prelude's own `void SV_UnlinkEdict (edict_t *)`
 * forward declaration below and above every `#include "world.h"`, so the
 * declarations rename with the definitions -- and so pr_edict_arena.c's
 * ED_Free call site reaches the real, renamed world.c function instead of
 * the hand-written stub that used to stand in for it (removed from stubs.c).
 *
 * world.c's file statics (box_hull/box_clipnodes/box_planes,
 * SV_AreaTriggerEdicts, SV_TouchLinks, SV_SlowRecursiveHullCheck,
 * SV_ClipToLinks, World_ClipToNetwork) need no rename. The two cvars DO:
 * world_glue.c defines them under the plain names for the Rust side.
 * SV_PushGridEntityLinked is sv_phys.c's and is renamed with the rest of
 * that file in the Phase 7 M4 block below (it was stub-owned through M3,
 * when sv_phys.c was not yet one of build.rs's C_SOURCES). */
#define sv_fte_recursivehullckeck c_ref_sv_fte_recursivehullckeck
#define sv_fte_createareanode	  c_ref_sv_fte_createareanode
#define SV_InitBoxHull			  c_ref_SV_InitBoxHull
#define SV_HullForBox			  c_ref_SV_HullForBox
#define SV_HullForEntity		  c_ref_SV_HullForEntity
#define SV_CreateAreaNode		  c_ref_SV_CreateAreaNode
#define SV_ClearWorld			  c_ref_SV_ClearWorld
#define SV_UnlinkEdict			  c_ref_SV_UnlinkEdict
#define SV_FindTouchedLeafs		  c_ref_SV_FindTouchedLeafs
#define SV_LinkEdict			  c_ref_SV_LinkEdict
#define SV_HullPointContents	  c_ref_SV_HullPointContents
#define SV_PointContents		  c_ref_SV_PointContents
#define SV_TruePointContents	  c_ref_SV_TruePointContents
#define SV_PointContentsAllBsps	  c_ref_SV_PointContentsAllBsps
#define SV_TestEntityPosition	  c_ref_SV_TestEntityPosition
#define Q1BSP_RecursiveHullTrace  c_ref_Q1BSP_RecursiveHullTrace
#define SV_RecursiveHullCheck	  c_ref_SV_RecursiveHullCheck
#define SV_ClipMoveToEntity		  c_ref_SV_ClipMoveToEntity
#define SV_MoveBounds			  c_ref_SV_MoveBounds
#define SV_Move					  c_ref_SV_Move

/* ---- Phase 7 M4: sv_move.c + sv_phys.c, the monster-move / physics oracle --
 *
 * Same shape as the M3 world.c block above: every public symbol the two
 * files *define* is renamed c_ref_*, and the Rust port (quake-capi's `host`
 * feature) exports the same plain names beside it. The block sits above the
 * `#include "world.h"` further down, so world.h's
 * `void SV_PushGridEntityLinked (edict_t *)` declaration -- and world.c's
 * call to it -- rename together with sv_phys.c's definition.
 *
 * sv_move.c's only file-local data is `static int c_yes, c_no;`
 * (sv_move.c:37); sv_phys.c's file statics (pushable_ent_cache, push_grid_*,
 * sv_pusher_support*, sv_walk_support_*) likewise. Neither needs a rename.
 * Everything sv_phys.c defines with *external* linkage does, per this
 * header's invariant below: the twelve cvars (sv_main.c registers them by
 * address), sv_analyticphysics_frame (sv_user.c reads it), and the seven
 * sv_speeds_* counters (host.c reads and zeroes them). In the shipping build
 * those objects move to Quake/sv_phys_glue.c under the plain names;
 * sv_phys_glue.c is not one of build.rs's C_SOURCES, so stubs.c owns the
 * plain-named copies for the Rust side -- exactly as it already does for
 * world.c's two sv_fte_* cvars.
 *
 * `extern cvar_t sv_speeds;` (sv_phys.c:343) is host.c's, not sv_phys.c's,
 * so it stays stub-owned and shared un-renamed between the two sides. */
#define SV_CheckBottom					 c_ref_SV_CheckBottom
#define SV_movestep						 c_ref_SV_movestep
#define SV_StepDirection				 c_ref_SV_StepDirection
#define SV_FixCheckBottom				 c_ref_SV_FixCheckBottom
#define SV_NewChaseDir					 c_ref_SV_NewChaseDir
#define SV_CloseEnough					 c_ref_SV_CloseEnough
#define SV_MoveToGoal					 c_ref_SV_MoveToGoal
#define SV_PushGridEntityLinked			 c_ref_SV_PushGridEntityLinked
#define SV_CheckAllEnts					 c_ref_SV_CheckAllEnts
#define SV_CheckVelocity				 c_ref_SV_CheckVelocity
#define SV_CheckWaterTransition			 c_ref_SV_CheckWaterTransition
#define SV_Physics						 c_ref_SV_Physics
#define sv_friction						 c_ref_sv_friction
#define sv_stopspeed					 c_ref_sv_stopspeed
#define sv_gravity						 c_ref_sv_gravity
#define sv_maxvelocity					 c_ref_sv_maxvelocity
#define sv_nostep						 c_ref_sv_nostep
#define sv_freezenonclients				 c_ref_sv_freezenonclients
#define sv_gameplayfix_spawnbeforethinks c_ref_sv_gameplayfix_spawnbeforethinks
#define sv_gameplayfix_bouncedownslopes	 c_ref_sv_gameplayfix_bouncedownslopes
#define sv_gameplayfix_elevators		 c_ref_sv_gameplayfix_elevators
#define sv_fastpushmove					 c_ref_sv_fastpushmove
#define sv_pushgrid						 c_ref_sv_pushgrid
#define sv_analyticphysics				 c_ref_sv_analyticphysics
#define sv_analyticphysics_frame		 c_ref_sv_analyticphysics_frame
#define sv_speeds_think_ms				 c_ref_sv_speeds_think_ms
#define sv_speeds_pusher_ms				 c_ref_sv_speeds_pusher_ms
#define sv_speeds_build_ms				 c_ref_sv_speeds_build_ms
#define sv_speeds_thinks				 c_ref_sv_speeds_thinks
#define sv_speeds_pushers				 c_ref_sv_speeds_pushers
#define sv_speeds_pushables				 c_ref_sv_speeds_pushables
#define sv_speeds_grid_entries			 c_ref_sv_speeds_grid_entries

#include "protocol.h" /* entity_state_t, which edict_t embeds */
#include "progs.h"
#include "pr_trace.h" /* the PR_TRACE_* hooks; no-ops without -Dtrace=true */

/* stub-owned (stubs.c): the ambient VM, the engine seams pr_edict.c keeps,
 * and protocol.h's null baseline. The prelude pre-empts protocol.h and
 * server.h, so these come by hand. */
extern qcvm_t		 *qcvm;
extern globalvars_t  *pr_global_struct;
extern entity_state_t nullentitystate;
edict_t				 *EDICT_NUM (int n);
int					  NUM_FOR_EDICT (edict_t *e);
void				  SV_UnlinkEdict (edict_t *ent);
void				  ED_Print (edict_t *ed);
void				  PR_SwitchQCVM (qcvm_t *nvm);
const char			 *PR_GlobalString (int ofs);
const char			 *PR_GlobalStringNoContents (int ofs);
ddef_t				 *ED_FieldAtOfs (int ofs);
extern const int	  type_size[NUM_TYPE_SIZES];
ddef_t				 *ED_FindField (const char *name);
dfunction_t			 *ED_FindFunction (const char *fn_name);

/* server.h slice: model_parse.c reads sv.modelname; snd_mix.c (Phase 4)
 * reads sv.active */
typedef enum
{
	ss_loading,
	ss_active
} server_state_t; /* server.h; pr_exec.c's OP_ADDRESS guard reads it */
typedef struct
{
	char		   modelname[64];
	qboolean	   active;
	char		   name[64];  /* Phase 5: net_loop.c's Loop_SearchForHosts */
	server_state_t state;	  /* Phase 6 M3: pr_exec.c's OP_ADDRESS guard */
	qcvm_t		   qcvm;	  /* Phase 6 M3: pr_exec.c's PR_Profile_f */
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

/* ---- Phase 7 M3: the quakedef.h/server.h/render.h slice world.c needs ----
 *
 * quakedef.h's DIST_EPSILON and assert_always (quakedef.h:66, :335). The
 * assert_always guard is spelled exactly as the engine's so a later real
 * quakedef.h slice cannot silently disagree. */
#ifndef DIST_EPSILON
#define DIST_EPSILON (0.03125) // 1/32 epsilon to keep floating point happy (moved from world.c)
#endif
FUNC_NORETURN void COM_Assert_Failed (const char *expr, const char *file, int line);
#ifndef assert_always
#define assert_always(e) ((e) ? (void)0 : COM_Assert_Failed (#e, __FILE__, __LINE__))
#endif

/* server.h's movetype/solid/edict-flag enumerators world.c compares against.
 * They are object-like macros rather than an enum so abi_probe.c -- the one
 * TU that #includes the real server.h -- can #undef them out of the way
 * before that include, the same dodge it already uses for sv/svs/cl/cls.
 * Values transcribed from Quake/server.h:227-285. Phase 7 M4 widened the
 * slice to everything sv_move.c and sv_phys.c compare against; abi_probe.c
 * #undef's the whole set. */
#define MOVETYPE_NONE		0
#define MOVETYPE_ANGLENOCLIP 1
#define MOVETYPE_ANGLECLIP	2
#define MOVETYPE_WALK		3
#define MOVETYPE_STEP		4
#define MOVETYPE_FLY		5
#define MOVETYPE_TOSS		6
#define MOVETYPE_PUSH		7
#define MOVETYPE_NOCLIP		8
#define MOVETYPE_FLYMISSILE 9
#define MOVETYPE_BOUNCE		10
#define MOVETYPE_GIB		11
#define SOLID_NOT			0
#define SOLID_TRIGGER		1
#define SOLID_BBOX			2
#define SOLID_SLIDEBOX		3
#define SOLID_BSP			4
#define FL_FLY				1
#define FL_SWIM				2
#define FL_CONVEYOR			4
#define FL_CLIENT			8
#define FL_INWATER			16
#define FL_MONSTER			32
#define FL_GODMODE			64
#define FL_NOTARGET			128
#define FL_ITEM				256
#define FL_ONGROUND			512
#define FL_PARTIALGROUND	1024
#define FL_WATERJUMP		2048
#define FL_JUMPRELEASED		4096

/* quakedef.h:68 -- SV_AddGravity/SV_FinishGravity's analytic half-step
 * (sv_phys.c:682,693) divides by it, and the comment on that line says the
 * value must not change. */
#ifndef MAX_PHYSICS_FREQ
#define MAX_PHYSICS_FREQ (72.0)
#endif

/* Phase 7 M4: the three engine seams sv_phys.c reaches that no header in
 * this slice declares. All three are stub-owned (sv_main.c and host.c are
 * not in build.rs's C_SOURCES) and shared un-renamed, so the c_ref oracle
 * and the Rust port drive the same recorder:
 *   - SV_StartSound  (server.h:338, defined sv_main.c:1274) -- water-entry
 *     and hit-ground sounds; Host_Error's three ways, hence the guarded
 *     glue helper in stubs.c
 *   - Host_EndGame   (quakedef.h:480, defined host.c:185) -- the two
 *     bad-movetype sites
 *   - sv_player      (server.h:331, defined sv_main.c) -- SV_WalkMove's
 *     FL_WATERJUMP test (sv_phys.c:1893) */
void SV_StartSound (edict_t *entity, float *origin, int channel, const char *sample, int volume, float attenuation);
FUNC_NORETURN void Host_EndGame (const char *message, ...) FUNC_PRINTF (1, 2);
extern edict_t *sv_player;

/* Phase 7 M4: sv_phys.c's own public cvars, its per-frame analytic latch and
 * the sv_speeds counters. server.h:314-329 and host.c declare these in the
 * engine; this prelude pre-empts server.h, so they come by hand. Every name
 * here is rewritten to c_ref_* by the M4 rename block above, which is what
 * lets stubs.c hold a second, plain-named set for the Rust port (the copies
 * Quake/sv_phys_glue.c owns in the shipping build). */
extern cvar_t sv_friction;
extern cvar_t sv_stopspeed;
extern cvar_t sv_gravity;
extern cvar_t sv_maxvelocity;
extern cvar_t sv_nostep;
extern cvar_t sv_freezenonclients;
extern cvar_t sv_gameplayfix_spawnbeforethinks;
extern cvar_t sv_gameplayfix_bouncedownslopes;
extern cvar_t sv_gameplayfix_elevators;
extern cvar_t sv_fastpushmove;
extern cvar_t sv_pushgrid;
extern cvar_t sv_analyticphysics;
extern qboolean sv_analyticphysics_frame;
extern double	sv_speeds_think_ms, sv_speeds_pusher_ms, sv_speeds_build_ms;
extern int		sv_speeds_thinks, sv_speeds_pushers, sv_speeds_pushables, sv_speeds_grid_entries;

/* host.c:70 -- read by sv_phys.c's timing blocks and NOT renamed: there is
 * one such cvar in the engine and stubs.c owns the single definition. */
extern cvar_t sv_speeds;

/* pr_ext.c's extension-enable cvar. world.c branches on it in five places
 * (SV_HullForEntity, SV_CreateAreaNode, SV_LinkEdict, SV_RecursiveHullCheck,
 * SV_ClipMoveToEntity), so it is stub-owned and shared un-renamed: the Rust
 * port reads the same object through quake-c-sys. */
extern cvar_t pr_checkextension;

/* render.h's entity_t (and the two structs it embeds by value), transcribed:
 * render.h #includes tasks.h -> q_stdinc.h -> SDL.h, which this build must
 * stay clear of. World_ClipToNetwork walks cl.entities as entity_t, so the
 * layout has to be the real one -- the Rust side sees the same objects
 * through world_glue.c's World_Glue_ClEntity accessor. This transcription is
 * also what abi_probe.c's Phase 7 host probe measures (it used to carry its
 * own private copy; that was moved here so both TUs cannot drift apart).
 * render.h carries the codebase's "!!! if this is changed, it must be
 * changed in rust/quake-ctest/stubs/abi_probe.c too !!!" markers on
 * entity_s/entlerp_s/lightcache_s. */
typedef struct lightcache_s
{
	int	   surfidx;
	vec3_t pos;
	short  ds;
	short  dt;
} lightcache_t;

typedef struct entlerp_s
{
	qboolean movestep;
	int		 prev_frame;
	double	 frame_change_time;
	double	 frame_duration;
	double	 frame_finish_time;
	int		 snap_frames;
	double	 snap_msgtime;
	vec3_t	 prev_origin;
	vec3_t	 prev_angles;
	double	 move_change_time;
	double	 move_duration;
} entlerp_t;

typedef struct entity_s
{
	qboolean forcelink;

	int update_type;

	entity_state_t baseline;
	entity_state_t netstate;

	double			 msgtime;
	vec3_t			 msg_origins[2];
	vec3_t			 origin;
	vec3_t			 msg_angles[2];
	vec3_t			 angles;
	struct qmodel_s *model;
	struct efrag_s	*efrag;
	int				 frame;
	float			 syncbase;
	byte			*colormap;
	int				 effects;
	int				 skinnum;
	int				 visframe;

	int dlightframe;
	int dlightbits;

	struct mnode_s *topnode;

	byte	  eflags;
	byte	  alpha;
	entlerp_t lerp;

#ifdef PSET_SCRIPT
	struct trailstate_s *trailstate;
	struct trailstate_s *emitstate;
#endif
	float  traildelay;
	vec3_t trailorg;

	lightcache_t lightcache;

	int	   contentscache;
	vec3_t contentscache_origin;

	struct entity_blas_s *blas_data;
} entity_t;

#include "world.h" /* the renames above are already in effect */

/* the quakedef.h slice snd_mix.c's pause_loops computation reads, plus the
 * three client_state_t members world.c touches: SV_Move's `qcvm == &cl.qcvm`
 * CSQC test and World_ClipToNetwork's cl.entities/cl.num_entities walk. The
 * stub definitions expose setters for the differential tests. */
typedef struct
{
	qboolean  paused;
	int		  viewentity;
	qmodel_t *worldmodel;
	int		  num_entities;
	entity_t *entities;
	qcvm_t	  qcvm;
} ctest_cl_t;
extern ctest_cl_t cl;
/* ctest_svs_t (needs client_t, which needs sizebuf_t from net.h) is defined
 * further down, after the net.h include block. */
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
/* ctest_cls_stub_t is defined further down (needs sizebuf_t from net.h for
 * cls.message). */
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
void					Con_SafePrintf (const char *fmt, ...);

/* Phase 7 M2: the server.h/client.h slice cvar.c's CVAR_SERVERINFO /
 * CVAR_USERINFO replication blocks and cmd.c's Cmd_ExecuteString /
 * Cmd_ForwardToServer read. Stub-owned mirror structs, like ctest_cl_t
 * above; definitions/instances live in stubs.c. */
/* Named client_t/client_s (not ctest_-prefixed like ctest_svs_t below):
 * cvar.c's own CVAR_SERVERINFO replication block (cvar.c:513) declares a
 * local `client_t *current_client` by that literal name, so the type must
 * exist under this exact name for cvar.c's unmodified source to compile.
 * abi_probe.c separately #includes the real server.h/client.h for its own
 * struct-layout ABI probe further down in that file; it #define/#undef's
 * this name (and svs/sv/cl/cls, same idiom) out of the way for just those
 * two #includes to avoid a duplicate-definition error in that TU. */
typedef struct client_s
{
	qboolean  active;
	sizebuf_t message;
	char	  name[32];
	/* Phase 7 M4: SV_Physics_Client (sv_phys.c:1996-2000) gates on
	 * svs.clients[num-1].active and .knowntoqc */
	qboolean knowntoqc;
} client_t;
extern client_t *host_client; // valid only while cmd_source == src_client

typedef struct
{
	int		  maxclients;
	char	  serverinfo[512];
	client_t *clients;
} ctest_svs_t;
extern ctest_svs_t svs;

typedef struct
{
	cactive_t state;
	int		  signon;
	int		  demonum;
	qboolean  demoplayback;
	char	  userinfo[512];
	sizebuf_t message;
} ctest_cls_stub_t;
extern ctest_cls_stub_t cls;

extern cvar_t cl_name;
extern cvar_t cl_topcolor;
extern cvar_t cl_bottomcolor;

#endif /* C_REF_PRELUDE_H */
