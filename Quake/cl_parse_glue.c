/*
Copyright (C) 2026 vkqr-engine contributors

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.

See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program; if not, write to the Free Software
Foundation, Inc., 59 Temple Place - Suite 330, Boston, MA  02111-1307, USA.
*/
// cl_parse_glue.c -- the C frame around the Rust server-message parser port.
//
// Compiled instead of cl_parse.c under -Duse_rust_host (Rust migration
// Phase 7 M7, T7.3). Four jobs, the view_glue.c / cl_tent_glue.c shape:
//
//  1. Own the one C-visible object cl_parse.c defined: svc_strings[128]
//     (cl_parse.c:30-88). No other translation unit reads it today, but it is
//     external linkage in the original and the Illegible-server-message raise
//     formats svc_strings[lastcmd], so the storage stays in C and the Rust
//     core reaches it through an extern.
//  2. Guard every callee cl_parse.c reaches that can Host_Error /
//     Host_EndGame (ADR-009 rule 3). Each ClParse_Glue_* below is a
//     Host_Guard trampoline returning 0/1/2.
//  3. Turn the status the Rust core returns back into the *exact* original
//     raise, from a pure C frame. cl_parse.c has 34 live raise sites (31
//     Host_Error + 3 Host_EndGame; the three Host_Errors at :886/:890/:894
//     are inside the #if 0 CL_KeepaliveMessage, and the UF_UNUSED2
//     Host_EndGame at :439 is dead because protocol.h:33 defines
//     LERP_BANDAID). ClParse_Raise is the PRBI_Raise pattern from
//     pr_cmds_glue.c:309: one switch, one arm per site, plus a
//     CLPARSE_RAISE_GUARD arm that re-issues a caught guard jump.
//     Host_Reraise is called only from here.
//  4. Forward the remaining cl_parse.c entry points to their Rust cores.
//     Both Sys_Error sites (cl_parse.c:1530, :1928) abort rather than
//     jumping, so Rust calls Sys_Error directly -- the world.c / sv_phys.c /
//     sv_send.c precedent.

#include "quakedef.h"
#include "bgmusic.h"
#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

/* ---------------------------------------------------------------------------
 * C-visible objects (cl_parse.c:30-88).
 */

const char *svc_strings[128] = {
	"svc_bad", "svc_nop", "svc_disconnect", "svc_updatestat",
	"svc_version",	 // [long] server version
	"svc_setview",	 // [short] entity number
	"svc_sound",	 // <see code>
	"svc_time",		 // [float] server time
	"svc_print",	 // [string] null terminated string
	"svc_stufftext", // [string] stuffed into client's console buffer
					 // the string should be \n terminated
	"svc_setangle",	 // [vec3] set the view angle to this absolute value

	"svc_serverinfo",	// [long] version
						// [string] signon string
						// [string]..[0]model cache [string]...[0]sounds cache
						// [string]..[0]item cache
	"svc_lightstyle",	// [byte] [string]
	"svc_updatename",	// [byte] [string]
	"svc_updatefrags",	// [byte] [short]
	"svc_clientdata",	// <shortbits + data>
	"svc_stopsound",	// <see code>
	"svc_updatecolors", // [byte] [byte]
	"svc_particle",		// [vec3] <variable>
	"svc_damage",		// [byte] impact [byte] blood [vec3] from

	"svc_spawnstatic",
	/*"OBSOLETE svc_spawnbinary"*/ "21 svc_spawnstatic_fte", "svc_spawnbaseline",

	"svc_temp_entity", // <variable>
	"svc_setpause", "svc_signonnum", "svc_centerprint", "svc_killedmonster", "svc_foundsecret", "svc_spawnstaticsound", "svc_intermission",
	"svc_finale",  // [string] music [string] text
	"svc_cdtrack", // [byte] track [byte] looptrack
	"svc_sellscreen", "svc_cutscene",
	// johnfitz -- new server messages
	"svc_showpic_dp",			  // 35
	"svc_hidepic_dp",			  // 36
	"svc_skybox_fitz",			  // 37					// [string] skyname
	"38",						  // 38
	"39",						  // 39
	"svc_bf_fitz",				  // 40						// no data
	"svc_fog_fitz",				  // 41					// [byte] density [byte] red [byte] green [byte] blue [float] time
	"svc_spawnbaseline2_fitz",	  // 42			// support for large modelindex, large framenum, alpha, using flags
	"svc_spawnstatic2_fitz",	  // 43			// support for large modelindex, large framenum, alpha, using flags
	"svc_spawnstaticsound2_fitz", //	44		// [coord3] [short] samp [byte] vol [byte] aten
								  // johnfitz

	// 2021 RE-RELEASE:
	"svc_setviews",		  // 45
	"svc_updateping",	  // 46
	"svc_updatesocial",	  // 47
	"svc_updateplinfo",	  // 48
	"svc_rawprint",		  // 49
	"svc_servervars",	  // 50
	"svc_seq",			  // 51
	"svc_achievement",	  // 52
	"svc_chat",			  // 53
	"svc_levelcompleted", // 54
	"svc_backtolobby",	  // 55
	"svc_localsound"	  // 56
};

/* ---------------------------------------------------------------------------
 * ADR-009 status codes. Mirrored verbatim by rust/quake-capi/src/cl_parse.rs
 * and rust/quake-ctest/stubs/cl_parse_ref.c; the three copies must stay in
 * step.
 */

#define CLPARSE_OK					 0
#define CLPARSE_RAISE_GUARD			 1	/* a == Host_Guard status */
#define CLPARSE_ERR_ENTITYNUM		 2	/* :109, :115  a == num */
#define CLPARSE_ERR_SOUNDNUM		 3	/* :823  a == sound_num */
#define CLPARSE_ERR_SOUNDENT		 4	/* :827  a == ent */
#define CLPARSE_ERR_LOCALSOUND		 5	/* :847  a == sound_num */
#define CLPARSE_ERR_PEXT1			 6	/* :962  a == unsupported bits */
#define CLPARSE_ERR_PEXT2			 7	/* :969  a == unsupported bits */
#define CLPARSE_ERR_VERSION			 8	/* :979, :1868  a == i */
#define CLPARSE_ERR_MAXCLIENTS		 9	/* :1013  a == cl.maxclients */
#define CLPARSE_ERR_TOOMANYMODELS	 10 /* :1050 */
#define CLPARSE_ERR_TOOMANYSOUNDS	 11 /* :1072 */
#define CLPARSE_ERR_MODELNOTFOUND	 12 /* :1095  s == model_precache[i] */
#define CLPARSE_ERR_BADMODNUM		 13 /* :1209, :1288 */
#define CLPARSE_ERR_TOOMANYSTATICS	 14 /* :1551 */
#define CLPARSE_ERR_BADMESSAGE		 15 /* :1808 */
#define CLPARSE_ERR_ILLEGIBLE		 16 /* :1846  a == cmd, s == svc_strings[lastcmd] */
#define CLPARSE_ERR_UPDATENAME		 17 /* :1960 */
#define CLPARSE_ERR_UPDATEFRAGS		 18 /* :1968 */
#define CLPARSE_ERR_UPDATECOLORS	 19 /* :1975 */
#define CLPARSE_ERR_SIGNON			 20 /* :2017  a == i, b == cls.signon */
#define CLPARSE_ERR_DPPRECACHE		 21 /* :2151 */
#define CLPARSE_ERR_UPDATESTATBYTE	 22 /* :2159 */
#define CLPARSE_ERR_UPDATESTATSTRING 23 /* :2165 */
#define CLPARSE_ERR_UPDATESTATFLOAT	 24 /* :2171 */
#define CLPARSE_ERR_SPAWNSTATIC2	 25 /* :2179 */
#define CLPARSE_ERR_SPAWNBASELINE2	 26 /* :2185 */
#define CLPARSE_ERR_UPDATEENTITIES	 27 /* :2193 */
#define CLPARSE_ERR_CGAMEPACKET		 28 /* :2199 */
#define CLPARSE_ERR_CSQC_MISSING	 29 /* :2207 */
#define CLPARSE_ERR_VOICECHAT		 30 /* :2212 */
#define CLPARSE_END_DELTAINFO		 31 /* :362  Host_EndGame */
#define CLPARSE_END_UF_UNUSED1		 32 /* :444  Host_EndGame */
#define CLPARSE_END_DISCONNECTED	 33 /* :1874 Host_EndGame */

FUNC_NORETURN static void ClParse_Raise (int status, int a, int b, const char *s)
{
	switch (status)
	{
	case CLPARSE_RAISE_GUARD:
		/* a guarded seam's Host_Error/Host_EndGame, re-issued now that the
		   Rust frames above it have returned normally (ADR-009 rule 3). */
		Host_Reraise (a);
		Sys_Error ("ClParse_Raise: Host_Reraise returned");
	case CLPARSE_ERR_ENTITYNUM:
		Host_Error ("CL_EntityNum: %i is an invalid number", a);
	case CLPARSE_ERR_SOUNDNUM:
		Host_Error ("CL_ParseStartSoundPacket: %i > MAX_SOUNDS", a);
	case CLPARSE_ERR_SOUNDENT:
		Host_Error ("CL_ParseStartSoundPacket: ent = %i", a);
	case CLPARSE_ERR_LOCALSOUND:
		Host_Error ("CL_ParseLocalSound: %i > MAX_SOUNDS", a);
	case CLPARSE_ERR_PEXT1:
		Host_Error ("Server returned FTE1 protocol extensions that are not supported (%#x)", (unsigned int)a);
	case CLPARSE_ERR_PEXT2:
		Host_Error ("Server returned FTE2 protocol extensions that are not supported (%#x)", (unsigned int)a);
	case CLPARSE_ERR_VERSION:
		Host_Error ("Server returned version %i, not %i or %i or %i", a, PROTOCOL_NETQUAKE, PROTOCOL_FITZQUAKE, PROTOCOL_RMQ);
	case CLPARSE_ERR_MAXCLIENTS:
		Host_Error ("Bad maxclients (%u) from server", (unsigned int)a);
	case CLPARSE_ERR_TOOMANYMODELS:
		Host_Error ("Server sent too many model precaches");
	case CLPARSE_ERR_TOOMANYSOUNDS:
		Host_Error ("Server sent too many sound precaches");
	case CLPARSE_ERR_MODELNOTFOUND:
		Host_Error ("Model %s not found", s);
	case CLPARSE_ERR_BADMODNUM:
		Host_Error ("CL_ParseModel: bad modnum");
	case CLPARSE_ERR_TOOMANYSTATICS:
		Host_Error ("Too many static entities");
	case CLPARSE_ERR_BADMESSAGE:
		Host_Error ("CL_ParseServerMessage: Bad server message");
	case CLPARSE_ERR_ILLEGIBLE:
		Host_Error ("Illegible server message %d, previous was %s", a, s);
	case CLPARSE_ERR_UPDATENAME:
		Host_Error ("CL_ParseServerMessage: svc_updatename > MAX_SCOREBOARD");
	case CLPARSE_ERR_UPDATEFRAGS:
		Host_Error ("CL_ParseServerMessage: svc_updatefrags > MAX_SCOREBOARD");
	case CLPARSE_ERR_UPDATECOLORS:
		Host_Error ("CL_ParseServerMessage: svc_updatecolors > MAX_SCOREBOARD");
	case CLPARSE_ERR_SIGNON:
		Host_Error ("Received signon %i when at %i", a, b);
	case CLPARSE_ERR_DPPRECACHE:
		Host_Error ("Received svcdp_precache but extension not active");
	case CLPARSE_ERR_UPDATESTATBYTE:
		Host_Error ("Received svcdp_updatestatbyte but extension not active");
	case CLPARSE_ERR_UPDATESTATSTRING:
		Host_Error ("Received svcfte_updatestatstring but extension not active");
	case CLPARSE_ERR_UPDATESTATFLOAT:
		Host_Error ("Received svcfte_updatestatfloat but extension not active");
	case CLPARSE_ERR_SPAWNSTATIC2:
		Host_Error ("Received svcfte_spawnstatic2 but extension not active");
	case CLPARSE_ERR_SPAWNBASELINE2:
		Host_Error ("Received svcfte_spawnbaseline2 but extension not active");
	case CLPARSE_ERR_UPDATEENTITIES:
		Host_Error ("Received svcfte_updateentities but extension not active");
	case CLPARSE_ERR_CGAMEPACKET:
		Host_Error ("Received svcfte_cgamepacket but extension not active");
	case CLPARSE_ERR_CSQC_MISSING:
		Host_Error ("CSQC_Parse_Event: Missing or incompatible CSQC\n");
	case CLPARSE_ERR_VOICECHAT:
		Host_Error ("Received svcfte_voicechat but extension not active");
	case CLPARSE_END_DELTAINFO:
		Host_EndGame ("unsupported entity delta info\n");
	case CLPARSE_END_UF_UNUSED1:
		Host_EndGame ("UF_UNUSED1 bit\n");
	case CLPARSE_END_DISCONNECTED:
		Host_EndGame ("Server disconnected\n");
	default:
		Sys_Error ("ClParse_Raise: unknown status %i", status);
	}
}

/* ---------------------------------------------------------------------------
 * Guarded callbacks (ADR-009 rule 3). Over-guarding costs one setjmp pair and
 * returns CLPARSE_OK, so anything whose reachability could not be closed by
 * inspection is guarded; under-guarding is a correctness bug.
 */

/* cl_parse.c:757, :1159, :2026 -- CL_SignonReply reaches Cbuf_AddText, the
   MSG_Write* family on cls.message, and CL_ClearState. */
static void ClParse_InvokeSignonReply (void *p)
{
	(void)p;
	CL_SignonReply ();
}

int ClParse_Glue_SignonReply (void)
{
	return Host_Guard (ClParse_InvokeSignonReply, NULL);
}

/* cl_parse.c:945 -- CL_ClearState reaches Mem_Free and the qcvm teardown. */
static void ClParse_InvokeClearState (void *p)
{
	(void)p;
	CL_ClearState ();
}

int ClParse_Glue_ClearState (void)
{
	return Host_Guard (ClParse_InvokeClearState, NULL);
}

/* cl_parse.c:951 -- Key_ClearStates reaches Key_Event, hence Cbuf_AddText. */
static void ClParse_InvokeKeyClearStates (void *p)
{
	(void)p;
	Key_ClearStates ();
}

int ClParse_Glue_KeyClearStates (void)
{
	return Host_Guard (ClParse_InvokeKeyClearStates, NULL);
}

/* cl_parse.c:941 -- SCR_BeginLoadingPlaque reaches SCR_UpdateScreen. */
static void ClParse_InvokeBeginLoadingPlaque (void *p)
{
	(void)p;
	SCR_BeginLoadingPlaque ();
}

int ClParse_Glue_BeginLoadingPlaque (void)
{
	return Host_Guard (ClParse_InvokeBeginLoadingPlaque, NULL);
}

/* cl_parse.c:1108 -- R_NewMap reaches the whole level-load path. */
static void ClParse_InvokeNewMap (void *p)
{
	(void)p;
	R_NewMap ();
}

int ClParse_Glue_NewMap (void)
{
	return Host_Guard (ClParse_InvokeNewMap, NULL);
}

/* cl_parse.c:2024 -- R_CheckEfrags. */
static void ClParse_InvokeCheckEfrags (void *p)
{
	(void)p;
	R_CheckEfrags ();
}

int ClParse_Glue_CheckEfrags (void)
{
	return Host_Guard (ClParse_InvokeCheckEfrags, NULL);
}

/* cl_parse.c:1993 -- CL_ParseTEnt is itself a Host_Reraise wrapper under
   -Duse_rust_host (cl_tent_glue.c), so a Rust frame must never call it
   directly. */
static void ClParse_InvokeParseTEnt (void *p)
{
	(void)p;
	CL_ParseTEnt ();
}

int ClParse_Glue_ParseTEnt (void)
{
	return Host_Guard (ClParse_InvokeParseTEnt, NULL);
}

/* cl_parse.c:1662-1663 -- COM_Effectinfo_Enumerate runs its callback for every
   effectinfo.txt line, and the callback here is cl_main.c:939's
   CL_GenerateRandomParticlePrecache, which reaches PScript_FindParticleType.
   The implicit "effectinfo." load immediately above it shares the frame. */
/* cl_main.c:939 -- external linkage but declared in no header, exactly like
   CL_EntityNum below; cl_parse.c:1657 re-declared it locally at the point of
   use and this frame has to do the same. */
int CL_GenerateRandomParticlePrecache (const char *pname);

static void ClParse_InvokeEffectinfoEnumerate (void *p)
{
	(void)p;
	PScript_FindParticleType ("effectinfo."); // make sure this is implicitly loaded.
	COM_Effectinfo_Enumerate (CL_GenerateRandomParticlePrecache);
}

int ClParse_Glue_EffectinfoEnumerate (void)
{
	return Host_Guard (ClParse_InvokeEffectinfoEnumerate, NULL);
}

/* cl_parse.c:2201-2203 -- the CSQC entry. ADR-008: the ambient qcvm is
   switched in and back out inside the guard, so a caught jump can never leave
   the client vm selected where a Rust frame would observe it. */
static void ClParse_InvokeCsqcParseEvent (void *p)
{
	(void)p;
	PR_SwitchQCVM (&cl.qcvm);
	PR_ExecuteProgram (cl.qcvm.extfuncs.CSQC_Parse_Event);
	PR_SwitchQCVM (NULL);
}

int ClParse_Glue_CsqcParseEvent (void)
{
	return Host_Guard (ClParse_InvokeCsqcParseEvent, NULL);
}

/* cl_parse.c:163-171 -- the sv.active debug print for an entity that arrived
   without a reset. EDICT_NUM and PR_GetString both raise, and the block runs
   against the server vm, so it stays in one C frame (ADR-008). */
static void ClParse_InvokeDebugNewEntity (void *p)
{
	unsigned int entnum = *(unsigned int *)p;
	qcvm_t		*old = qcvm;
	qcvm = NULL;
	PR_SwitchQCVM (&sv.qcvm);
	Con_DPrintf ("New entity %i(%s / %s) without reset\n", entnum, PR_GetString (EDICT_NUM (entnum)->v.classname), PR_GetString (EDICT_NUM (entnum)->v.model));
	PR_SwitchQCVM (old);
}

int ClParse_Glue_DebugNewEntity (unsigned int entnum)
{
	unsigned int n = entnum;
	return Host_Guard (ClParse_InvokeDebugNewEntity, &n);
}

/* cl_parse.c:562, :601, :1227, :1338 -- R_TranslateNewPlayerSkin reaches
   Mod_Extradata, hence Mod_LoadModel. */
static void ClParse_InvokeTranslateNewPlayerSkin (void *p)
{
	R_TranslateNewPlayerSkin (*(int *)p);
}

int ClParse_Glue_TranslateNewPlayerSkin (int playernum)
{
	int n = playernum;
	return Host_Guard (ClParse_InvokeTranslateNewPlayerSkin, &n);
}

/* cl_parse.c:1531 -- R_TranslatePlayerSkin reaches TexMgr_ReloadImage, which
   re-opens the source file and can fail inside the image loaders. */
static void ClParse_InvokeTranslatePlayerSkin (void *p)
{
	R_TranslatePlayerSkin (*(int *)p);
}

int ClParse_Glue_TranslatePlayerSkin (int playernum)
{
	int n = playernum;
	return Host_Guard (ClParse_InvokeTranslatePlayerSkin, &n);
}

/* cl_parse.c:1578 -- R_AddEfrags reaches R_SplitEntityOnNode and the efrag
   allocator. */
static void ClParse_InvokeAddEfrags (void *p)
{
	R_AddEfrags ((entity_t *)p);
}

int ClParse_Glue_AddEfrags (void *ent)
{
	return Host_Guard (ClParse_InvokeAddEfrags, ent);
}

/* cl_parse.c:1090, :1623 -- Mod_ForName reaches Mod_LoadModel. Both call
   sites pass crash = false. */
typedef struct
{
	const char *name;
	void	  **out;
} clparse_modforname_args_t;

static void ClParse_InvokeModForName (void *p)
{
	clparse_modforname_args_t *a = (clparse_modforname_args_t *)p;
	*a->out = Mod_ForName (a->name, false);
}

int ClParse_Glue_ModForName (const char *name, void **out)
{
	clparse_modforname_args_t args;
	args.name = name;
	args.out = out;
	*out = NULL;
	return Host_Guard (ClParse_InvokeModForName, &args);
}

/* cl_parse.c:1968 -- Cbuf_AddText reaches SZ_GetSpace; under -Duse_rust_cvar
   the plain name is itself a Host_Reraise wrapper. */
static void ClParse_InvokeCbufAddText (void *p)
{
	Cbuf_AddText ((const char *)p);
}

int ClParse_Glue_CbufAddText (const char *text)
{
	return Host_Guard (ClParse_InvokeCbufAddText, (void *)(uintptr_t)text);
}

/* cl_parse.c:1964, :2087, :2095 -- Cmd_ExecuteString runs arbitrary console
   commands; under -Duse_rust_cvar the plain name is a Host_Reraise wrapper. */
typedef struct
{
	const char *text;
	int			src;
	int		   *out;
} clparse_cmdexec_args_t;

static void ClParse_InvokeCmdExecuteString (void *p)
{
	clparse_cmdexec_args_t *a = (clparse_cmdexec_args_t *)p;
	*a->out = Cmd_ExecuteString (a->text, (cmd_source_t)a->src);
}

int ClParse_Glue_CmdExecuteString (const char *text, int src, int *out)
{
	clparse_cmdexec_args_t args;
	args.text = text;
	args.src = src;
	args.out = out;
	*out = 0;
	return Host_Guard (ClParse_InvokeCmdExecuteString, &args);
}

/* cl_parse.c:1636, :1662, :1682 -- PScript_FindParticleType parses particle
   scripts off disk. */
typedef struct
{
	const char *name;
	int		   *out;
} clparse_findptype_args_t;

static void ClParse_InvokeFindParticleType (void *p)
{
	clparse_findptype_args_t *a = (clparse_findptype_args_t *)p;
	*a->out = PScript_FindParticleType (a->name);
}

int ClParse_Glue_FindParticleType (const char *name, int *out)
{
	clparse_findptype_args_t args;
	args.name = name;
	args.out = out;
	*out = 0;
	return Host_Guard (ClParse_InvokeFindParticleType, &args);
}

/* cl_parse.c:1693 -- PScript_UpdateModelEffects. */
static void ClParse_InvokeUpdateModelEffects (void *p)
{
	PScript_UpdateModelEffects ((qmodel_t *)p);
}

int ClParse_Glue_UpdateModelEffects (void *mod)
{
	return Host_Guard (ClParse_InvokeUpdateModelEffects, mod);
}

/* cl_parse.c:1713 -- PScript_ParticleTrail reaches CL_EntityNum
   (r_part_fte.c:5961), which under -Duse_rust_host is the re-raising wrapper
   at the bottom of this file. */
typedef struct
{
	const float *start;
	const float *end;
	int			 type;
	float		 timeinterval;
	int			 dlkey;
	void	   **tsk;
} clparse_ptrail_args_t;

static void ClParse_InvokeParticleTrail (void *p)
{
	clparse_ptrail_args_t *a = (clparse_ptrail_args_t *)p;
	PScript_ParticleTrail (
		(float *)(uintptr_t)a->start, (float *)(uintptr_t)a->end, a->type, a->timeinterval, a->dlkey, NULL, (struct trailstate_s **)a->tsk);
}

int ClParse_Glue_ParticleTrail (const float *start, const float *end, int type, float timeinterval, int dlkey, void **tsk)
{
	clparse_ptrail_args_t args;
	args.start = start;
	args.end = end;
	args.type = type;
	args.timeinterval = timeinterval;
	args.dlkey = dlkey;
	args.tsk = tsk;
	return Host_Guard (ClParse_InvokeParticleTrail, &args);
}

/* cl_parse.c:1736 -- PScript_RunParticleEffectState, same reachability. */
typedef struct
{
	const float *org;
	const float *dir;
	float		 count;
	int			 typenum;
	void	   **tsk;
} clparse_prunstate_args_t;

static void ClParse_InvokeRunParticleEffectState (void *p)
{
	clparse_prunstate_args_t *a = (clparse_prunstate_args_t *)p;
	PScript_RunParticleEffectState ((float *)(uintptr_t)a->org, (float *)(uintptr_t)a->dir, a->count, a->typenum, (struct trailstate_s **)a->tsk);
}

int ClParse_Glue_RunParticleEffectState (const float *org, const float *dir, float count, int typenum, void **tsk)
{
	clparse_prunstate_args_t args;
	args.org = org;
	args.dir = dir;
	args.count = count;
	args.typenum = typenum;
	args.tsk = tsk;
	return Host_Guard (ClParse_InvokeRunParticleEffectState, &args);
}

/* cl_parse.c:2091 -- Sky_LoadSkyBox reaches the image loaders. */
static void ClParse_InvokeLoadSkyBox (void *p)
{
	Sky_LoadSkyBox ((const char *)p);
}

int ClParse_Glue_LoadSkyBox (const char *name)
{
	return Host_Guard (ClParse_InvokeLoadSkyBox, (void *)(uintptr_t)name);
}

/* ---------------------------------------------------------------------------
 * Re-raising public entry points (ADR-009). Each keeps the exact cl_parse.c
 * signature; the Rust body is a quake_rs_* status core and the jump is
 * re-issued from here, never from a Rust frame.
 */

/* cl_parse.c:105. External linkage but declared in no header; r_part_fte.c:156
   re-declares it locally and calls it at :4458, :5961 and :7251, so the plain
   name must stay a real C function. */
entity_t *CL_EntityNum (int num)
{
	void *ent = NULL;
	int	  r = quake_rs_cl_entity_num (num, &ent);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, num, 0, NULL);
	return (entity_t *)ent;
}

/* cl_parse.c:840. External linkage but declared in no header. */
void CL_ParseLocalSound (void)
{
	int detail = 0;
	int r = quake_rs_cl_parse_local_sound (&detail);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, detail, 0, NULL);
}

/* cl_parse.c:1527 (client.h:419). The Sys_Error inside aborts, but
   R_TranslatePlayerSkin is guarded, so a status can still come back. */
void CL_NewTranslation (int slot)
{
	int detail = 0;
	int r = quake_rs_cl_new_translation (slot, &detail);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, detail, 0, NULL);
}

/* cl_parse.c:1671 (client.h:418); called by r_part_fte.c:3644 and :6645. */
void CL_RegisterParticles (void)
{
	int detail = 0;
	int r = quake_rs_cl_register_particles (&detail);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, detail, 0, NULL);
}

/* cl_parse.c:1784 (client.h:417); called by cl_main.c:988. */
void CL_ParseServerMessage (void)
{
	int			a = 0, b = 0;
	const char *s = NULL;
	int			r = quake_rs_cl_parse_server_message (&a, &b, &s);
	if (r != CLPARSE_OK)
		ClParse_Raise (r, a, b, s);
}

#endif /* USE_RUST_HOST */
