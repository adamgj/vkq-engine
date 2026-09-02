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
/* atomics.h:53-56, same single-threaded rationale as the store above. */
static inline uint32_t Atomic_LoadUInt32 (volatile atomic_uint32_t *atomic)
{
	return atomic->value;
}
/* atomics.h:157-172, minus the _ReadBarrier/_WriteBarrier intrinsics the
 * real header pairs them with: this harness is single-threaded, so the
 * barriers have nothing to order. Same 8-byte layout. */
typedef struct
{
	void *volatile value;
} atomic_ptr_t;
static inline void *Atomic_LoadPtr (volatile atomic_ptr_t *atomic)
{
	return atomic->value;
}
static inline void Atomic_StorePtr (volatile atomic_ptr_t *atomic, void *desired)
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
#define TEXPREF_MIPMAP	 0x0001
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

/* server_state_t / server_t / `sv` used to be hand-cut stand-ins here.
 * Phase 7 M6 replaced them with the real Quake/server.h, included at the
 * bottom of this header (everything server.h needs -- qcvm_t, sizebuf_t,
 * entity_state_t, usercmd_t, cvar_t, struct qsocket_s -- is only in scope
 * by then). */

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

/* server.h's movetype/solid/edict-flag enumerators used to be transcribed
 * here as object-like macros, because quakedef.h is neutered and world.c /
 * sv_move.c / sv_phys.c never saw the real enums. Phase 7 M6 includes the
 * real server.h at the bottom of this header, so the enumerators themselves
 * are in scope and the transcription (and abi_probe.c's #undef dodge around
 * it) is gone. */

/* quakedef.h:68 -- SV_AddGravity/SV_FinishGravity's analytic half-step
 * (sv_phys.c:682,693) divides by it, and the comment on that line says the
 * value must not change. */
#ifndef MAX_PHYSICS_FREQ
#define MAX_PHYSICS_FREQ (72.0)
#endif

/* Phase 7 M4/M6: Host_EndGame (quakedef.h:480, defined host.c:185) is the
 * one seam of sv_phys.c's bad-movetype sites that stays stub-owned -- host.c
 * is not in build.rs's C_SOURCES. SV_StartSound and sv_player used to be
 * declared (and stub-defined) here for the same reason; M6 pulled sv_main.c
 * and sv_user.c into C_SOURCES, so both are now real, renamed c_ref_* below
 * and declared by the real server.h. */
FUNC_NORETURN void Host_EndGame (const char *message, ...) FUNC_PRINTF (1, 2);

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

/* `cl` used to be a six-field ctest_cl_t stand-in here. Phase 7 M6 replaced
 * it with the real client_state_t from Quake/client.h, included at the
 * bottom of this header; cl_main.c is still not in C_SOURCES, so the
 * instance itself stays stub-owned (stubs.c) and un-renamed. */
/* keydest_t and key_dest used to be transcribed here; Phase 7 M7 includes
 * the real Quake/keys.h below (cl_input.c and cl_main.c need the rest of it
 * anyway), so this is just host_frametime now. */
extern double host_frametime;

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
/* cactive_t / client_static_t come from the real client.h, at the bottom. */
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

/* ---- Phase 7 M6: the real Quake/server.h and Quake/client.h --------------
 *
 * Through M5 this header carried hand-cut stand-ins under the engine's own
 * names -- ctest_server_stub_t `sv`, a four-field `client_t`, ctest_svs_t
 * `svs`, ctest_cl_t `cl`, ctest_cls_stub_t `cls`. M6 puts sv_main.c,
 * sv_send.c and sv_user.c into build.rs's C_SOURCES, and those three touch
 * essentially every field of the real structs, so the stand-ins are gone and
 * the engine's own headers are included here instead. Every earlier oracle's
 * reader (sv.modelname, sv.active, sv.name, sv.state, sv.qcvm,
 * svs.maxclients, svs.serverinfo, svs.clients, cls.state, cls.signon, ...)
 * names a field that exists on the real struct, so they compile unchanged.
 *
 * abi_probe.c used to #include these two headers itself, behind a
 * #define/#undef rename dodge, precisely because this header occupied the
 * four names. That dodge is gone with the stand-ins; the probe now measures
 * the same declarations every other oracle TU sees.
 *
 * This is the last thing the header does: server.h needs qcvm_t (progs.h),
 * entity_state_t/usercmd_t (protocol.h), sizebuf_t (common.h), cvar_t
 * (cvar.h) and struct qsocket_s (net_defs.h); client.h additionally embeds
 * entity_t (the render.h transcription above) by value in `viewent`, and
 * needs qfileofs_t (sys.h, via common.h) and FILE.
 *
 * quakedef.h constants the two headers expect from their normal
 * #include "quakedef.h" chain, which this header pre-empts. MAX_SOUNDS,
 * MAX_LIGHTSTYLES, MAX_DATAGRAM, MAX_MSGLEN, MAX_EDICTS and SIGNONS are
 * already defined above by the earlier phases' slices;
 * SERVER_INFO_STRING_SIZE, CLIENT_USER_INFO_STRING_SIZE, NUM_PING_TIMES,
 * NUM_TOTAL_SPAWN_PARMS, NUM_CSHIFTS, MAX_MAPSTRING, MAX_DEMOS and
 * MAX_DEMONAME are #define'd by server.h/client.h themselves. The values
 * below are gated against the real quakedef.h by a _Static_assert in
 * Quake/harness.c. */
#include <stdio.h> /* client_static_t::demofile */

#define MAX_MODELS		   8192
#define MAX_PARTICLETYPES  2048
#define MAX_STYLESTRING	   64
#define MAX_SCOREBOARDNAME 32
#define VID_CBITS		   6
#define VID_GRADES		   (1 << VID_CBITS)

/* Phase 7 M6 renames. sv_main.c, sv_send.c and sv_user.c are oracle sources
 * now, so every global they define is c_ref_*; the block has to precede the
 * two #includes below so server.h's own declarations rename with them.
 *
 * `sv`, `svs` and `sv_player` move with their files: stubs.c no longer
 * defines them, the oracle does, and the Rust-side glue in stubs.c reaches
 * the same storage through these macros. ADR-007 makes sv/svs Rust-owned at
 * M6-T6.6; until that storage move lands, the C oracle owns the one copy.
 *
 * SV_StartSound is the one M4 seam this flips: it was a stub-owned recorder
 * shared by both sides, and it is sv_main.c's function, so from M6 it is the
 * real implementation on the oracle side and for the glue that drives the
 * Rust side. See stubs.c's ctest_phys_sound_* block. */

/* sv_main.c */
#define sv					   c_ref_sv
#define svs					   c_ref_svs
#define sv_protocol			   c_ref_sv_protocol
#define sv_protocol_pext1	   c_ref_sv_protocol_pext1
#define sv_protocol_pext2	   c_ref_sv_protocol_pext2
#define sv_netsort			   c_ref_sv_netsort
#define sv_smoothplatformlerps c_ref_sv_smoothplatformlerps
#define SV_Init				   c_ref_SV_Init
#define SV_StartParticle	   c_ref_SV_StartParticle
#define SV_StartSound		   c_ref_SV_StartSound
#define SV_LocalSound		   c_ref_SV_LocalSound
#define SV_SendServerinfo	   c_ref_SV_SendServerinfo
#define SV_ConnectClient	   c_ref_SV_ConnectClient
#define SV_CheckForNewClients  c_ref_SV_CheckForNewClients
#define SV_ClearDatagram	   c_ref_SV_ClearDatagram
#define SV_ModelIndex		   c_ref_SV_ModelIndex
#define SV_SaveSpawnparms	   c_ref_SV_SaveSpawnparms
#define SV_ModelForIndex	   c_ref_SV_ModelForIndex
#define SV_SpawnServer		   c_ref_SV_SpawnServer

/* sv_send.c */
#define SV_CalcStats					 c_ref_SV_CalcStats
#define SVFTE_DestroyFrames				 c_ref_SVFTE_DestroyFrames
#define SVFTE_SetupFrames				 c_ref_SVFTE_SetupFrames
#define SVFTE_Ack						 c_ref_SVFTE_Ack
#define SV_BuildEntityState				 c_ref_SV_BuildEntityState
#define MSG_WriteStaticOrBaseLine		 c_ref_MSG_WriteStaticOrBaseLine
#define SV_AddToFatPVS					 c_ref_SV_AddToFatPVS
#define SV_FatPVS						 c_ref_SV_FatPVS
#define SV_VisibleToClient				 c_ref_SV_VisibleToClient
#define SV_WriteEntitiesToClient		 c_ref_SV_WriteEntitiesToClient
#define SV_CleanupEnts					 c_ref_SV_CleanupEnts
#define SV_WriteDamageToMessage			 c_ref_SV_WriteDamageToMessage
#define SV_WriteClientdataToMessage		 c_ref_SV_WriteClientdataToMessage
#define SV_PresendClientDatagram		 c_ref_SV_PresendClientDatagram
#define SV_SendClientDatagram			 c_ref_SV_SendClientDatagram
#define SV_UpdateToReliableMessages		 c_ref_SV_UpdateToReliableMessages
#define SV_SendNop						 c_ref_SV_SendNop
#define SV_SendPrespawnModelPrecaches	 c_ref_SV_SendPrespawnModelPrecaches
#define SV_SendPrespawnSoundPrecaches	 c_ref_SV_SendPrespawnSoundPrecaches
#define SV_SendPrespawnParticlePrecaches c_ref_SV_SendPrespawnParticlePrecaches
#define SV_SendPrespawnStatics			 c_ref_SV_SendPrespawnStatics
#define SV_SendAmbientSounds			 c_ref_SV_SendAmbientSounds
#define SV_SendPrespawnBaselines		 c_ref_SV_SendPrespawnBaselines
#define SV_SendClientMessages			 c_ref_SV_SendClientMessages
#define SV_CreateBaseline				 c_ref_SV_CreateBaseline
#define SV_SendReconnect				 c_ref_SV_SendReconnect

/* sv_user.c */
#define sv_player			 c_ref_sv_player
#define sv_edgefriction		 c_ref_sv_edgefriction
#define sv_idealpitchscale	 c_ref_sv_idealpitchscale
#define sv_altnoclip		 c_ref_sv_altnoclip
#define sv_maxspeed			 c_ref_sv_maxspeed
#define sv_accelerate		 c_ref_sv_accelerate
#define SV_SetIdealPitch	 c_ref_SV_SetIdealPitch
#define SV_UserFriction		 c_ref_SV_UserFriction
#define SV_Accelerate		 c_ref_SV_Accelerate
#define SV_AirAccelerate	 c_ref_SV_AirAccelerate
#define DropPunchAngle		 c_ref_DropPunchAngle
#define SV_WaterMove		 c_ref_SV_WaterMove
#define SV_WaterJump		 c_ref_SV_WaterJump
#define SV_NoclipMove		 c_ref_SV_NoclipMove
#define SV_AirMove			 c_ref_SV_AirMove
#define SV_ClientThink		 c_ref_SV_ClientThink
#define SV_ReadClientMove	 c_ref_SV_ReadClientMove
#define SV_ReadClientMessage c_ref_SV_ReadClientMessage
#define SV_RunClients		 c_ref_SV_RunClients

/* ---- Phase 7 M6 seam: the quakedef.h / host.c declarations sv_main.c,
 * sv_send.c and sv_user.c reach that no header in this slice supplies. All
 * of them belong to files that are not oracle sources (host.c above all), so
 * they stay un-renamed and stub-owned; stubs.c defines them. ---- */
#include "quakever.h" /* ENGINE_NAME_AND_VER (sv_main.c:549); self-contained */

void	      Host_Callback_Notify (cvar_t *var);
extern double realtime;
extern int    current_skill;
extern cvar_t max_edicts;

/* quakedef.h:107-134 stat_t, verbatim. It carries MAX_CL_STATS, which
 * server.h and client.h size arrays with. */
/* clang-format off */
typedef enum
{
	MAX_CL_BASE_STATS	= 32,
	MAX_CL_STATS		= 256,

	STAT_HEALTH			= 0,
	STAT_FRAGS			= 1,
	STAT_WEAPON			= 2,
	STAT_AMMO			= 3,
	STAT_ARMOR			= 4,
	STAT_WEAPONFRAME	= 5,
	STAT_SHELLS			= 6,
	STAT_NAILS			= 7,
	STAT_ROCKETS		= 8,
	STAT_CELLS			= 9,
	STAT_ACTIVEWEAPON	= 10,
	STAT_NONCLIENT		= 11,	// first stat not included in svc_clientdata
	STAT_TOTALSECRETS	= 11,
	STAT_TOTALMONSTERS	= 12,
	STAT_SECRETS		= 13,	// bumped on client side by svc_foundsecret
	STAT_MONSTERS		= 14,	// bumped by svc_killedmonster
	STAT_ITEMS			= 15,	//replaces clc_clientdata info
	STAT_VIEWHEIGHT		= 16, // replaces clc_clientdata info
	STAT_VIEWZOOM		= 21, // DP
	STAT_IDEALPITCH		= 25, // nq-emu
	STAT_PUNCHANGLE_X	= 26, // nq-emu
	STAT_PUNCHANGLE_Y	= 27, // nq-emu
	STAT_PUNCHANGLE_Z	= 28, // nq-emu
} stat_t;
/* clang-format on */

/* glquake.h:601-623 -- sv_send.c's developer packet-size stats. glquake.h is
 * Vulkan-dependent, so these are transcribed rather than #included; the
 * instances are stub-owned (gl_screen.c is not an oracle source). */
typedef struct
{
	int packetsize;
	int edicts;
	int visedicts;
	int efrags;
	int tempents;
	int beams;
	int dlights;
} devstats_t;
extern devstats_t dev_stats, dev_peakstats;

typedef struct
{
	double packetsize;
	double efrags;
	double beams;
	double varstring;
} overflowtimes_t;
extern overflowtimes_t dev_overflows;
#define CONSOLE_RESPAM_TIME 3

extern cvar_t devstats;

void SCR_CenterPrintClear (void); /* screen.h:34 */
void Host_ClearMemory (void);	  /* quakedef.h:463 */

#define ON_EPSILON 0.1 /* quakedef.h:64 */

float V_CalcRoll (vec3_t angles, vec3_t velocity); /* view.h:36 */
/* ---- end Phase 7 M6 seam ---- */

/* ---- Phase 7 M7 seam: the client stratum's declarations ------------------
 *
 * chase.c, cl_demo.c, cl_input.c, cl_main.c, cl_parse.c, cl_tent.c and
 * view.c become oracle sources here (task T7.0), one layer up from what T6.0
 * did for the server stratum, and under the same rule: include the REAL
 * engine header wherever it is free of SDL/Vulkan, transcribe only where it
 * is not, and say why.
 *
 * Included below as-is: console.h, screen.h, keys.h, input.h, cdaudio.h,
 * view.h and vid.h carry no #include lines at all, and every type they name
 * is already in scope by this point; harness.h includes only q_types.h.
 *
 * Transcribed here instead, each for a stated reason:
 *  - render.h #includes tasks.h -> q_stdinc.h -> SDL.h, which this build must
 *    stay clear of. Its efrag_t and entity_t are already transcribed above
 *    for exactly that reason; refdef_t and the R_* prototypes the client
 *    calls follow the same way, copied verbatim from Quake/render.h.
 *  - glquake.h #includes tasks.h and is Vulkan-typed throughout.
 *  - quakedef.h is what this whole prelude replaces (it reaches SDL through
 *    q_stdinc.h), so its client-visible slice comes by hand, as every earlier
 *    phase's quakedef.h slice in this header does.
 */

typedef uint64_t task_handle_t; /* tasks.h:32; render.h/view.h take it by value */

#include "vid.h" /* vrect_t (refdef_t embeds two), viddef_t and `vid` */

/* render.h:140-169 refdef_t, verbatim. */
typedef struct
{
	vrect_t vrect;							   // subwindow in video for refresh
											   // FIXME: not need vrect next field here?
	vrect_t aliasvrect;						   // scaled Alias version
	int		vrectright, vrectbottom;		   // right & bottom screen coords
	int		aliasvrectright, aliasvrectbottom; // scaled Alias versions
	float	vrectrightedge;					   // rightmost right edge we care about,
											   //  for use in edge list
	float	fvrectx, fvrecty;				   // for floating-point compares
	float	fvrectx_adj, fvrecty_adj;		   // left and top edges, for clamping
	int		vrect_x_adj_shift20;			   // (vrect.x + 0.5 - epsilon) << 20
	int		vrectright_adj_shift20;			   // (vrectright + 0.5 - epsilon) << 20
	float	fvrectright_adj, fvrectbottom_adj;
	// right and bottom edges, for clamping
	float	fvrectright;		   // rightmost edge, for Alias clamping
	float	fvrectbottom;		   // bottommost edge, for Alias clamping
	float	horizontalFieldOfView; // at Z = 1.0, this many X is visible
								   // 2.0 = 90 degrees
	float	xOrigin;			   // should probably allways be 0.5
	float	yOrigin;			   // between be around 0.3 to 0.5

	vec3_t vieworg;
	vec3_t viewangles;

	float basefov;
	float fov_x, fov_y;

	int ambientlight;
} refdef_t;

/* render.h:200-201 and the render.h prototypes the seven files call. */
extern refdef_t r_refdef;
extern vec3_t	r_origin, vpn, vright, vup;

void R_RenderView (
	qboolean use_tasks, task_handle_t begin_rendering_task, task_handle_t setup_frame_task, task_handle_t draw_done_task); // must set r_refdef first
void R_CheckEfrags (void);
void R_AddEfrags (entity_t *ent);
void R_NewMap (void);
void R_ParseParticleEffect (void);
void R_RunParticleEffect (vec3_t org, vec3_t dir, int color, int count);
void R_RocketTrail (vec3_t start, vec3_t end, int type);
void R_EntityParticles (entity_t *ent);
void R_BlobExplosion (vec3_t org);
void R_ParticleExplosion (vec3_t org);
void R_ParticleExplosion2 (vec3_t org, int colorStart, int colorLength);
void R_LavaSplash (vec3_t org);
void R_TeleportSplash (vec3_t org);

/* glquake.h's client-visible slice. PSET_SCRIPT is defined above (it is
 * unconditional in quakedef.h), so this is glquake.h's PSET_SCRIPT arm:
 * glquake.h:110-135, :542-545, :705-738, :780-781, :810-814. */
void PScript_Shutdown (void);
struct trailstate_s;
int	 PScript_ParticleTrail (vec3_t startpos, vec3_t end, int type, float timeinterval, int dlkey, vec3_t axis[3], struct trailstate_s **tsk);
int	 PScript_RunParticleEffectState (vec3_t org, vec3_t dir, float count, int typenum, struct trailstate_s **tsk);
void PScript_RunParticleWeather (vec3_t minb, vec3_t maxb, vec3_t dir, float count, int colour, const char *efname);
int	 PScript_FindParticleType (const char *fullname);
int	 PScript_RunParticleEffectTypeString (vec3_t org, vec3_t dir, float count, const char *name);
int	 PScript_EntParticleTrail (vec3_t oldorg, entity_t *ent, const char *name);
void PScript_DelinkTrailstate (struct trailstate_s **tsk);
void PScript_ClearParticles (qboolean load);

/* ---- Phase 7 M9f (T9f.0): the pr_ext.c oracle's renderer-facing slice ----
 *
 * stubs/pr_ext_ref.c composes Quake/pr_ext.c into this link (the composing-TU
 * pattern; see that file's header). pr_ext.c reaches a handful of glquake.h /
 * draw.h declarations the real headers cannot supply here, because glquake.h
 * pulls vulkan/vulkan.h and tasks.h. Two kinds of declaration follow, and they
 * are labelled individually:
 *
 *   FAITHFUL   -- character-for-character the real declaration; a Phase 8 port
 *                 can trust it.
 *   COMPILE-ONLY -- a minimal stand-in with the right *spelling* but not the
 *                 real layout. Every consumer of one is Phase 8 renderer code
 *                 (PF_cl_draw*, DrawQC_CharacterQuad, PF_getsurface*) that no
 *                 differential drives and that nothing in this link calls, and
 *                 nothing here takes sizeof() of a COMPILE-ONLY struct.
 *
 * Definitions for the objects declared here live in stubs/pr_ext_ref.c as link
 * doubles, not in a real renderer TU.
 */

/* FAITHFUL: glquake.h:41, :107, :636, :639 */
extern int glwidth, glheight;
#define P_INVALID	   -1
#define LMBLOCK_WIDTH  1024
#define LMBLOCK_HEIGHT 1024

/* COMPILE-ONLY: the Vulkan handles and renderer structs DrawQC_CharacterQuad
 * (pr_ext.c:4885) and PF_cl_drawfill (pr_ext.c:5133) name. The handle types are
 * opaque pointers as vulkan.h defines them; the structs carry only the members
 * pr_ext.c actually touches, in the real spelling. */
typedef uint64_t				 VkDeviceSize;
typedef struct VkBuffer_T		*VkBuffer;
typedef struct VkCommandBuffer_T *VkCommandBuffer;
typedef struct VkPipeline_T		*VkPipeline;
typedef struct VkPipelineLayout_T *VkPipelineLayout;
typedef struct VkRenderPass_T	 *VkRenderPass;
typedef struct VkDescriptorSet_T *VkDescriptorSet;
typedef int						 VkPipelineBindPoint;
#define VK_PIPELINE_BIND_POINT_GRAPHICS 0
typedef struct
{
	int32_t x, y;
} VkOffset2D;
typedef struct
{
	uint32_t width, height;
} VkExtent2D;
typedef struct
{
	VkOffset2D offset;
	VkExtent2D extent;
} VkRect2D;
typedef uint32_t VkShaderStageFlags;
typedef struct
{
	VkShaderStageFlags stageFlags;
	uint32_t		   offset;
	uint32_t		   size;
} VkPushConstantRange;

/* FAITHFUL: glquake.h:152-163 */
typedef struct vulkan_pipeline_layout_s
{
	VkPipelineLayout	handle;
	VkPushConstantRange push_constant_range;
	int					mboit_input_attachment_set;
} vulkan_pipeline_layout_t;
typedef struct vulkan_pipeline_s
{
	VkPipeline				 handle;
	vulkan_pipeline_layout_t layout;
} vulkan_pipeline_t;

/* COMPILE-ONLY: glquake.h:221-239 secondary_cb_contexts_t and :283-... the
 * render_pass_index_t enum, cut down to the members pr_ext.c indexes with.
 * SCBX_GUI's real numeric value is NOT reproduced -- nothing in this link
 * indexes a real context array. */
typedef enum
{
	SCBX_GUI,
	SCBX_NUM
} secondary_cb_contexts_t;
typedef enum
{
	RENDER_PASS_INDEX_MAIN,
	RENDER_PASS_INDEX_UI,
	RENDER_PASS_INDEX_COUNT
} render_pass_index_t;

/* COMPILE-ONLY: glquake.h:339-349. pr_ext.c only ever dereferences `cb`; the
 * real struct also carries `canvastype current_canvas`, `uint32_t
 * vbo_indices[MAX_BATCH_SIZE]` and `unsigned int num_vbo_indices`, which are
 * omitted because MAX_BATCH_SIZE and canvastype are renderer-private and
 * nothing here takes sizeof (cb_context_t). */
typedef struct cb_context_s
{
	VkCommandBuffer	  cb;
	VkRenderPass	  render_pass;
	int				  render_pass_index;
	int				  subpass;
	vulkan_pipeline_t current_pipeline;
} cb_context_t;

/* COMPILE-ONLY: glquake.h:351-... vulkanglobals_t has ~100 members; only the
 * seven pr_ext.c:4940-4947 / :5180-5184 name are mirrored, so this struct's
 * layout is NOT the real one. */
typedef struct
{
	cb_context_t *secondary_cb_contexts[SCBX_NUM];
	void (*vk_cmd_bind_descriptor_sets) (
		VkCommandBuffer, VkPipelineBindPoint, VkPipelineLayout, uint32_t, uint32_t, const VkDescriptorSet *, uint32_t, const uint32_t *);
	void (*vk_cmd_bind_vertex_buffers) (VkCommandBuffer, uint32_t, uint32_t, const VkBuffer *, const VkDeviceSize *);
	void (*vk_cmd_draw) (VkCommandBuffer, uint32_t, uint32_t, uint32_t, uint32_t);
	vulkan_pipeline_t		 basic_alphatest_pipeline[RENDER_PASS_INDEX_COUNT];
	vulkan_pipeline_t		 basic_blend_pipeline[RENDER_PASS_INDEX_COUNT];
	vulkan_pipeline_t		 basic_notex_blend_pipeline[RENDER_PASS_INDEX_COUNT];
	vulkan_pipeline_layout_t basic_pipeline_layout;
} vulkanglobals_t;
extern vulkanglobals_t vulkan_globals;

/* COMPILE-ONLY: gl_texmgr.h's gltexture_s is completed here with just the one
 * member pr_ext.c:4945 reads off char_texture. The real struct has ~20 more. */
struct gltexture_s
{
	VkDescriptorSet descriptor_set;
};

/* FAITHFUL: glquake.h:625-630 */
typedef struct
{
	float position[3];
	float texcoord[2];
	byte  color[4];
} basicvertex_t;

/* FAITHFUL: glquake.h:790 and the two renderer entry points DrawQC_
 * CharacterQuad calls; vkCmdSetScissor is Vulkan's own. */
byte *R_VertexAllocate (int size, VkBuffer *buffer, VkDeviceSize *buffer_offset);
void  R_BindPipeline (cb_context_t *cbx, VkPipelineBindPoint bind_point, vulkan_pipeline_t pipeline);
void  vkCmdSetScissor (VkCommandBuffer cb, uint32_t first, uint32_t count, const VkRect2D *scissors);
int	  R_LightPoint (vec3_t p, float ofs, lightcache_t *cache, vec3_t *lightcolor);

/* FAITHFUL: Quake/draw.h:32-42's picflags_t and the Draw_* entry points
 * pr_ext.c's PF_cl_drawpic / PF_cl_drawsubpic / PF_cl_getimagesize reach
 * (draw.h itself is not includable here: every prototype takes cb_context_t).
 * gl_texmgr.c:63's palette table comes along for PF_cl_drawcharacter's colour
 * lookup. */
typedef enum
{
	PICFLAG_AUTO = 0,
	PICFLAG_WAD = (1u << 0),
	PICFLAG_WRAP = (1u << 2),
	PICFLAG_MIPMAP = (1u << 3),
	PICFLAG_NOLOAD = (1u << 31)
} picflags_t;
qpic_t *Draw_PicFromWad2 (const char *name, unsigned int texflags, int picflags);
qpic_t *Draw_GetCachedPic (const char *path);
qpic_t *Draw_TryCachePic (const char *path, unsigned int texflags, int picflags);
void	Draw_Pic (cb_context_t *cbx, float x, float y, qpic_t *pic, float alpha, qboolean alpha_blend);
void	Draw_SubPic (
	   cb_context_t *cbx, float x, float y, float w, float h, qpic_t *pic, float s1, float t1, float s2, float t2, float *rgb, float alpha);
extern unsigned int d_8to24table[256];

extern int r_trace_line_cache_counter;
#define InvalidateTraceLineCache()    \
	do                                \
	{                                 \
		++r_trace_line_cache_counter; \
	} while (0);

extern qboolean render_warp;
extern int		render_scale;

void		Fog_ParseServerMessage (void);
const char *Fog_GetFogCommand (qboolean always);
void		Fog_NewMap (void);
void		Sky_NewMap (void);
void		Sky_LoadSkyBox (const char *name);
const char *Sky_GetSkyCommand (qboolean always);

void R_UpdateEntityDlights (void);
void R_ClearParticles (void);
void R_TranslatePlayerSkin (int playernum);
void R_TranslateNewPlayerSkin (int playernum);
void R_AllocateEntityBLAS (entity_t *e);
void R_FreeEntityBLAS (entity_t *e);

/* quakedef.h's client-visible slice: :136-167 items_t (view.c's cshift
 * powerup arms), :214 MAX_SCOREBOARD, :392, :408, :484, :496. */
/* clang-format off */
typedef enum
{
	IT_SHOTGUN			= 1,
	IT_SUPER_SHOTGUN	= 2,
	IT_NAILGUN			= 4,
	IT_SUPER_NAILGUN	= 8,
	IT_GRENADE_LAUNCHER	= 16,
	IT_ROCKET_LAUNCHER	= 32,
	IT_LIGHTNING		= 64,
	IT_SUPER_LIGHTNING	= 128,
	IT_SHELLS			= 256,
	IT_NAILS			= 512,
	IT_ROCKETS			= 1024,
	IT_CELLS			= 2048,
	IT_AXE				= 4096,
	IT_ARMOR1			= 8192,
	IT_ARMOR2			= 16384,
	IT_ARMOR3			= 32768,
	IT_SUPERHEALTH		= 65536,
	IT_KEY1				= 131072,
	IT_KEY2				= 262144,
	IT_INVISIBILITY		= 524288,
	IT_INVULNERABILITY	= 1048576,
	IT_SUIT				= 2097152,
	IT_QUAD				= 4194304,
	IT_SIGIL1			= (1<<28),
	IT_SIGIL2			= (1<<29),
	IT_SIGIL3			= (1<<30),
	IT_SIGIL4			= (1<<31),
} items_t;
/* clang-format on */

#define MAX_SCOREBOARD 16

extern qboolean noclip_anglehack;
extern int		host_framecount;
void			Host_ShutdownServer (qboolean crash);
void			DemoList_Rebuild (void);

/* cl_main.c:1168 is the one SDL call in the client stratum (the `copy` of a
 * console link). SDL3's own header is off-limits here, so the prototype is
 * spelled out and stubs.c owns the symbol. */
int SDL_SetClipboardText (const char *text);
/* common.h:169 declares MSG_WriteStaticOrBaseLine, but this header includes
 * common.h (line 845) well before the M6 rename block, so cl_demo.c:395,407
 * would see only the plain declaration and call an undeclared c_ref_ name.
 * Redeclared here, where the rename macro is in effect. */
void MSG_WriteStaticOrBaseLine (
	sizebuf_t *buf, int idx, struct entity_state_s *state, unsigned int protocol_pext2, unsigned int protocol, unsigned int protocolflags);

/* The engine's own client-side headers. None of these seven has a single
 * #include line of its own except harness.h (q_types.h), and every type they
 * name is in scope by now, so they come in whole rather than transcribed --
 * the T6.0 rule. view.h must sit after the rename block above: it declares
 * view.c's own entry points and v_blend/vid_gamma/vid_contrast, which are
 * oracle symbols now. */
/* ---- Phase 7 M7 renames --------------------------------------------------
 * chase.c, cl_demo.c, cl_input.c, cl_main.c, cl_parse.c, cl_tent.c and view.c
 * are oracle sources from Phase 7 M7 (T7.0). Every non-static symbol they
 * define is renamed here so the C reference and the Rust port can coexist in
 * one link, the same way the M6 block above does for the sv_* stratum.
 *
 * The list was taken from `llvm-nm --defined-only` on the seven objects, not
 * from reading the sources, so it cannot drift silently;
 * scripts/harness/check_ctest_symbols.sh is the gate that keeps it complete.
 *
 * This block must sit before the client-stratum headers included below
 * (client.h, view.h, ...): those declare the very symbols being renamed, and a
 * declaration that escapes the macro would leave an un-renamed prototype for
 * an object that no longer exists under that name.
 */

/* chase.c */
#define chase_active           c_ref_chase_active
#define chase_back             c_ref_chase_back
#define Chase_Init             c_ref_Chase_Init
#define chase_right            c_ref_chase_right
#define chase_up               c_ref_chase_up
#define Chase_UpdateForClient  c_ref_Chase_UpdateForClient
#define Chase_UpdateForDrawing c_ref_Chase_UpdateForDrawing
#define TraceLine              c_ref_TraceLine

/* cl_demo.c */
#define CL_GetMessage    c_ref_CL_GetMessage
#define CL_PlayDemo_f    c_ref_CL_PlayDemo_f
#define CL_Record_f      c_ref_CL_Record_f
#define CL_Resume_Record c_ref_CL_Resume_Record
#define CL_Seek_f        c_ref_CL_Seek_f
#define CL_Stop_f        c_ref_CL_Stop_f
#define CL_StopPlayback  c_ref_CL_StopPlayback
#define CL_TimeDemo_f    c_ref_CL_TimeDemo_f

/* cl_input.c */
#define CL_AdjustAngles  c_ref_CL_AdjustAngles
#define cl_alwaysrun     c_ref_cl_alwaysrun
#define CL_AngleLocked   c_ref_CL_AngleLocked
#define cl_anglespeedkey c_ref_cl_anglespeedkey
#define cl_backspeed     c_ref_cl_backspeed
#define CL_BaseMove      c_ref_CL_BaseMove
#define CL_FinishMove    c_ref_CL_FinishMove
#define cl_forwardspeed  c_ref_cl_forwardspeed
#define CL_InitInput     c_ref_CL_InitInput
#define CL_KeyState      c_ref_CL_KeyState
#define cl_movespeedkey  c_ref_cl_movespeedkey
#define cl_pitchspeed    c_ref_cl_pitchspeed
#define CL_SendMove      c_ref_CL_SendMove
#define cl_sidespeed     c_ref_cl_sidespeed
#define cl_upspeed       c_ref_cl_upspeed
#define cl_yawspeed      c_ref_cl_yawspeed
#define in_attack        c_ref_in_attack
#define IN_AttackDown    c_ref_IN_AttackDown
#define IN_AttackUp      c_ref_IN_AttackUp
#define in_back          c_ref_in_back
#define IN_BackDown      c_ref_IN_BackDown
#define IN_BackUp        c_ref_IN_BackUp
#define in_down          c_ref_in_down
#define IN_DownDown      c_ref_IN_DownDown
#define IN_DownUp        c_ref_IN_DownUp
#define in_forward       c_ref_in_forward
#define IN_ForwardDown   c_ref_IN_ForwardDown
#define IN_ForwardUp     c_ref_IN_ForwardUp
#define IN_Impulse       c_ref_IN_Impulse
#define in_impulse       c_ref_in_impulse
#define in_jump          c_ref_in_jump
#define IN_JumpDown      c_ref_IN_JumpDown
#define IN_JumpUp        c_ref_IN_JumpUp
#define in_klook         c_ref_in_klook
#define IN_KLookDown     c_ref_IN_KLookDown
#define IN_KLookUp       c_ref_IN_KLookUp
#define in_left          c_ref_in_left
#define IN_LeftDown      c_ref_IN_LeftDown
#define IN_LeftUp        c_ref_IN_LeftUp
#define in_lookdown      c_ref_in_lookdown
#define IN_LookdownDown  c_ref_IN_LookdownDown
#define IN_LookdownUp    c_ref_IN_LookdownUp
#define in_lookup        c_ref_in_lookup
#define IN_LookupDown    c_ref_IN_LookupDown
#define IN_LookupUp      c_ref_IN_LookupUp
#define in_mlook         c_ref_in_mlook
#define IN_MLookDown     c_ref_IN_MLookDown
#define IN_MLookUp       c_ref_IN_MLookUp
#define in_moveleft      c_ref_in_moveleft
#define IN_MoveleftDown  c_ref_IN_MoveleftDown
#define IN_MoveleftUp    c_ref_IN_MoveleftUp
#define in_moveright     c_ref_in_moveright
#define IN_MoverightDown c_ref_IN_MoverightDown
#define IN_MoverightUp   c_ref_IN_MoverightUp
#define in_right         c_ref_in_right
#define IN_RightDown     c_ref_IN_RightDown
#define IN_RightUp       c_ref_IN_RightUp
#define in_speed         c_ref_in_speed
#define IN_SpeedDown     c_ref_IN_SpeedDown
#define IN_SpeedUp       c_ref_IN_SpeedUp
#define in_strafe        c_ref_in_strafe
#define IN_StrafeDown    c_ref_IN_StrafeDown
#define IN_StrafeUp      c_ref_IN_StrafeUp
#define in_up            c_ref_in_up
#define IN_UpDown        c_ref_IN_UpDown
#define IN_UpUp          c_ref_IN_UpUp
#define in_use           c_ref_in_use
#define IN_UseDown       c_ref_IN_UseDown
#define IN_UseUp         c_ref_IN_UseUp
#define KeyDown          c_ref_KeyDown
#define KeyUp            c_ref_KeyUp

/* cl_main.c -- `cl` and `cls` included; see the DUPLICATE-SYMBOL
 * HAZARD note in stubs.c for the storage split they create. */
#define cfg_unbindall                     c_ref_cfg_unbindall
#define cl                                c_ref_cl
#define CL_AccumulateCmd                  c_ref_CL_AccumulateCmd
#define CL_AllocDlight                    c_ref_CL_AllocDlight
#define cl_bottomcolor                    c_ref_cl_bottomcolor
#define CL_ClearState                     c_ref_CL_ClearState
#define CL_ClearTrailStates               c_ref_CL_ClearTrailStates
#define cl_confirmquit                    c_ref_cl_confirmquit
#define CL_DecayLights                    c_ref_CL_DecayLights
#define CL_Disconnect                     c_ref_CL_Disconnect
#define CL_Disconnect_f                   c_ref_CL_Disconnect_f
#define cl_dlights                        c_ref_cl_dlights
#define CL_EstablishConnection            c_ref_CL_EstablishConnection
#define CL_FreeState                      c_ref_CL_FreeState
#define CL_GenerateRandomParticlePrecache c_ref_CL_GenerateRandomParticlePrecache
#define CL_Init                           c_ref_CL_Init
#define CL_LerpPoint                      c_ref_CL_LerpPoint
#define cl_lightstyle                     c_ref_cl_lightstyle
#define cl_maxpitch                       c_ref_cl_maxpitch
#define cl_maxvisedicts                   c_ref_cl_maxvisedicts
#define cl_minpitch                       c_ref_cl_minpitch
#define cl_name                           c_ref_cl_name
#define CL_NextDemo                       c_ref_CL_NextDemo
#define cl_nolerp                         c_ref_cl_nolerp
#define cl_numvisedicts                   c_ref_cl_numvisedicts
#define cl_numvisedicts_alpha_overwater   c_ref_cl_numvisedicts_alpha_overwater
#define cl_numvisedicts_alpha_underwater  c_ref_cl_numvisedicts_alpha_underwater
#define CL_PrintEntities_f                c_ref_CL_PrintEntities_f
#define CL_ReadFromServer                 c_ref_CL_ReadFromServer
#define CL_RelinkEntities                 c_ref_CL_RelinkEntities
#define CL_SendCmd                        c_ref_CL_SendCmd
#define CL_SendInitialUserinfo            c_ref_CL_SendInitialUserinfo
#define cl_shownet                        c_ref_cl_shownet
#define CL_SignonReply                    c_ref_CL_SignonReply
#define cl_startdemos                     c_ref_cl_startdemos
#define cl_topcolor                       c_ref_cl_topcolor
#define CL_Tracepos_f                     c_ref_CL_Tracepos_f
#define CL_Viewpos_f                      c_ref_CL_Viewpos_f
#define cl_visedicts                      c_ref_cl_visedicts
#define cl_visedicts_alpha                c_ref_cl_visedicts_alpha
#define cls                               c_ref_cls
#define lookspring                        c_ref_lookspring
#define lookstrafe                        c_ref_lookstrafe
#define m_forward                         c_ref_m_forward
#define m_pitch                           c_ref_m_pitch
#define m_side                            c_ref_m_side
#define m_yaw                             c_ref_m_yaw
#define needs_relink                      c_ref_needs_relink
#define sensitivity                       c_ref_sensitivity
#define SV_UpdateInfo                     c_ref_SV_UpdateInfo

/* cl_parse.c */
#define CL_EntityNum          c_ref_CL_EntityNum
#define CL_NewTranslation     c_ref_CL_NewTranslation
#define CL_ParseLocalSound    c_ref_CL_ParseLocalSound
#define CL_ParseServerMessage c_ref_CL_ParseServerMessage
#define CL_RegisterParticles  c_ref_CL_RegisterParticles
#define svc_strings           c_ref_svc_strings

/* cl_tent.c */
#define cl_beams          c_ref_cl_beams
#define CL_InitTEnts      c_ref_CL_InitTEnts
#define CL_NewTempEntity  c_ref_CL_NewTempEntity
#define CL_ParseTEnt      c_ref_CL_ParseTEnt
#define cl_temp_entities  c_ref_cl_temp_entities
#define CL_UpdateBeam     c_ref_CL_UpdateBeam
#define CL_UpdateTEnts    c_ref_CL_UpdateTEnts
#define num_temp_entities c_ref_num_temp_entities

/* view.c */
#define angledelta                c_ref_angledelta
#define CalcGunAngle              c_ref_CalcGunAngle
#define cl_bob                    c_ref_cl_bob
#define cl_bobcycle               c_ref_cl_bobcycle
#define cl_bobup                  c_ref_cl_bobup
#define cl_rollangle              c_ref_cl_rollangle
#define cl_rollspeed              c_ref_cl_rollspeed
#define crosshair                 c_ref_crosshair
#define crosshair_def             c_ref_crosshair_def
#define cshift_lava               c_ref_cshift_lava
#define cshift_slime              c_ref_cshift_slime
#define cshift_water              c_ref_cshift_water
#define gl_cshiftpercent          c_ref_gl_cshiftpercent
#define gl_cshiftpercent_bonus    c_ref_gl_cshiftpercent_bonus
#define gl_cshiftpercent_contents c_ref_gl_cshiftpercent_contents
#define gl_cshiftpercent_damage   c_ref_gl_cshiftpercent_damage
#define gl_cshiftpercent_powerup  c_ref_gl_cshiftpercent_powerup
#define r_viewmodel_quake         c_ref_r_viewmodel_quake
#define scr_ofsx                  c_ref_scr_ofsx
#define scr_ofsy                  c_ref_scr_ofsy
#define scr_ofsz                  c_ref_scr_ofsz
#define V_AddIdle                 c_ref_V_AddIdle
#define v_autopitch               c_ref_v_autopitch
#define v_blend                   c_ref_v_blend
#define V_BonusFlash_f            c_ref_V_BonusFlash_f
#define V_BoundOffsets            c_ref_V_BoundOffsets
#define V_CalcBlend               c_ref_V_CalcBlend
#define V_CalcBob                 c_ref_V_CalcBob
#define V_CalcIntermissionRefdef  c_ref_V_CalcIntermissionRefdef
#define V_CalcPowerupCshift       c_ref_V_CalcPowerupCshift
#define V_CalcRefdef              c_ref_V_CalcRefdef
#define V_CalcRoll                c_ref_V_CalcRoll
#define V_CalcViewRoll            c_ref_V_CalcViewRoll
#define v_centermove              c_ref_v_centermove
#define v_centerspeed             c_ref_v_centerspeed
#define V_cshift_f                c_ref_V_cshift_f
#define V_DriftPitch              c_ref_V_DriftPitch
#define v_gunkick                 c_ref_v_gunkick
#define v_idlescale               c_ref_v_idlescale
#define V_Init                    c_ref_V_Init
#define v_ipitch_cycle            c_ref_v_ipitch_cycle
#define v_ipitch_level            c_ref_v_ipitch_level
#define v_iroll_cycle             c_ref_v_iroll_cycle
#define v_iroll_level             c_ref_v_iroll_level
#define v_iyaw_cycle              c_ref_v_iyaw_cycle
#define v_iyaw_level              c_ref_v_iyaw_level
#define v_kickpitch               c_ref_v_kickpitch
#define v_kickroll                c_ref_v_kickroll
#define v_kicktime                c_ref_v_kicktime
#define V_ParseDamage             c_ref_V_ParseDamage
#define v_punchangles             c_ref_v_punchangles
#define v_punchangles_times       c_ref_v_punchangles_times
#define V_RenderView              c_ref_V_RenderView
#define V_ResetBlend              c_ref_V_ResetBlend
#define V_RestoreAngles           c_ref_V_RestoreAngles
#define V_SetContentsColor        c_ref_V_SetContentsColor
#define V_SetupFrame              c_ref_V_SetupFrame
#define V_StartPitchDrift         c_ref_V_StartPitchDrift
#define V_StopPitchDrift          c_ref_V_StopPitchDrift
/* ---- end Phase 7 M7 renames ---- */

#include "console.h"
#include "screen.h"
#include "keys.h"
#include "input.h"
#include "cdaudio.h"
#include "harness.h"
#include "view.h"
/* ---- end Phase 7 M7 seam ---- */

/* ---- Phase 7 M8 seam: the quakedef.h / glquake.h declarations host.c and
 * host_cmd.c reach that no header in this slice supplies (task T8.1). Both
 * files become oracle TUs here, composed by stubs/host_ref.c and
 * stubs/host_cmd_ref.c rather than listed in build.rs's C_SOURCES; the reason
 * (the ADR-009 trap machinery stubs.c owns under the plain Host_Error /
 * Host_EndGame / Host_Guard / Host_Reraise names) is written out at the top of
 * stubs/host_ref.c. Everything below is copied verbatim from the real header
 * named on each line. ---- */

#define HOST_NETITERVAL_FREQ (71.9990) /* quakedef.h:70 */

/* quakedef.h:475-477. Host_Guard's real result set. stubs.c's substitute
 * guard returns its own CTEST_GUARD_* values and says so at its definition;
 * these three are what the REAL Host_Guard (host.c:302), now an oracle
 * function, returns. */
#define HOST_GUARD_OK			0
#define HOST_GUARD_ABORTSERVER	1
#define HOST_GUARD_SCREEN_ERROR 2

extern qboolean in_update_screen; /* glquake.h:543 */

/* quakedef.h:412-461 verbatim: the file-list stratum host_cmd.c defines and
 * menu.c consumes. */
typedef struct filelist_item_s
{
	char					name[32];
	struct filelist_item_s *next;
} filelist_item_t;

extern filelist_item_t *modlist;
extern filelist_item_t *extralevels;
extern filelist_item_t *demolist;
extern filelist_item_t *savelist;

typedef enum
{
	MAPTYPE_CUSTOM_MOD_START,
	MAPTYPE_CUSTOM_MOD_LEVEL,
	MAPTYPE_CUSTOM_MOD_END,
	MAPTYPE_CUSTOM_MOD_DM,

	MAPTYPE_MOD_START,
	MAPTYPE_MOD_LEVEL,
	MAPTYPE_MOD_END,
	MAPTYPE_MOD_DM,

	MAPTYPE_CUSTOM_ID_START,
	MAPTYPE_CUSTOM_ID_LEVEL,
	MAPTYPE_CUSTOM_ID_END,
	MAPTYPE_CUSTOM_ID_DM,

	MAPTYPE_ID_START,
	MAPTYPE_ID_EP1_LEVEL,
	MAPTYPE_ID_EP2_LEVEL,
	MAPTYPE_ID_EP3_LEVEL,
	MAPTYPE_ID_EP4_LEVEL,
	MAPTYPE_ID_END,
	MAPTYPE_ID_DM,
	MAPTYPE_ID_LEVEL,

	MAPTYPE_BMODEL,

	MAPTYPE_COUNT,
} maptype_t;

maptype_t	ExtraMaps_GetType (const filelist_item_t *item);
qboolean	ExtraMaps_IsStart (maptype_t type);
const char *ExtraMaps_GetMessage (const filelist_item_t *item);

extern filelist_item_t **extralevels_sorted;

const char *Modlist_GetFullName (const filelist_item_t *item);

#define SAVEGAME_COMMENT_LENGTH 39 /* quakedef.h:97 */

/* quakedef.h:173-207 verbatim: the mission-pack item bit sets Host_Give_f
 * (host_cmd.c:2663) switches on. */
typedef enum
{
	RIT_SHELLS = 128,
	RIT_NAILS = 256,
	RIT_ROCKETS = 512,
	RIT_CELLS = 1024,
	RIT_AXE = 2048,
	RIT_LAVA_NAILGUN = 4096,
	RIT_LAVA_SUPER_NAILGUN = 8192,
	RIT_MULTI_GRENADE = 16384,
	RIT_MULTI_ROCKET = 32768,
	RIT_PLASMA_GUN = 65536,
	RIT_ARMOR1 = 8388608,
	RIT_ARMOR2 = 16777216,
	RIT_ARMOR3 = 33554432,
	RIT_LAVA_NAILS = 67108864,
	RIT_PLASMA_AMMO = 134217728,
	RIT_MULTI_ROCKETS = 268435456,
	RIT_SHIELD = 536870912,
	RIT_ANTIGRAV = 1073741824,
	RIT_SUPERHEALTH = 2147483648,
} rogueitems_t;

typedef enum
{
	HIT_PROXIMITY_GUN_BIT = 16,
	HIT_MJOLNIR_BIT = 7,
	HIT_LASER_CANNON_BIT = 23,
	HIT_PROXIMITY_GUN = (1 << HIT_PROXIMITY_GUN_BIT),
	HIT_MJOLNIR = (1 << HIT_MJOLNIR_BIT),
	HIT_LASER_CANNON = (1 << HIT_LASER_CANNON_BIT),
	HIT_WETSUIT = (1 << (23 + 2)),
	HIT_EMPATHY_SHIELDS = (1 << (23 + 3)),
} hipnoticitems_t;

/* quakedef.h:482-497 verbatim: host_cmd.c owns all of these; host.c calls
 * four of them, so without the declarations host.c's TU falls back to
 * implicit int. */
void Host_Quit_f (void);
void Host_Resetdemos (void);

void ExtraMaps_Init (void);
void Modlist_Init (void);
void DemoList_Init (void);
void SaveList_Init (void);

void ExtraMaps_NewGame (void);
void ExtraMaps_Clear (void);
void ExtraMaps_ShutDown (void);
void DemoList_Rebuild (void);
void SaveList_Rebuild (void);

#include "server.h"
#include "client.h"

/* ---- Phase 7 M10b seam: the menu.h / platform.h declarations keys.c reaches
 * (task M10b). keys.c becomes an oracle TU here, composed by stubs/keys_ref.c
 * rather than listed in build.rs's C_SOURCES; the reason (host.c's link
 * doubles for Key_Init / Key_UpdateForDest / History_Shutdown /
 * Key_WriteBindings, and the two host_differential.rs tests that assert on
 * them) is written out at the top of stubs/keys_ref.c.
 *
 * menu.h is NOT included as-is: it declares M_Print/M_Draw over cb_context_t,
 * which is Vulkan-typed (gl_context.h). platform.h is include-clean but is
 * otherwise entirely SDL-facing, so only the one entry point keys.c calls is
 * taken. Both groups are copied verbatim from the line named on each. ---- */

extern qboolean m_is_quitting; /* menu.h:58 */

void	 M_Keydown (int key);			 /* menu.h:65 */
void	 M_Charinput (int key);			 /* menu.h:66 */
qboolean M_TextEntry (void);			 /* menu.h:67 */
qboolean M_WaitingForKeyBinding (void);	 /* menu.h:68 */
void	 M_ToggleMenu_f (void);			 /* menu.h:69 */

char *PL_GetClipboardData (void); /* platform.h:34 */

#endif /* C_REF_PRELUDE_H */
