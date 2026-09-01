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
// cl_main_glue.c -- the C frame around the Rust client main-loop port.
//
// Compiled instead of cl_main.c under -Duse_rust_host (Rust migration Phase 7
// M7, T7.4), mirroring cl_input_glue.c, cl_tent_glue.c and cl_parse_glue.c:
//
//  1. Own the C-visible objects cl_main.c defined, except cl/cls. ADR-007's
//     client dual-view row closes here: `cl` and `cls` move to Rust storage
//     (quake-capi/src/cl_main.rs) because Rust is now their only writer of
//     record. Everything else cl_main.c defined -- the seventeen cvars,
//     cl_lightstyle[], cl_dlights[], the visedicts counters and arrays, and
//     needs_relink -- stays C storage here, exactly like cl_tent_glue.c keeps
//     cl_temp_entities[]/cl_beams[]: those objects have many C readers and no
//     ADR-007 dual-view problem, so moving them would be churn.
//
//     Five of them have external linkage but no header declaration --
//     cl_maxpitch and cl_minpitch (used by cl_input.c and in_sdl.c),
//     cl_confirmquit (menu.c), needs_relink (view.c) and
//     CL_GenerateRandomParticlePrecache (cl_parse.c) -- so this file must
//     define them under those exact names or those translation units fail to
//     link. The header-less-external defect class is why the Pattern A
//     checklist enumerates every non-static file-scope symbol before a flip.
//
//  2. Guard everything cl_main.c reached that can Host_Error / Host_EndGame
//     (ADR-009 rule 3). Notably: every MSG_Write* run into cls.message or a
//     client's message (batched -- each write reaches SZ_GetSpace, which
//     Host_Errors at net_msg.c:488), the NET_* funnels, Host_ShutdownServer,
//     CL_ParseServerMessage, CL_UpdateTEnts, Cbuf_InsertText, Cvar_Set/
//     Cvar_SetValue (arbitrary cvar callbacks), Key_EndChat, the PScript_*
//     and R_* entry points, and TraceLine.
//
//  3. Re-raise, from a pure C frame, what those guards caught. cl_main.c has
//     three Host_Error sites of its own (:232 CL_EstablishConnection,
//     :983 CL_ReadFromServer, :1108 CL_SendCmd); the Rust cores return
//     CLMAIN_RAISE_* for those and the plain wrappers below turn them back
//     into the original Host_Error call. Host_Reraise is called only here.
//
//  4. Leave everything else plain. Con_Printf and friends, Cmd_Argc/Cmd_Argv,
//     Cvar_RegisterVariable, Cmd_AddCommand2, the Mem_* allocator (which
//     Sys_Errors rather than jumping), va, q_snprintf, q_strlcpy, q_strdup,
//     Info_GetKey/Info_SetKey, the mathlib helpers and COM_Rand cannot
//     longjmp, so the Rust side calls them directly.
//
//     The static command handlers of cl_main.c become plain C wrappers here
//     rather than Rust function pointers: Cmd_ExecuteString invokes them from
//     C, and a Cvar_Set or Info_* call inside one must not longjmp across a
//     Rust frame.

#include "quakedef.h"
#include "bgmusic.h"

#include "steam.h" // quake_rs.h declares the Phase 2 Steam shims in terms of steamgame_t
#include "quake_rs.h"

#ifdef USE_RUST_HOST

#include "SDL3/SDL_clipboard.h"

/* ---------------------------------------------------------------------------
 * 1. C-visible storage (cl_main.c:29-72). Verbatim, including declaration
 * order -- Cvar_RegisterVariable order is observable in config.cfg and the
 * addresses of cl_lightstyle/cl_dlights are taken by other translation units.
 */

// these two are not intended to be set directly
cvar_t cl_name = {"_cl_name", "player", CVAR_ARCHIVE | CVAR_USERINFO};

cvar_t cl_topcolor = {"topcolor", "0", CVAR_ARCHIVE | CVAR_USERINFO};
cvar_t cl_bottomcolor = {"bottomcolor", "0", CVAR_ARCHIVE | CVAR_USERINFO};

cvar_t cl_shownet = {"cl_shownet", "0", CVAR_NONE}; // can be 0, 1, or 2
cvar_t cl_nolerp = {"cl_nolerp", "0", CVAR_NONE};

cvar_t cfg_unbindall = {"cfg_unbindall", "1", CVAR_ARCHIVE};

cvar_t lookspring = {"lookspring", "0", CVAR_NONE};
cvar_t lookstrafe = {"lookstrafe", "0", CVAR_NONE};
cvar_t sensitivity = {"sensitivity", "3", CVAR_ARCHIVE};

cvar_t m_pitch = {"m_pitch", "0.022", CVAR_ARCHIVE};
cvar_t m_yaw = {"m_yaw", "0.022", CVAR_ARCHIVE};
cvar_t m_forward = {"m_forward", "1", CVAR_ARCHIVE};
cvar_t m_side = {"m_side", "0.8", CVAR_ARCHIVE};

cvar_t cl_maxpitch = {"cl_maxpitch", "90", CVAR_ARCHIVE};  // johnfitz -- variable pitch clamping
cvar_t cl_minpitch = {"cl_minpitch", "-90", CVAR_ARCHIVE}; // johnfitz -- variable pitch clamping

cvar_t cl_startdemos = {"cl_startdemos", "1", CVAR_ARCHIVE};
cvar_t cl_confirmquit = {"cl_confirmquit", "0", CVAR_ARCHIVE};

// cl and cls now live in Rust (quake-capi/src/cl_main.rs); client.h's externs
// bind to that storage. ADR-007.

// FIXME: put these on hunk?
lightstyle_t cl_lightstyle[MAX_LIGHTSTYLES];
dlight_t	 cl_dlights[MAX_DLIGHTS];

int		   cl_numvisedicts;
int		   cl_numvisedicts_alpha_overwater;
int		   cl_numvisedicts_alpha_underwater;
int		   cl_maxvisedicts;
entity_t **cl_visedicts;
entity_t **cl_visedicts_alpha;

qboolean needs_relink;

/* ---------------------------------------------------------------------------
 * 2. Guards.
 */

/* Raise codes for cl_main.c's own three Host_Error sites. Distinct from the
 * GUARD_* statuses Host_Guard returns, so a core can report either. */
/* Linkage under Pattern A: a cl_main.c file-static keeps 'static' here unless
 * the code that takes its address moved to Rust. CL_Init did, and
 * quake-capi/src/cl_main.rs:2095-2104 declares these six extern "C" to register
 * them, so they must have external linkage even though cl_main.c:1188+ makes
 * them static -- an unavoidable consequence of the flip, not a transliteration
 * slip. CL_Viewpos_Completion_f stays static: its address is taken only at
 * :767 in this file. CL_SendInitialUserinfo is external in cl_main.c:241
 * already. All are referenced above their definitions, hence these forward
 * declarations. */
void CL_LegacyColor_f (void);
void CL_ServerExtension_FullServerinfo_f (void);
void CL_ServerExtension_FullUserinfo_f (void);
void CL_ServerExtension_Ignore_f (void);
void CL_ServerExtension_ServerinfoUpdate_f (void);
void CL_ServerExtension_UserinfoUpdate_f (void);
static void CL_Viewpos_Completion_f (const char *partial);
void CL_SendInitialUserinfo (void *ctx, const char *key, const char *val);

#define CLMAIN_RAISE_CONNECT_FAILED	  (-101)
#define CLMAIN_RAISE_LOST_READ		  (-102)
#define CLMAIN_RAISE_LOST_SEND		  (-103)

/* Batched, guarded sizebuf writers. Every MSG_Write* reaches SZ_GetSpace
 * (net_msg.c:481), which Host_Errors on overflow, so no Rust frame may sit
 * under one. A run of writes is buffered on the Rust side and replayed here
 * inside a single Host_Guard; the emitted byte stream is identical for any
 * batch size because the ops replay in insertion order. */

typedef struct
{
	int			kind;
	int			i;
	const void *p;
} clmain_write_t;

typedef struct
{
	sizebuf_t			 *sb;
	const clmain_write_t *ops;
	int					  count;
} clmain_writebatch_arg_t;

static void ClMain_InvokeWriteBatch (void *p)
{
	clmain_writebatch_arg_t *a = (clmain_writebatch_arg_t *)p;
	int						 k;

	for (k = 0; k < a->count; k++)
	{
		const clmain_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			MSG_WriteByte (a->sb, op->i);
			break;
		case 1:
			MSG_WriteString (a->sb, (const char *)op->p);
			break;
		default:
			Sys_Error ("ClMain_InvokeWriteBatch: unknown op %i", op->kind);
			break;
		}
	}
}

int ClMain_Glue_WriteBatch (void *sb, const clmain_write_t *ops, int count)
{
	clmain_writebatch_arg_t arg;
	arg.sb = (sizebuf_t *)sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (ClMain_InvokeWriteBatch, &arg);
}

/* Shared argument shapes for the single-operand guards. */
typedef struct
{
	void *p;
	void *q;
	int	  i;
	float f;
} clmain_arg_t;

typedef struct
{
	const char *s;
	const char *t;
	int			i;
	float		f;
	void	  **outp;
	int		   *out;
} clmain_sarg_t;

/* cl_main.c:99 -- PR_ClearProgs (&cl.qcvm) */
static void ClMain_InvokePRClearProgs (void *p)
{
	PR_ClearProgs ((qcvm_t *)((clmain_arg_t *)p)->p);
}
int ClMain_Glue_PRClearProgs (void *qcvm)
{
	clmain_arg_t arg = {0};
	arg.p = qcvm;
	return Host_Guard (ClMain_InvokePRClearProgs, &arg);
}

/* cl_main.c:104, :109, :719 -- R_FreeEntityBLAS (ent) */
static void ClMain_InvokeFreeEntityBLAS (void *p)
{
	R_FreeEntityBLAS ((entity_t *)((clmain_arg_t *)p)->p);
}
int ClMain_Glue_FreeEntityBLAS (void *ent)
{
	clmain_arg_t arg = {0};
	arg.p = ent;
	return Host_Guard (ClMain_InvokeFreeEntityBLAS, &arg);
}

/* cl_main.c:133 -- Host_ClearMemory () */
static void ClMain_InvokeHostClearMemory (void *p)
{
	(void)p;
	Host_ClearMemory ();
}
int ClMain_Glue_HostClearMemory (void)
{
	return Host_Guard (ClMain_InvokeHostClearMemory, NULL);
}

/* cl_main.c:158 -- PScript_Shutdown () */
static void ClMain_InvokePScriptShutdown (void *p)
{
	(void)p;
	PScript_Shutdown ();
}
int ClMain_Glue_PScriptShutdown (void)
{
	return Host_Guard (ClMain_InvokePScriptShutdown, NULL);
}

/* cl_main.c:80-90 -- PScript_DelinkTrailstate (&ts) */
static void ClMain_InvokeDelinkTrailstate (void *p)
{
	PScript_DelinkTrailstate ((struct trailstate_s **)((clmain_arg_t *)p)->p);
}
int ClMain_Glue_DelinkTrailstate (void *ts)
{
	clmain_arg_t arg = {0};
	arg.p = ts;
	return Host_Guard (ClMain_InvokeDelinkTrailstate, &arg);
}

/* cl_main.c:169 -- Key_EndChat () */
static void ClMain_InvokeKeyEndChat (void *p)
{
	(void)p;
	Key_EndChat ();
}
int ClMain_Glue_KeyEndChat (void)
{
	return Host_Guard (ClMain_InvokeKeyEndChat, NULL);
}

/* cl_main.c:172-174 -- the unconditional, adjacent audio stop group. */
static void ClMain_InvokeStopAudio (void *p)
{
	(void)p;
	S_StopAllSounds (true, false);
	BGM_Stop ();
	CDAudio_Stop ();
}
int ClMain_Glue_StopAudio (void)
{
	return Host_Guard (ClMain_InvokeStopAudio, NULL);
}

/* cl_main.c:189 -- NET_SendUnreliableMessage (cls.netcon, &cls.message) */
static void ClMain_InvokeNetSendUnreliable (void *p)
{
	clmain_arg_t *a = (clmain_arg_t *)p;
	a->i = NET_SendUnreliableMessage ((struct qsocket_s *)a->p, (sizebuf_t *)a->q);
}
int ClMain_Glue_NetSendUnreliable (void *sock, void *sb, int *out)
{
	clmain_arg_t arg = {0};
	int			 r;
	arg.p = sock;
	arg.q = sb;
	*out = 0;
	r = Host_Guard (ClMain_InvokeNetSendUnreliable, &arg);
	*out = arg.i;
	return r;
}

/* cl_main.c:191 -- NET_Close (cls.netcon) */
static void ClMain_InvokeNetClose (void *p)
{
	NET_Close ((struct qsocket_s *)((clmain_arg_t *)p)->p);
}
int ClMain_Glue_NetClose (void *sock)
{
	clmain_arg_t arg = {0};
	arg.p = sock;
	return Host_Guard (ClMain_InvokeNetClose, &arg);
}

/* cl_main.c:196, :211 -- Host_ShutdownServer (false) */
static void ClMain_InvokeHostShutdownServer (void *p)
{
	Host_ShutdownServer (((clmain_arg_t *)p)->i ? true : false);
}
int ClMain_Glue_HostShutdownServer (int crash)
{
	clmain_arg_t arg = {0};
	arg.i = crash;
	return Host_Guard (ClMain_InvokeHostShutdownServer, &arg);
}

/* cl_main.c:203 -- SCR_CenterPrintClear () */
static void ClMain_InvokeCenterPrintClear (void *p)
{
	(void)p;
	SCR_CenterPrintClear ();
}
int ClMain_Glue_CenterPrintClear (void)
{
	return Host_Guard (ClMain_InvokeCenterPrintClear, NULL);
}

/* cl_main.c:229 -- NET_Connect (host) */
static void ClMain_InvokeNetConnect (void *p)
{
	clmain_sarg_t *a = (clmain_sarg_t *)p;
	*a->outp = NET_Connect (a->s);
}
int ClMain_Glue_NetConnect (const char *host, void **out)
{
	clmain_sarg_t arg = {0};
	arg.s = host;
	arg.outp = out;
	*out = NULL;
	return Host_Guard (ClMain_InvokeNetConnect, &arg);
}

/* cl_main.c:278 -- Info_Enumerate (cls.userinfo, CL_SendInitialUserinfo, NULL).
 * The callback is this file's own CL_SendInitialUserinfo, which re-raises; the
 * jump lands in this guard, one frame up. */
static void ClMain_InvokeInfoEnumerate (void *p)
{
	Info_Enumerate ((const char *)((clmain_sarg_t *)p)->s, CL_SendInitialUserinfo, NULL);
}
int ClMain_Glue_InfoEnumerate (const char *info)
{
	clmain_sarg_t arg = {0};
	arg.s = info;
	return Host_Guard (ClMain_InvokeInfoEnumerate, &arg);
}

/* cl_main.c:290 -- SCR_EndLoadingPlaque () */
static void ClMain_InvokeEndLoadingPlaque (void *p)
{
	(void)p;
	SCR_EndLoadingPlaque ();
}
int ClMain_Glue_EndLoadingPlaque (void)
{
	return Host_Guard (ClMain_InvokeEndLoadingPlaque, NULL);
}

/* cl_main.c:319 -- SCR_BeginLoadingPlaque () */
static void ClMain_InvokeBeginLoadingPlaque (void *p)
{
	(void)p;
	SCR_BeginLoadingPlaque ();
}
int ClMain_Glue_BeginLoadingPlaque (void)
{
	return Host_Guard (ClMain_InvokeBeginLoadingPlaque, NULL);
}

/* cl_main.c:322 -- Cbuf_InsertText (str) */
static void ClMain_InvokeCbufInsertText (void *p)
{
	Cbuf_InsertText (((clmain_sarg_t *)p)->s);
}
int ClMain_Glue_CbufInsertText (const char *text)
{
	clmain_sarg_t arg = {0};
	arg.s = text;
	return Host_Guard (ClMain_InvokeCbufInsertText, &arg);
}

/* cl_main.c:680 -- SCR_UpdateZoom () */
static void ClMain_InvokeUpdateZoom (void *p)
{
	(void)p;
	SCR_UpdateZoom ();
}
int ClMain_Glue_UpdateZoom (void)
{
	return Host_Guard (ClMain_InvokeUpdateZoom, NULL);
}

/* cl_main.c:720 -- InvalidateTraceLineCache () */
static void ClMain_InvokeInvalidateTraceLineCache (void *p)
{
	(void)p;
	InvalidateTraceLineCache ();
}
int ClMain_Glue_InvalidateTraceLineCache (void)
{
	return Host_Guard (ClMain_InvokeInvalidateTraceLineCache, NULL);
}

/* cl_main.c:757 -- R_EntityParticles (ent) */
static void ClMain_InvokeEntityParticles (void *p)
{
	R_EntityParticles ((entity_t *)((clmain_arg_t *)p)->p);
}
int ClMain_Glue_EntityParticles (void *ent)
{
	clmain_arg_t arg = {0};
	arg.p = ent;
	return Host_Guard (ClMain_InvokeEntityParticles, &arg);
}

/* cl_main.c:640 -- R_RocketTrail (start, end, type) */
typedef struct
{
	const float *start;
	const float *end;
	int			 type;
} clmain_trail_arg_t;

static void ClMain_InvokeRocketTrail (void *p)
{
	clmain_trail_arg_t *a = (clmain_trail_arg_t *)p;
	vec3_t				s, e;
	VectorCopy (a->start, s);
	VectorCopy (a->end, e);
	R_RocketTrail (s, e, a->type);
}
int ClMain_Glue_RocketTrail (const float *start, const float *end, int type)
{
	clmain_trail_arg_t arg;
	arg.start = start;
	arg.end = end;
	arg.type = type;
	return Host_Guard (ClMain_InvokeRocketTrail, &arg);
}

/* cl_main.c:925 -- R_AllocateEntityBLAS (ent) */
static void ClMain_InvokeAllocateEntityBLAS (void *p)
{
	R_AllocateEntityBLAS ((entity_t *)((clmain_arg_t *)p)->p);
}
int ClMain_Glue_AllocateEntityBLAS (void *ent)
{
	clmain_arg_t arg = {0};
	arg.p = ent;
	return Host_Guard (ClMain_InvokeAllocateEntityBLAS, &arg);
}

/* cl_main.c:931 -- R_UpdateEntityDlights () */
static void ClMain_InvokeUpdateEntityDlights (void *p)
{
	(void)p;
	R_UpdateEntityDlights ();
}
int ClMain_Glue_UpdateEntityDlights (void)
{
	return Host_Guard (ClMain_InvokeUpdateEntityDlights, NULL);
}

/* cl_main.c:844, :850 -- PScript_ParticleTrail (...) */
typedef struct
{
	const float *start;
	const float *end;
	int			 type;
	float		 timeinterval;
	int			 dlkey;
	const float *axis;
	void	   **tsk;
} clmain_ptrail_arg_t;

static void ClMain_InvokeParticleTrail (void *p)
{
	clmain_ptrail_arg_t *a = (clmain_ptrail_arg_t *)p;
	vec3_t				 s, e, ax[3];
	VectorCopy (a->start, s);
	VectorCopy (a->end, e);
	VectorCopy (a->axis + 0, ax[0]);
	VectorCopy (a->axis + 3, ax[1]);
	VectorCopy (a->axis + 6, ax[2]);
	PScript_ParticleTrail (s, e, a->type, a->timeinterval, a->dlkey, ax, (struct trailstate_s **)a->tsk);
}
int ClMain_Glue_ParticleTrail (const float *start, const float *end, int type, float timeinterval, int dlkey, const float *axis, void **tsk)
{
	clmain_ptrail_arg_t arg;
	arg.start = start;
	arg.end = end;
	arg.type = type;
	arg.timeinterval = timeinterval;
	arg.dlkey = dlkey;
	arg.axis = axis;
	arg.tsk = tsk;
	return Host_Guard (ClMain_InvokeParticleTrail, &arg);
}

/* cl_main.c:857 etc -- PScript_EntParticleTrail (oldorg, ent, name) */
typedef struct
{
	const float *oldorg;
	void		*ent;
	const char	*name;
	int			 out;
} clmain_entrail_arg_t;

static void ClMain_InvokeEntParticleTrail (void *p)
{
	clmain_entrail_arg_t *a = (clmain_entrail_arg_t *)p;
	vec3_t				  o;
	VectorCopy (a->oldorg, o);
	a->out = PScript_EntParticleTrail (o, (entity_t *)a->ent, a->name);
}
int ClMain_Glue_EntParticleTrail (const float *oldorg, void *ent, const char *name, int *out)
{
	clmain_entrail_arg_t arg;
	int					 r;
	arg.oldorg = oldorg;
	arg.ent = ent;
	arg.name = name;
	arg.out = 0;
	*out = 0;
	r = Host_Guard (ClMain_InvokeEntParticleTrail, &arg);
	*out = arg.out;
	return r;
}

/* cl_main.c:901, :913 -- PScript_RunParticleEffectState (...) */
typedef struct
{
	const float *org;
	const float *dir;
	float		 count;
	int			 typenum;
	void	   **tsk;
} clmain_pstate_arg_t;

static void ClMain_InvokeRunParticleEffectState (void *p)
{
	clmain_pstate_arg_t *a = (clmain_pstate_arg_t *)p;
	vec3_t				 o, d;
	VectorCopy (a->org, o);
	VectorCopy (a->dir, d);
	PScript_RunParticleEffectState (o, d, a->count, a->typenum, (struct trailstate_s **)a->tsk);
}
int ClMain_Glue_RunParticleEffectState (const float *org, const float *dir, float count, int typenum, void **tsk)
{
	clmain_pstate_arg_t arg;
	arg.org = org;
	arg.dir = dir;
	arg.count = count;
	arg.typenum = typenum;
	arg.tsk = tsk;
	return Host_Guard (ClMain_InvokeRunParticleEffectState, &arg);
}

/* cl_main.c:948 -- PScript_FindParticleType (name) */
static void ClMain_InvokeFindParticleType (void *p)
{
	clmain_sarg_t *a = (clmain_sarg_t *)p;
	*a->out = PScript_FindParticleType (a->s);
}
int ClMain_Glue_FindParticleType (const char *name, int *out)
{
	clmain_sarg_t arg = {0};
	arg.s = name;
	arg.out = out;
	*out = 0;
	return Host_Guard (ClMain_InvokeFindParticleType, &arg);
}

/* cl_main.c:989 -- CL_ParseServerMessage () */
static void ClMain_InvokeParseServerMessage (void *p)
{
	(void)p;
	CL_ParseServerMessage ();
}
int ClMain_Glue_ParseServerMessage (void)
{
	return Host_Guard (ClMain_InvokeParseServerMessage, NULL);
}

/* cl_main.c:997 -- CL_UpdateTEnts () */
static void ClMain_InvokeUpdateTEnts (void *p)
{
	(void)p;
	CL_UpdateTEnts ();
}
int ClMain_Glue_UpdateTEnts (void)
{
	return Host_Guard (ClMain_InvokeUpdateTEnts, NULL);
}

/* cl_main.c:1054 -- IN_Move (&cl.pendingcmd) */
static void ClMain_InvokeInMove (void *p)
{
	IN_Move ((usercmd_t *)((clmain_arg_t *)p)->p);
}
int ClMain_Glue_InMove (void *cmd)
{
	clmain_arg_t arg = {0};
	arg.p = cmd;
	return Host_Guard (ClMain_InvokeInMove, &arg);
}

/* cl_main.c:1101 -- NET_CanSendMessage (cls.netcon) */
static void ClMain_InvokeNetCanSendMessage (void *p)
{
	clmain_arg_t *a = (clmain_arg_t *)p;
	a->i = NET_CanSendMessage ((struct qsocket_s *)a->p) ? 1 : 0;
}
int ClMain_Glue_NetCanSendMessage (void *sock, int *out)
{
	clmain_arg_t arg = {0};
	int			 r;
	arg.p = sock;
	*out = 0;
	r = Host_Guard (ClMain_InvokeNetCanSendMessage, &arg);
	*out = arg.i;
	return r;
}

/* cl_main.c:1107 -- NET_SendMessage (cls.netcon, &cls.message) */
static void ClMain_InvokeNetSendMessage (void *p)
{
	clmain_arg_t *a = (clmain_arg_t *)p;
	a->i = NET_SendMessage ((struct qsocket_s *)a->p, (sizebuf_t *)a->q);
}
int ClMain_Glue_NetSendMessage (void *sock, void *sb, int *out)
{
	clmain_arg_t arg = {0};
	int			 r;
	arg.p = sock;
	arg.q = sb;
	*out = 0;
	r = Host_Guard (ClMain_InvokeNetSendMessage, &arg);
	*out = arg.i;
	return r;
}

/* cl_main.c:1130 -- TraceLine (start, end, impact) */
typedef struct
{
	const float *start;
	const float *end;
	float		*impact;
} clmain_trace_arg_t;

static void ClMain_InvokeTraceLine (void *p)
{
	clmain_trace_arg_t *a = (clmain_trace_arg_t *)p;
	vec3_t				s, e, w;
	VectorCopy (a->start, s);
	VectorCopy (a->end, e);
	TraceLine (s, e, w);
	VectorCopy (w, a->impact);
}
int ClMain_Glue_TraceLine (const float *start, const float *end, float *impact)
{
	clmain_trace_arg_t arg;
	arg.start = start;
	arg.end = end;
	arg.impact = impact;
	return Host_Guard (ClMain_InvokeTraceLine, &arg);
}

/* cl_main.c:1212 -- R_TranslateNewPlayerSkin (slot) */
static void ClMain_InvokeTranslateNewPlayerSkin (void *p)
{
	R_TranslateNewPlayerSkin (((clmain_arg_t *)p)->i);
}
int ClMain_Glue_TranslateNewPlayerSkin (int slot)
{
	clmain_arg_t arg = {0};
	arg.i = slot;
	return Host_Guard (ClMain_InvokeTranslateNewPlayerSkin, &arg);
}

/* cl_main.c:1268 -- PR_SetEngineString (s) */
static void ClMain_InvokePRSetEngineString (void *p)
{
	clmain_sarg_t *a = (clmain_sarg_t *)p;
	*a->out = (int)PR_SetEngineString (a->s);
}
int ClMain_Glue_PRSetEngineString (const char *s, int *out)
{
	clmain_sarg_t arg = {0};
	arg.s = s;
	arg.out = out;
	*out = 0;
	return Host_Guard (ClMain_InvokePRSetEngineString, &arg);
}

/* cl_main.c:1286 -- Cvar_Set (var->name, value); the callback is arbitrary. */
static void ClMain_InvokeCvarSet (void *p)
{
	clmain_sarg_t *a = (clmain_sarg_t *)p;
	Cvar_Set (a->s, a->t);
}
int ClMain_Glue_CvarSet (const char *name, const char *value)
{
	clmain_sarg_t arg = {0};
	arg.s = name;
	arg.t = value;
	return Host_Guard (ClMain_InvokeCvarSet, &arg);
}

/* cl_main.c:1357-1358 -- Cvar_SetValue (name, v) */
static void ClMain_InvokeCvarSetValue (void *p)
{
	clmain_sarg_t *a = (clmain_sarg_t *)p;
	Cvar_SetValue (a->s, a->f);
}
int ClMain_Glue_CvarSetValue (const char *name, float value)
{
	clmain_sarg_t arg = {0};
	arg.s = name;
	arg.f = value;
	return Host_Guard (ClMain_InvokeCvarSetValue, &arg);
}

/* cl_main.c:1366-1394 -- the twenty-five Cvar_RegisterVariable calls in
   CL_Init. Under -Duse_rust_cvar Cvar_RegisterVariable is itself a
   Host_Reraise wrapper (ADR-009 rule 3), so it must not be called from a Rust
   frame. Mirrors Chase_Glue_RegisterVariable. */
static void ClMain_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}
int ClMain_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (ClMain_InvokeRegisterVariable, var);
}

/* ---------------------------------------------------------------------------
 * Non-raising shims that exist only because the operand type is opaque to
 * Rust (cmd_function_t) or lives in a platform header (SDL).
 */

/* cl_main.c:1414-1416 -- cmd->completion = CL_Viewpos_Completion_f */
void ClMain_Glue_SetViewposCompletion (void *cmd)
{
	if (cmd)
		((cmd_function_t *)cmd)->completion = CL_Viewpos_Completion_f;
}

/* cl_main.c:1173 -- SDL_SetClipboardText (buf) */
void ClMain_Glue_SetClipboardText (const char *text)
{
	SDL_SetClipboardText (text);
}

/* cl_main.c:1272 -- Cvar_FindVar + the CVAR_SERVERINFO test, so Rust never has
 * to know cvar_t's layout for a cvar it does not own. Returns 1 when the name
 * resolved to a CVAR_SERVERINFO cvar, and writes its canonical name. */
int ClMain_Glue_FindServerinfoCvar (const char *keyname, const char **out_name)
{
	cvar_t *var = Cvar_FindVar (keyname);
	*out_name = NULL;
	if (var && (var->flags & CVAR_SERVERINFO))
	{
		*out_name = var->name;
		return 1;
	}
	return 0;
}

/* ---------------------------------------------------------------------------
 * 3. Entry points. Each calls the Rust core and re-issues, from this pure C
 * frame, whatever the core reported.
 */

static void ClMain_Raise (int r)
{
	switch (r)
	{
	case CLMAIN_RAISE_CONNECT_FAILED:
		Host_Error ("CL_Connect: connect failed");
		break;
	case CLMAIN_RAISE_LOST_READ:
		Host_Error ("CL_ReadFromServer: lost server connection");
		break;
	case CLMAIN_RAISE_LOST_SEND:
		Host_Error ("CL_SendCmd: lost server connection");
		break;
	default:
		Host_Reraise (r);
		break;
	}
}

void CL_ClearTrailStates (void)
{
	ClMain_Raise (quake_rs_cl_clear_trail_states ());
}

void CL_FreeState (void)
{
	ClMain_Raise (quake_rs_cl_free_state ());
}

void CL_ClearState (void)
{
	ClMain_Raise (quake_rs_cl_clear_state ());
}

void CL_Disconnect (void)
{
	ClMain_Raise (quake_rs_cl_disconnect ());
}

void CL_Disconnect_f (void)
{
	ClMain_Raise (quake_rs_cl_disconnect_f ());
}

void CL_EstablishConnection (const char *host)
{
	ClMain_Raise (quake_rs_cl_establish_connection (host));
}

void CL_SendInitialUserinfo (void *ctx, const char *key, const char *val)
{
	ClMain_Raise (quake_rs_cl_send_initial_userinfo (ctx, key, val));
}

void CL_SignonReply (void)
{
	ClMain_Raise (quake_rs_cl_signon_reply ());
}

void CL_NextDemo (void)
{
	ClMain_Raise (quake_rs_cl_next_demo ());
}

void CL_PrintEntities_f (void)
{
	ClMain_Raise (quake_rs_cl_print_entities_f ());
}

dlight_t *CL_AllocDlight (int key)
{
	return (dlight_t *)quake_rs_cl_alloc_dlight (key);
}

void CL_DecayLights (void)
{
	quake_rs_cl_decay_lights ();
}

float CL_LerpPoint (void)
{
	return quake_rs_cl_lerp_point ();
}

void CL_RelinkEntities (void)
{
	ClMain_Raise (quake_rs_cl_relink_entities ());
}

int CL_GenerateRandomParticlePrecache (const char *pname)
{
	int out = 0;
	ClMain_Raise (quake_rs_cl_generate_random_particle_precache (pname, &out));
	return out;
}

int CL_ReadFromServer (void)
{
	int out = 0;
	ClMain_Raise (quake_rs_cl_read_from_server (&out));
	return out;
}

void CL_AccumulateCmd (void)
{
	ClMain_Raise (quake_rs_cl_accumulate_cmd ());
}

void CL_SendCmd (void)
{
	ClMain_Raise (quake_rs_cl_send_cmd ());
}

void CL_Tracepos_f (void)
{
	ClMain_Raise (quake_rs_cl_tracepos_f ());
}

void CL_Viewpos_f (void)
{
	ClMain_Raise (quake_rs_cl_viewpos_f ());
}

void SV_UpdateInfo (int edict, const char *keyname, const char *value)
{
	ClMain_Raise (quake_rs_sv_update_info (edict, keyname, value));
}

void CL_Init (void)
{
	ClMain_Raise (quake_rs_cl_init ());
}

/* The static command handlers of cl_main.c. Cmd_ExecuteString calls these from
 * C, so the re-raise has to happen here rather than under the Rust core. */

static void CL_Viewpos_Completion_f (const char *partial)
{
	ClMain_Raise (quake_rs_cl_viewpos_completion_f (partial));
}

void CL_ServerExtension_FullServerinfo_f (void)
{
	ClMain_Raise (quake_rs_cl_serverext_full_serverinfo_f ());
}

void CL_ServerExtension_ServerinfoUpdate_f (void)
{
	ClMain_Raise (quake_rs_cl_serverext_serverinfo_update_f ());
}

void CL_ServerExtension_FullUserinfo_f (void)
{
	ClMain_Raise (quake_rs_cl_serverext_full_userinfo_f ());
}

void CL_ServerExtension_UserinfoUpdate_f (void)
{
	ClMain_Raise (quake_rs_cl_serverext_userinfo_update_f ());
}

void CL_ServerExtension_Ignore_f (void)
{
	ClMain_Raise (quake_rs_cl_serverext_ignore_f ());
}

void CL_LegacyColor_f (void)
{
	ClMain_Raise (quake_rs_cl_legacy_color_f ());
}

#endif /* USE_RUST_HOST */
