/* Phase 7 M7 oracle fixture TU for Quake/cl_main.c (T7.4).
 *
 * c_ref_prelude.h is force-included (build.rs) and already includes the real
 * Quake/client.h, so client_state_t, client_static_t, entity_t, dlight_t and
 * scoreboard_t are the engine's own declarations here. Quake/cl_main.c is an
 * oracle source, so every one of its entry points is reachable as
 * c_ref_<name>.
 *
 * Same three roles cl_parse_ref.c / cl_tent_ref.c / cl_input_ref.c play:
 *
 *  1. Define the PLAIN (Rust-reading) twins of everything Quake/cl_main_glue.c
 *     owns and nothing else already provides. cl_main_glue.c is gated
 *     `#ifdef USE_RUST_HOST` and is not in build.rs's C_SOURCES, and cl_main.c
 *     is an oracle source whose every symbol is renamed, so without this file
 *     those objects have no definition under their plain name. The
 *     authoritative list came from a link probe (`cargo test -p quake-ctest
 *     --no-run`), which reported exactly 41 ClMain_Glue_* trampolines, ten
 *     plain command entry points and fifteen objects. cl_shownet and
 *     cl_lightstyle are NOT here: cl_parse_ref.c already owns their plain
 *     twins, and a stub object may be defined only once across the whole
 *     stubs archive.
 *
 *  2. Re-implement ClMain_Raise and the 41 ADR-009 trampolines, mirroring
 *     Quake/cl_main_glue.c's bodies exactly, plus the re-raising entry points
 *     the Rust core installs as command handlers.
 *
 *  3. Provide the fixture seeders and read-backs, every one of which writes
 *     BOTH the plain copy and the c_ref_ copy in the same call.
 *
 * WHAT MOVED IN T7.4, and why it changes the shape of this file relative to
 * cl_parse_ref.c: ADR-007's cl/cls row closes here. `cl` and `cls` are now
 * Rust storage (quake-capi/src/cl_main.rs), so the plain side reads Rust's
 * objects and the oracle reads cl_main.c's c_ref_cl / c_ref_cls -- still two
 * distinct copies, still two seeds per call, but stubs.c no longer defines
 * the plain pair.
 *
 * DEGENERATE-FIXTURE HAZARD, stated deliberately: nothing in this link runs
 * CL_Init or CL_ParseServerInfo, so cl.entities / cl.scores / cl_dlights /
 * cls.message are NULL/0 from static init on BOTH sides unless seeded. A
 * bit-exact differential passes happily when both sides do nothing to
 * nothing: with cl.entities NULL, CL_PrintEntities_f prints nothing on both
 * sides and CL_RelinkEntities walks zero entities. ctest_clmain_reset
 * therefore publishes a live starting state into both copies, and every test
 * asserts something positive alongside the cross-side comparison.
 *
 * CALLEE SELECTION (the rule sv_send_ref.c:1051 records): a trampoline calls
 * the SAME unrenamed helper the real glue calls wherever that helper is a
 * single shared stubs.c symbol in this link -- which covers the R_*, SCR_*,
 * PScript_*, NET_*, Host_*, Key_EndChat, IN_Move, PR_ClearProgs, BGM_Stop and
 * CDAudio_Stop group. Where the prelude renames the callee AND a plain twin
 * exists (Cvar_Set, Cvar_SetValue, Cvar_RegisterVariable, Cbuf_InsertText,
 * Cvar_FindVar, S_StopAllSounds, CL_ParseServerMessage, CL_UpdateTEnts) the
 * name is #undef'd so the port's trampoline reaches the port's copy, exactly
 * as the real glue would. Where the prelude renames it and NO plain twin
 * exists (TraceLine, PR_SetEngineString) the oracle's copy is the only one in
 * the link and both sides share it, so it cannot be a source of divergence.
 *
 * MSG_Write* is the one callee with neither shape: quake-capi exports the
 * writers as quake_rs_msg_write_* status cores that Quake/net_msg_glue.c
 * wraps (ADR-009: SZ_GetSpace can Host_Error), so this link has no plain
 * MSG_WriteByte at all. ClMain_Glue_WriteBatch drives the cores and
 * Host_Reraise's a non-zero status from inside the guard, which is precisely
 * what net_msg_glue.c does -- the guard catches it and the caller sees the
 * same GUARD_HOST_ERROR the oracle's MSG_WriteByte would have produced.
 *
 * HARNESS-ONLY RAISE HAZARD: stubs.c:48-61 makes Sys_Error longjmp when
 * armed, so driving any abort stub through the Rust port longjmps across Rust
 * frames. That is safe only because every driver below enters through
 * Host_Guard, whose setjmp sits in a pure C frame outside the Rust call. It
 * is a property of the harness, not of the port.
 *
 * WHAT THE ABORT STUBS COST, stated so it is not mistaken for coverage:
 * CL_FreeState reaches stubs.c's PR_ClearProgs abort stub, CL_Disconnect
 * reaches Host_ShutdownServer, CL_SendCmd reaches NET_CanSendMessage and
 * CL_ReadFromServer reaches CL_GetMessage. Both sides stop at the first stub,
 * identically, so what those functions do AFTER that point is not covered
 * here -- the comparison is of which stub was reached, with what message, and
 * of every observable mutation made before it.
 *
 * CL_RelinkEntities used to be on that list, gated by SCR_UpdateZoom at
 * cl_main.c:681 -- before the entity loop, so the loop, the teleport
 * threshold at :502 and the frametime clamp at :670 were all unreachable and
 * mutating any of them changed nothing any test could see. SCR_UpdateZoom,
 * R_UpdateEntityDlights and the two PScript_* entry points are now counting
 * or recording stubs in stubs.c, each justified there against the real
 * function's behaviour. That is deliberately narrower than turning shared
 * abort stubs into no-ops: no-oping would delete the "reached a module that
 * is not an oracle source" tripwire for every other suite in the harness,
 * whereas counting keeps the seam observable. R_AllocateEntityBLAS and
 * R_RocketTrail remain abort stubs and still bound this suite.
 */

#include <string.h>

/* Host_Guard/Host_Reraise live in stubs.c and are not declared by any header
 * the prelude pulls in (the real engine declares them via host.h). */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* quake-capi/src/cl_main.rs's status cores. cl_main_glue.c gets these
 * prototypes from the generated quake_rs.h, which this link has no
 * counterpart for. */
extern int	 quake_rs_cl_clear_trail_states (void);
extern int	 quake_rs_cl_free_state (void);
extern int	 quake_rs_cl_clear_state (void);
extern int	 quake_rs_cl_disconnect (void);
extern int	 quake_rs_cl_disconnect_f (void);
extern int	 quake_rs_cl_establish_connection (const char *host);
extern int	 quake_rs_cl_send_initial_userinfo (void *ctx, const char *key, const char *val);
extern int	 quake_rs_cl_signon_reply (void);
extern int	 quake_rs_cl_next_demo (void);
extern int	 quake_rs_cl_print_entities_f (void);
extern void *quake_rs_cl_alloc_dlight (int key);
extern void	 quake_rs_cl_decay_lights (void);
extern float quake_rs_cl_lerp_point (void);
extern int	 quake_rs_cl_relink_entities (void);
extern int	 quake_rs_cl_generate_random_particle_precache (const char *pname, int *out);
extern int	 quake_rs_cl_read_from_server (int *out);
extern int	 quake_rs_cl_accumulate_cmd (void);
extern int	 quake_rs_cl_send_cmd (void);
extern int	 quake_rs_cl_tracepos_f (void);
extern int	 quake_rs_cl_viewpos_f (void);
extern int	 quake_rs_cl_viewpos_completion_f (const char *partial);
extern int	 quake_rs_cl_serverext_full_serverinfo_f (void);
extern int	 quake_rs_cl_serverext_serverinfo_update_f (void);
extern int	 quake_rs_cl_serverext_full_userinfo_f (void);
extern int	 quake_rs_cl_serverext_userinfo_update_f (void);
extern int	 quake_rs_cl_serverext_ignore_f (void);
extern int	 quake_rs_cl_legacy_color_f (void);
extern int	 quake_rs_sv_update_info (int edict, const char *keyname, const char *value);

/* quake-capi/src/net_msg.rs -- the writers the WriteBatch trampoline replays.
 * There is no plain MSG_WriteByte in this link; see the module doc. */
extern int quake_rs_msg_write_byte (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_string (sizebuf_t *sb, const char *s);

/* ==========================================================================
 * 1. Plain (Rust-reading) storage this wave owns.
 *
 * The prelude's rename macros are live in this TU and would rewrite every
 * definition below to c_ref_*, colliding with the real oracle objects
 * compiled from cl_main.c (LNK2005), so each name is #undef'd first. Once
 * #undef'd the bare name means the PLAIN copy for the rest of the file;
 * oracle access always spells c_ref_* by hand.
 *
 * cvar_t.value is normally filled in by Cvar_RegisterVariable, which never
 * runs for these in this link, so ctest_clmain_reset seeds every .value
 * explicitly -- a zeroed .value flattens the branch that reads it on BOTH
 * sides at once, which is the vacuous shape this milestone has been bitten by
 * before.
 */

#undef cl_name
cvar_t cl_name = {"_cl_name", "player", CVAR_ARCHIVE | CVAR_USERINFO};

#undef cl_topcolor
cvar_t cl_topcolor = {"topcolor", "0", CVAR_ARCHIVE | CVAR_USERINFO};
#undef cl_bottomcolor
cvar_t cl_bottomcolor = {"bottomcolor", "0", CVAR_ARCHIVE | CVAR_USERINFO};

#undef cl_nolerp
cvar_t cl_nolerp = {"cl_nolerp", "0", CVAR_NONE};

#undef cfg_unbindall
cvar_t cfg_unbindall = {"cfg_unbindall", "1", CVAR_ARCHIVE};

#undef lookstrafe
cvar_t lookstrafe = {"lookstrafe", "0", CVAR_NONE};
#undef sensitivity
cvar_t sensitivity = {"sensitivity", "3", CVAR_ARCHIVE};

#undef m_pitch
cvar_t m_pitch = {"m_pitch", "0.022", CVAR_ARCHIVE};
#undef m_yaw
cvar_t m_yaw = {"m_yaw", "0.022", CVAR_ARCHIVE};
#undef m_forward
cvar_t m_forward = {"m_forward", "1", CVAR_ARCHIVE};
#undef m_side
cvar_t m_side = {"m_side", "0.8", CVAR_ARCHIVE};

#undef cl_startdemos
cvar_t cl_startdemos = {"cl_startdemos", "1", CVAR_ARCHIVE};
#undef cl_confirmquit
cvar_t cl_confirmquit = {"cl_confirmquit", "0", CVAR_ARCHIVE};

#undef cl_dlights
dlight_t cl_dlights[MAX_DLIGHTS];

/* cl_tent_ref.c owns the plain cl_visedicts / cl_numvisedicts /
   cl_maxvisedicts trio; only the alpha lists are unclaimed. */
#undef cl_visedicts_alpha
entity_t **cl_visedicts_alpha;
#undef cl_numvisedicts_alpha_overwater
int cl_numvisedicts_alpha_overwater;
#undef cl_numvisedicts_alpha_underwater
int cl_numvisedicts_alpha_underwater;

/* ADR-007: quake-capi/src/cl_main.rs defines the plain pair now; cl_main.c
   defines c_ref_cl / c_ref_cls. Both sides are read here, so the rename must
   be off and both spellings written out. */
#undef cl
#undef cls
#undef sv
extern server_t sv;
extern client_state_t  cl;
extern client_static_t cls;

/* Renamed callees with a plain twin elsewhere in the link. After the #undef
   the bare name means the port's copy for the rest of this file, which is the
   copy the real glue would reach. */
#undef Cvar_Set
#undef Cvar_SetValue
#undef Cvar_RegisterVariable
#undef Cvar_FindVar
#undef Cbuf_InsertText
#undef S_StopAllSounds
#undef CL_ParseServerMessage
#undef CL_UpdateTEnts
extern void	   Cvar_Set (const char *var_name, const char *value);
extern void	   Cvar_SetValue (const char *var_name, float value);
extern void	   Cvar_RegisterVariable (cvar_t *variable);
extern cvar_t *Cvar_FindVar (const char *var_name);
extern void	   Cbuf_InsertText (const char *text);
extern void	   S_StopAllSounds (qboolean clear, qboolean stopmusic);
extern void	   CL_ParseServerMessage (void);
extern void	   CL_UpdateTEnts (void);

/* Entry points this file defines. All are renamed by the prelude because
   cl_main.c defines them, and their only declarations came from headers the
   prelude had already renamed, so each needs a plain prototype after the
   #undef. */
#undef CL_Disconnect_f
#undef CL_PrintEntities_f
#undef CL_Tracepos_f
#undef CL_Viewpos_f
#undef CL_Viewpos_Completion_f
#undef CL_LegacyColor_f
#undef CL_SendInitialUserinfo
#undef CL_ServerExtension_FullServerinfo_f
#undef CL_ServerExtension_ServerinfoUpdate_f
#undef CL_ServerExtension_FullUserinfo_f
#undef CL_ServerExtension_UserinfoUpdate_f
#undef CL_ServerExtension_Ignore_f
void CL_Disconnect_f (void);
void CL_PrintEntities_f (void);
void CL_Tracepos_f (void);
void CL_Viewpos_f (void);
void CL_Viewpos_Completion_f (const char *partial);
void CL_LegacyColor_f (void);
void CL_SendInitialUserinfo (void *ctx, const char *key, const char *val);
void CL_ServerExtension_FullServerinfo_f (void);
void CL_ServerExtension_ServerinfoUpdate_f (void);
void CL_ServerExtension_FullUserinfo_f (void);
void CL_ServerExtension_UserinfoUpdate_f (void);
void CL_ServerExtension_Ignore_f (void);

/* Oracle twins with no declaration in scope. c_ref_prelude.h renames the
   symbol but the declaration it would have come from either does not exist
   (cl_confirmquit is a header-less external -- menu.c:118 declares it by hand)
   or lives in a header the prelude does not reach. */
extern cvar_t c_ref_cl_confirmquit;
extern void	  c_ref_CL_PrintEntities_f (void);
extern void	  c_ref_CL_Tracepos_f (void);
extern void	  c_ref_CL_Viewpos_f (void);

/* Shared stubs.c abort stub with no declaration in scope. */
extern void BGM_Stop (void);

/* The oracle twins this file drives. cl_main.c's own declarations are in
   client.h, which the prelude renamed, so these are already in scope under
   their c_ref_ spelling -- except the two the header does not declare. */
extern dlight_t *c_ref_CL_AllocDlight (int key);
extern float	 c_ref_CL_LerpPoint (void);

/* cl_parse_ref.c owns the plain CL_ClearState / CL_FreeState / CL_SignonReply
   / CL_GenerateRandomParticlePrecache twins (hand transcriptions of cl_main.c
   made when cl/cls were still C-owned). They are NOT redefined here -- one
   definition per stubs archive -- and the drivers below deliberately call the
   Rust cores directly rather than those twins, so this suite tests the port
   and not cl_parse_ref.c's transcription. */

/* ==========================================================================
 * 2. ADR-009 status codes and trampolines, mirroring Quake/cl_main_glue.c.
 */

#define CLMAIN_RAISE_CONNECT_FAILED (-101)
#define CLMAIN_RAISE_LOST_READ		(-102)
#define CLMAIN_RAISE_LOST_SEND		(-103)

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
			Host_Reraise (quake_rs_msg_write_byte (a->sb, op->i));
			break;
		case 1:
			Host_Reraise (quake_rs_msg_write_string (a->sb, (const char *)op->p));
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

static void ClMain_InvokePRClearProgs (void *p)
{
	PR_ClearProgs ((qcvm_t *)((clmain_arg_t *)p)->p);
}
int ClMain_Glue_PRClearProgs (void *vm)
{
	clmain_arg_t arg = {0};
	arg.p = vm;
	return Host_Guard (ClMain_InvokePRClearProgs, &arg);
}

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

static void ClMain_InvokeHostClearMemory (void *p)
{
	(void)p;
	Host_ClearMemory ();
}
int ClMain_Glue_HostClearMemory (void)
{
	return Host_Guard (ClMain_InvokeHostClearMemory, NULL);
}

static void ClMain_InvokePScriptShutdown (void *p)
{
	(void)p;
	PScript_Shutdown ();
}
int ClMain_Glue_PScriptShutdown (void)
{
	return Host_Guard (ClMain_InvokePScriptShutdown, NULL);
}

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

static void ClMain_InvokeKeyEndChat (void *p)
{
	(void)p;
	Key_EndChat ();
}
int ClMain_Glue_KeyEndChat (void)
{
	return Host_Guard (ClMain_InvokeKeyEndChat, NULL);
}

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

static void ClMain_InvokeCenterPrintClear (void *p)
{
	(void)p;
	SCR_CenterPrintClear ();
}
int ClMain_Glue_CenterPrintClear (void)
{
	return Host_Guard (ClMain_InvokeCenterPrintClear, NULL);
}

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

static void ClMain_InvokeEndLoadingPlaque (void *p)
{
	(void)p;
	SCR_EndLoadingPlaque ();
}
int ClMain_Glue_EndLoadingPlaque (void)
{
	return Host_Guard (ClMain_InvokeEndLoadingPlaque, NULL);
}

static void ClMain_InvokeBeginLoadingPlaque (void *p)
{
	(void)p;
	SCR_BeginLoadingPlaque ();
}
int ClMain_Glue_BeginLoadingPlaque (void)
{
	return Host_Guard (ClMain_InvokeBeginLoadingPlaque, NULL);
}

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

static void ClMain_InvokeUpdateZoom (void *p)
{
	(void)p;
	SCR_UpdateZoom ();
}
int ClMain_Glue_UpdateZoom (void)
{
	return Host_Guard (ClMain_InvokeUpdateZoom, NULL);
}

static void ClMain_InvokeInvalidateTraceLineCache (void *p)
{
	(void)p;
	InvalidateTraceLineCache ();
}
int ClMain_Glue_InvalidateTraceLineCache (void)
{
	return Host_Guard (ClMain_InvokeInvalidateTraceLineCache, NULL);
}

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

static void ClMain_InvokeUpdateEntityDlights (void *p)
{
	(void)p;
	R_UpdateEntityDlights ();
}
int ClMain_Glue_UpdateEntityDlights (void)
{
	return Host_Guard (ClMain_InvokeUpdateEntityDlights, NULL);
}

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
	/* pr_ext.c:4928 passes a NULL axis; r_part_fte.c accepts it. */
	if (!a->axis)
	{
		PScript_ParticleTrail (s, e, a->type, a->timeinterval, a->dlkey, NULL, (struct trailstate_s **)a->tsk);
		return;
	}
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

static void ClMain_InvokeParseServerMessage (void *p)
{
	(void)p;
	CL_ParseServerMessage ();
}
int ClMain_Glue_ParseServerMessage (void)
{
	return Host_Guard (ClMain_InvokeParseServerMessage, NULL);
}

static void ClMain_InvokeUpdateTEnts (void *p)
{
	(void)p;
	CL_UpdateTEnts ();
}
int ClMain_Glue_UpdateTEnts (void)
{
	return Host_Guard (ClMain_InvokeUpdateTEnts, NULL);
}

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

static void ClMain_InvokeRegisterVariable (void *p)
{
	Cvar_RegisterVariable ((cvar_t *)p);
}
int ClMain_Glue_RegisterVariable (cvar_t *var)
{
	return Host_Guard (ClMain_InvokeRegisterVariable, var);
}

/* Non-raising shims. */

void ClMain_Glue_SetViewposCompletion (void *cmd)
{
	if (cmd)
		((cmd_function_t *)cmd)->completion = CL_Viewpos_Completion_f;
}

/* The real glue calls SDL_SetClipboardText. There is no SDL in the ctest
   link (the whole point of the SDL-free core-header rule), so the text is
   captured for read-back instead. This is a harness substitution, not a port
   behaviour: what the differential can compare is the string cl_main.c would
   have handed SDL, which is the only part either side computes. */
static char ctest_clmain_clip[1024];

void ClMain_Glue_SetClipboardText (const char *text)
{
	if (!text)
		ctest_clmain_clip[0] = '\0';
	else
		q_snprintf (ctest_clmain_clip, sizeof (ctest_clmain_clip), "%s", text);
}

const char *ctest_clmain_clipboard (void)
{
	return ctest_clmain_clip;
}

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

/* ==========================================================================
 * 3. Plain entry points, mirroring cl_main_glue.c's re-raising wrappers.
 * Only the ones the link probe reported unresolved plus CL_SendInitialUserinfo
 * (which ClMain_Glue_InfoEnumerate passes to Info_Enumerate as a callback).
 */

void CL_SendInitialUserinfo (void *ctx, const char *key, const char *val)
{
	ClMain_Raise (quake_rs_cl_send_initial_userinfo (ctx, key, val));
}

void CL_Disconnect_f (void)
{
	ClMain_Raise (quake_rs_cl_disconnect_f ());
}

void CL_PrintEntities_f (void)
{
	ClMain_Raise (quake_rs_cl_print_entities_f ());
}

void CL_Tracepos_f (void)
{
	ClMain_Raise (quake_rs_cl_tracepos_f ());
}

void CL_Viewpos_f (void)
{
	ClMain_Raise (quake_rs_cl_viewpos_f ());
}

void CL_Viewpos_Completion_f (const char *partial)
{
	ClMain_Raise (quake_rs_cl_viewpos_completion_f (partial));
}

void CL_LegacyColor_f (void)
{
	ClMain_Raise (quake_rs_cl_legacy_color_f ());
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

/* ==========================================================================
 * 4. Fixture storage. Two of everything, seeded identically, so an accidental
 * write by one side shows up as a difference instead of propagating into the
 * other side's view.
 */

#define CTEST_CLMAIN_ENTITIES 64
#define CTEST_CLMAIN_MODELS	  8

static entity_t ctest_clmain_ents[CTEST_CLMAIN_ENTITIES];
static entity_t ctest_clmain_oracle_ents[CTEST_CLMAIN_ENTITIES];
static qmodel_t ctest_clmain_models[CTEST_CLMAIN_MODELS];

static scoreboard_t ctest_clmain_scores[MAX_SCOREBOARD];
static scoreboard_t ctest_clmain_oracle_scores[MAX_SCOREBOARD];

static byte ctest_clmain_msgbuf[8192];
static byte ctest_clmain_oracle_msgbuf[8192];

static client_state_t *ctest_clmain_cl (int side)
{
	return side ? &c_ref_cl : &cl;
}

static client_static_t *ctest_clmain_cls (int side)
{
	return side ? &c_ref_cls : &cls;
}

static dlight_t *ctest_clmain_dlights (int side)
{
	return side ? c_ref_cl_dlights : cl_dlights;
}

/* ==========================================================================
 * 5. Seeders. Every one writes BOTH sides in the same call.
 */

void ctest_clmain_set_time (double time, double oldtime, double mtime0, double mtime1)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_state_t *c = ctest_clmain_cl (i);
		c->time = time;
		c->oldtime = oldtime;
		c->mtime[0] = mtime0;
		c->mtime[1] = mtime1;
	}
}

void ctest_clmain_set_conn (int state, int signon, int demoplayback, int demorecording)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_static_t *s = ctest_clmain_cls (i);
		s->state = (cactive_t)state;
		s->signon = signon;
		s->demoplayback = demoplayback ? true : false;
		s->demorecording = demorecording ? true : false;
	}
}

void ctest_clmain_set_timedemo (int timedemo)
{
	int i;
	for (i = 0; i < 2; i++)
		ctest_clmain_cls (i)->timedemo = timedemo ? true : false;
}

void ctest_clmain_set_demoloop (int demonum, int count, const char *prefix)
{
	int i, j;
	for (i = 0; i < 2; i++)
	{
		client_static_t *s = ctest_clmain_cls (i);
		s->demonum = demonum;
		memset (s->demos, 0, sizeof (s->demos));
		for (j = 0; j < count && j < MAX_DEMOS; j++)
			q_snprintf (s->demos[j], MAX_DEMONAME, "%s%i", prefix, j);
	}
}

void ctest_clmain_set_counts (int maxclients, int viewentity, int num_entities)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_state_t *c = ctest_clmain_cl (i);
		c->maxclients = maxclients;
		c->viewentity = viewentity;
		c->num_entities = num_entities;
	}
}

void ctest_clmain_set_paused (int paused, int intermission)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_state_t *c = ctest_clmain_cl (i);
		c->paused = paused ? true : false;
		c->intermission = intermission;
	}
}

/* CL_LerpPoint's third disable arm is `sv.active && !host_netinterval`. `sv`
   is renamed, so the port reads quake-capi's sv and the oracle reads
   sv_main.c's c_ref_sv; host_netinterval is a single shared stubs.c object.
   Both copies are written here so the arm is drivable at all. */
/* `sv` has TWO copies: quake-capi/src/sv_main.rs owns the plain one (M6) and
   sv_main.c owns c_ref_sv. Without the #undef above, BOTH assignments here
   would land on c_ref_sv and CL_LerpPoint's `sv.active && !host_netinterval`
   arm would be taken on the oracle side only. Caught by a failing
   differential, not by reading. */
void ctest_clmain_set_sv_active (int active)
{
	sv.active = active ? true : false;
	c_ref_sv.active = active ? true : false;
}

void ctest_clmain_set_nolerp (float v)
{
	cl_nolerp.value = c_ref_cl_nolerp.value = v;
}

/* Non-degenerate dlights: a live one that CL_DecayLights must shrink, a dead
   one it must skip, a radius-0 one it must skip, and a keyed one
   CL_AllocDlight's exact-match scan must find. Identical on both sides. */
void ctest_clmain_seed_dlights (void)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		dlight_t *dl = ctest_clmain_dlights (i);
		memset (dl, 0, sizeof (dlight_t) * MAX_DLIGHTS);

		dl[0].die = 100.0f;
		dl[0].radius = 200.0f;
		dl[0].decay = 300.0f;
		dl[0].key = 0;
		dl[0].origin[0] = 1.0f;

		dl[1].die = 0.5f; /* < cl.time -> expired */
		dl[1].radius = 50.0f;
		dl[1].decay = 10.0f;
		dl[1].key = 11;

		dl[2].die = 100.0f;
		dl[2].radius = 0.0f; /* skipped by the !dl->radius arm */
		dl[2].decay = 10.0f;
		dl[2].key = 22;

		dl[3].die = 100.0f;
		dl[3].radius = 12.0f;
		dl[3].decay = 4.0f;
		dl[3].key = 33;
	}
}

void ctest_clmain_set_dlight (int idx, int key, float die, float radius, float decay)
{
	int i;
	if (idx < 0 || idx >= MAX_DLIGHTS)
		return;
	for (i = 0; i < 2; i++)
	{
		dlight_t *dl = &ctest_clmain_dlights (i)[idx];
		dl->key = key;
		dl->die = die;
		dl->radius = radius;
		dl->decay = decay;
	}
}

void ctest_clmain_attach_arrays (int nedicts)
{
	int i;

	if (nedicts < 0)
		nedicts = 0;
	if (nedicts > CTEST_CLMAIN_ENTITIES)
		nedicts = CTEST_CLMAIN_ENTITIES;

	memset (ctest_clmain_ents, 0, sizeof (ctest_clmain_ents));
	memset (ctest_clmain_oracle_ents, 0, sizeof (ctest_clmain_oracle_ents));
	memset (ctest_clmain_scores, 0, sizeof (ctest_clmain_scores));
	memset (ctest_clmain_oracle_scores, 0, sizeof (ctest_clmain_oracle_scores));

	for (i = 0; i < CTEST_CLMAIN_MODELS; i++)
	{
		memset (&ctest_clmain_models[i], 0, sizeof (ctest_clmain_models[i]));
		ctest_clmain_models[i].numframes = 4;
		ctest_clmain_models[i].type = mod_alias;
		ctest_clmain_models[i].synctype = ST_SYNC;
		q_snprintf (ctest_clmain_models[i].name, sizeof (ctest_clmain_models[i].name), "progs/ctest%i.mdl", i);
	}

	cl.entities = ctest_clmain_ents;
	c_ref_cl.entities = ctest_clmain_oracle_ents;
	cl.scores = ctest_clmain_scores;
	c_ref_cl.scores = ctest_clmain_oracle_scores;
	cl.max_edicts = c_ref_cl.max_edicts = nedicts;
}

/* Gives entity `idx` a model, a frame and a pose so CL_PrintEntities_f prints
   the populated arm rather than "EMPTY" and CL_RelinkEntities has something to
   lerp. Both sides get the SAME shared qmodel_t: it is read-only here, so a
   single copy cannot hide a divergence, and using two would make the printed
   name differ by construction. */
void ctest_clmain_set_entity (int idx, int model, int frame, float x, float y, float z, float pitch, float yaw, float roll)
{
	int i;
	if (idx < 0 || idx >= CTEST_CLMAIN_ENTITIES)
		return;
	for (i = 0; i < 2; i++)
	{
		entity_t *e = &(i ? ctest_clmain_oracle_ents : ctest_clmain_ents)[idx];
		e->model = (model >= 0 && model < CTEST_CLMAIN_MODELS) ? &ctest_clmain_models[model] : NULL;
		e->frame = frame;
		e->origin[0] = x;
		e->origin[1] = y;
		e->origin[2] = z;
		e->angles[0] = pitch;
		e->angles[1] = yaw;
		e->angles[2] = roll;
	}
}

/* ctest_clmain_set_entity writes origin/angles but not msg_origins/msg_angles,
   so CL_LerpEntity (cl_main.c:484, reached from :725) always saw a zero delta
   and its teleport threshold was unobservable -- the same degenerate shape one
   level down. msgtime matters just as much: cl_main.c:664 drops any entity
   whose msgtime differs from cl.mtime[0], and an unseeded 0 never matches, so
   before this setter the loop nulled every model and reached no lerp at all.
   Writes BOTH copies. */
void ctest_clmain_set_entity_msg (int idx, double msgtime, const float *o0, const float *o1, const float *a0, const float *a1)
{
	int i, j;
	if (idx < 0 || idx >= CTEST_CLMAIN_ENTITIES)
		return;
	for (i = 0; i < 2; i++)
	{
		entity_t *e = &(i ? ctest_clmain_oracle_ents : ctest_clmain_ents)[idx];
		e->msgtime = msgtime;
		for (j = 0; j < 3; j++)
		{
			e->msg_origins[0][j] = o0[j];
			e->msg_origins[1][j] = o1[j];
			e->msg_angles[0][j] = a0[j];
			e->msg_angles[1][j] = a1[j];
		}
	}
}

/* CL_RelinkEntities interpolates cl.velocity from cl.mvelocity at
   cl_main.c:679, and nothing else in this fixture writes mvelocity -- a zeroed
   pair makes that interpolation compare 0 against 0 on both sides, which is
   exactly the degenerate shape the module doc warns about. Writes BOTH
   copies. */
void ctest_clmain_set_mvelocity (const float *v0, const float *v1)
{
	int i;
	for (i = 0; i < 2; i++)
	{
		client_state_t *c = ctest_clmain_cl (i);
		VectorCopy (v0, c->mvelocity[0]);
		VectorCopy (v1, c->mvelocity[1]);
		c->velocity[0] = c->velocity[1] = c->velocity[2] = 0.0f;
	}
}

void ctest_clmain_attach_message (void)
{
	cls.message.data = ctest_clmain_msgbuf;
	cls.message.maxsize = (int)sizeof (ctest_clmain_msgbuf);
	cls.message.cursize = 0;
	cls.message.allowoverflow = false;
	cls.message.overflowed = false;

	c_ref_cls.message.data = ctest_clmain_oracle_msgbuf;
	c_ref_cls.message.maxsize = (int)sizeof (ctest_clmain_oracle_msgbuf);
	c_ref_cls.message.cursize = 0;
	c_ref_cls.message.allowoverflow = false;
	c_ref_cls.message.overflowed = false;
}

/* Shrinks cls.message so the ClMain_Glue_WriteBatch overflow arm is
   reachable: SZ_GetSpace Host_Errors, the guard catches it, and the Rust side
   must report the same status and message as the oracle's MSG_WriteByte. */
void ctest_clmain_set_message_maxsize (int maxsize)
{
	if (maxsize < 0)
		maxsize = 0;
	if (maxsize > (int)sizeof (ctest_clmain_msgbuf))
		maxsize = (int)sizeof (ctest_clmain_msgbuf);
	cls.message.maxsize = maxsize;
	c_ref_cls.message.maxsize = maxsize;
	if (cls.message.cursize > maxsize)
		cls.message.cursize = maxsize;
	if (c_ref_cls.message.cursize > maxsize)
		c_ref_cls.message.cursize = maxsize;
}

void ctest_clmain_set_userinfo (const char *info)
{
	q_snprintf (cls.userinfo, sizeof (cls.userinfo), "%s", info ? info : "");
	q_snprintf (c_ref_cls.userinfo, sizeof (c_ref_cls.userinfo), "%s", info ? info : "");
}

void ctest_clmain_set_serverinfo (const char *info)
{
	q_snprintf (cl.serverinfo, sizeof (cl.serverinfo), "%s", info ? info : "");
	q_snprintf (c_ref_cl.serverinfo, sizeof (c_ref_cl.serverinfo), "%s", info ? info : "");
}

/* ==========================================================================
 * 6. Read-backs.
 *
 * client_state_t is compared as a normalized byte image rather than through
 * field getters, which would silently miss whatever they forgot to list. The
 * normalization zeroes exactly the members whose VALUES are allowed to differ
 * between the two sides -- the pointers into the two separate fixture arrays,
 * the heap strings, and the embedded qcvm -- and nothing else.
 */

static void ctest_clmain_normalize (client_state_t *c)
{
	int i;
	for (i = 0; i < MAX_CL_STATS; i++)
		c->statss[i] = NULL;
	c->entities = NULL;
	c->static_entities = NULL;
	c->scores = NULL;
	c->efrag_allocs = NULL;
	c->free_efrags = NULL;
	for (i = 0; i < MAX_PARTICLETYPES; i++)
	{
		c->particle_precache[i].name = NULL;
		c->local_particle_precache[i].name = NULL;
	}
	memset (&c->qcvm, 0, sizeof (c->qcvm));
}

int ctest_clmain_cl_image_size (void)
{
	return (int)sizeof (client_state_t);
}

void ctest_clmain_get_cl_image (int side, void *out)
{
	memcpy (out, ctest_clmain_cl (side), sizeof (client_state_t));
	ctest_clmain_normalize ((client_state_t *)out);
}

int ctest_clmain_cls_image_size (void)
{
	return (int)sizeof (client_static_t);
}

void ctest_clmain_get_cls_image (int side, void *out)
{
	client_static_t *s = (client_static_t *)out;
	memcpy (out, ctest_clmain_cls (side), sizeof (client_static_t));
	s->message.data = NULL;
	s->demofile = NULL;
	s->netcon = NULL;
}

int ctest_clmain_dlight_size (void)
{
	return (int)sizeof (dlight_t);
}

void ctest_clmain_get_dlight (int side, int idx, void *out)
{
	if (idx < 0 || idx >= MAX_DLIGHTS)
	{
		memset (out, 0, sizeof (dlight_t));
		return;
	}
	memcpy (out, &ctest_clmain_dlights (side)[idx], sizeof (dlight_t));
}

int ctest_clmain_entity_size (void)
{
	return (int)sizeof (entity_t);
}

void ctest_clmain_get_entity (int side, int idx, void *out)
{
	if (idx < 0 || idx >= CTEST_CLMAIN_ENTITIES)
	{
		memset (out, 0, sizeof (entity_t));
		return;
	}
	memcpy (out, &(side ? ctest_clmain_oracle_ents : ctest_clmain_ents)[idx], sizeof (entity_t));
}

/* Scalar read-backs. The struct images already carry these bytes, but an
   image comparison only says "the two sides agree"; a test that wants to
   claim a specific value was computed has to read the value. */
double ctest_clmain_get_cl_time (int side)
{
	return ctest_clmain_cl (side)->time;
}

float ctest_clmain_get_dlight_radius (int side, int idx)
{
	if (idx < 0 || idx >= MAX_DLIGHTS)
		return -1.0f;
	return ctest_clmain_dlights (side)[idx].radius;
}

void ctest_clmain_get_velocity (int side, float *out)
{
	VectorCopy (ctest_clmain_cl (side)->velocity, out);
}

int ctest_clmain_get_message_size (int side)
{
	return ctest_clmain_cls (side)->message.cursize;
}

const unsigned char *ctest_clmain_get_message_data (int side)
{
	return side ? ctest_clmain_oracle_msgbuf : ctest_clmain_msgbuf;
}

const char *ctest_clmain_get_userinfo (int side)
{
	return ctest_clmain_cls (side)->userinfo;
}

const char *ctest_clmain_get_serverinfo (int side)
{
	return ctest_clmain_cl (side)->serverinfo;
}

/* cl.scores lives in the two fixture arrays rather than in client_state_t, so
   the normalized cl image cannot show it. The CL_ServerExtension_*Userinfo_f
   handlers write nothing else, so without these three the only thing left to
   assert about them would be that they did not crash. */
static void ctest_clmain_invoke_register_color_cvars (void *p)
{
	(void)p;
	Cvar_RegisterVariable (&cl_topcolor);
	Cvar_RegisterVariable (&cl_bottomcolor);
}

const char *ctest_clmain_get_score_userinfo (int side, int slot)
{
	if (slot < 0 || slot >= MAX_SCOREBOARD)
		return "";
	return (side ? ctest_clmain_oracle_scores : ctest_clmain_scores)[slot].userinfo;
}

const char *ctest_clmain_get_score_name (int side, int slot)
{
	if (slot < 0 || slot >= MAX_SCOREBOARD)
		return "";
	return (side ? ctest_clmain_oracle_scores : ctest_clmain_scores)[slot].name;
}

int ctest_clmain_get_score_colors (int side, int slot)
{
	if (slot < 0 || slot >= MAX_SCOREBOARD)
		return -1;
	return (side ? ctest_clmain_oracle_scores : ctest_clmain_scores)[slot].colors;
}

/* CL_LegacyColor_f resolves "topcolor"/"bottomcolor" by NAME through the Rust
   cvar table. Nothing in this link runs CL_Init, so they are registered here,
   once per process -- a second registration of the same name is an error, and
   the tests are order-independent. */
int ctest_clmain_register_color_cvars (void)
{
	static int done = 0;
	int		   r;
	if (done)
		return 0;
	r = Host_Guard (ctest_clmain_invoke_register_color_cvars, NULL);
	if (!r)
		done = 1;
	return r;
}

float ctest_clmain_get_color_cvar (int which)
{
	return which ? cl_bottomcolor.value : cl_topcolor.value;
}

/* ==========================================================================
 * 7. Drivers. Every entry point is entered through Host_Guard, so the setjmp
 * that catches an armed Sys_Error/Host_Error always sits in a pure C frame
 * outside the Rust call. The return value is the CTEST_GUARD_* status: 0 ok,
 * 1 Host_Error, 2 Sys_Error; the message is readable through stubs.c's
 * ctest_host_error_message() / ctest_sys_error_message().
 *
 * The Rust side calls the quake_rs_* core DIRECTLY rather than a plain
 * wrapper wherever cl_parse_ref.c already owns a hand-transcribed plain twin
 * of that name (CL_ClearState, CL_FreeState, CL_SignonReply,
 * CL_GenerateRandomParticlePrecache). Routing through those twins would test
 * cl_parse_ref.c's C transcription instead of the port, which is exactly the
 * vacuous shape this file exists to avoid.
 */

typedef struct
{
	int	  side;
	int	  i;
	int	  out;
	float f;
	void *p;
	void *q;
} clmain_drv_t;

static void ctest_clmain_invoke_lerp_point (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	a->f = a->side ? c_ref_CL_LerpPoint () : quake_rs_cl_lerp_point ();
}

int ctest_clmain_lerp_point (int side, float *out)
{
	clmain_drv_t a;
	int			 r;
	memset (&a, 0, sizeof (a));
	a.side = side;
	r = Host_Guard (ctest_clmain_invoke_lerp_point, &a);
	*out = a.f;
	return r;
}

static void ctest_clmain_invoke_alloc_dlight (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	dlight_t	 *dl = a->side ? c_ref_CL_AllocDlight (a->i) : (dlight_t *)quake_rs_cl_alloc_dlight (a->i);
	dlight_t	 *base = ctest_clmain_dlights (a->side);
	if (!dl)
		a->out = -1;
	else if (dl < base || dl >= base + MAX_DLIGHTS)
		a->out = -2;
	else
		a->out = (int)(dl - base);
}

/* Returns the guard status; *outidx receives the cl_dlights index the call
   handed back (-1 NULL, -2 outside the array), which is what lines the two
   sides up -- the raw pointers differ by construction. */
int ctest_clmain_alloc_dlight (int side, int key, int *outidx)
{
	clmain_drv_t a;
	int			 r;
	memset (&a, 0, sizeof (a));
	a.side = side;
	a.i = key;
	a.out = -3;
	r = Host_Guard (ctest_clmain_invoke_alloc_dlight, &a);
	*outidx = a.out;
	return r;
}

static void ctest_clmain_invoke_decay_lights (void *p)
{
	if (((clmain_drv_t *)p)->side)
		c_ref_CL_DecayLights ();
	else
		quake_rs_cl_decay_lights ();
}

int ctest_clmain_decay_lights (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_decay_lights, &a);
}

static void ctest_clmain_invoke_print_entities (void *p)
{
	if (((clmain_drv_t *)p)->side)
		c_ref_CL_PrintEntities_f ();
	else
		CL_PrintEntities_f ();
}

int ctest_clmain_print_entities (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_print_entities, &a);
}

static void ctest_clmain_invoke_signon_reply (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		c_ref_CL_SignonReply ();
	else
		ClMain_Raise (quake_rs_cl_signon_reply ());
}

int ctest_clmain_signon_reply (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_signon_reply, &a);
}

static void ctest_clmain_invoke_next_demo (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		c_ref_CL_NextDemo ();
	else
		ClMain_Raise (quake_rs_cl_next_demo ());
}

int ctest_clmain_next_demo (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_next_demo, &a);
}

static void ctest_clmain_invoke_relink (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		c_ref_CL_RelinkEntities ();
	else
		ClMain_Raise (quake_rs_cl_relink_entities ());
}

int ctest_clmain_relink_entities (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_relink, &a);
}

static void ctest_clmain_invoke_clear_state (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		c_ref_CL_ClearState ();
	else
		ClMain_Raise (quake_rs_cl_clear_state ());
}

int ctest_clmain_clear_state (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_clear_state, &a);
}

static void ctest_clmain_invoke_free_state (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		c_ref_CL_FreeState ();
	else
		ClMain_Raise (quake_rs_cl_free_state ());
}

int ctest_clmain_free_state (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_free_state, &a);
}

static void ctest_clmain_invoke_clear_trail_states (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		c_ref_CL_ClearTrailStates ();
	else
		ClMain_Raise (quake_rs_cl_clear_trail_states ());
}

int ctest_clmain_clear_trail_states (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_clear_trail_states, &a);
}

static void ctest_clmain_invoke_disconnect (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		c_ref_CL_Disconnect ();
	else
		ClMain_Raise (quake_rs_cl_disconnect ());
}

int ctest_clmain_disconnect (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_disconnect, &a);
}

static void ctest_clmain_invoke_disconnect_f (void *p)
{
	if (((clmain_drv_t *)p)->side)
		c_ref_CL_Disconnect_f ();
	else
		CL_Disconnect_f ();
}

int ctest_clmain_disconnect_f (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_disconnect_f, &a);
}

static void ctest_clmain_invoke_read_from_server (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		a->out = c_ref_CL_ReadFromServer ();
	else
	{
		int out = 0;
		ClMain_Raise (quake_rs_cl_read_from_server (&out));
		a->out = out;
	}
}

int ctest_clmain_read_from_server (int side, int *out)
{
	clmain_drv_t a;
	int			 r;
	memset (&a, 0, sizeof (a));
	a.side = side;
	r = Host_Guard (ctest_clmain_invoke_read_from_server, &a);
	*out = a.out;
	return r;
}

static void ctest_clmain_invoke_send_cmd (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		c_ref_CL_SendCmd ();
	else
		ClMain_Raise (quake_rs_cl_send_cmd ());
}

int ctest_clmain_send_cmd (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_send_cmd, &a);
}

static void ctest_clmain_invoke_accumulate_cmd (void *p)
{
	clmain_drv_t *a = (clmain_drv_t *)p;
	if (a->side)
		c_ref_CL_AccumulateCmd ();
	else
		ClMain_Raise (quake_rs_cl_accumulate_cmd ());
}

int ctest_clmain_accumulate_cmd (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_accumulate_cmd, &a);
}

extern void *ctest_world_model (void);
extern void	 ctest_world_reset (int vm_kind, int num_edicts);

/* CL_Tracepos_f traces from r_refdef.vieworg along vpn. Both are single
   shared stubs.c objects and TraceLine has no plain twin (CALLEE SELECTION
   above), so the trace itself is identical on the two sides by construction;
   what the comparison covers is CL_Tracepos_f's own arithmetic, its %i
   rounding and its Con_Printf call. With a NULL worldmodel
   SV_RecursiveHullCheck dereferences it and both sides die identically --
   the degenerate outcome this seeder exists to prevent. */
void ctest_clmain_attach_world (const float *org, const float *fwd)
{
	/* ctest_world_reset publishes its room into the ORACLE copy of cl only
	   (stubs.c is compiled with the prelude, so its `cl` is c_ref_cl): it
	   repoints cl.entities at its own array and zeroes cl.num_entities. Save
	   and restore what ctest_clmain_reset already put there, then republish
	   the one field this path needs on BOTH copies, so the two cl images stay
	   byte-comparable. */
	entity_t *saved_entities = c_ref_cl.entities;
	int		  saved_num_entities = c_ref_cl.num_entities;

	ctest_world_reset (0, 2);
	c_ref_cl.entities = saved_entities;
	c_ref_cl.num_entities = saved_num_entities;
	cl.worldmodel = (qmodel_t *)ctest_world_model ();
	c_ref_cl.worldmodel = (qmodel_t *)ctest_world_model ();
	VectorCopy (org, r_refdef.vieworg);
	VectorCopy (fwd, vpn);
}

static void ctest_clmain_invoke_tracepos (void *p)
{
	if (((clmain_drv_t *)p)->side)
		c_ref_CL_Tracepos_f ();
	else
		CL_Tracepos_f ();
}

int ctest_clmain_tracepos (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_tracepos, &a);
}

static void ctest_clmain_invoke_viewpos (void *p)
{
	if (((clmain_drv_t *)p)->side)
		c_ref_CL_Viewpos_f ();
	else
		CL_Viewpos_f ();
}

int ctest_clmain_viewpos (int side)
{
	clmain_drv_t a;
	memset (&a, 0, sizeof (a));
	a.side = side;
	return Host_Guard (ctest_clmain_invoke_viewpos, &a);
}

/* CL_LegacyColor_f, the five CL_ServerExtension_*_f handlers and
 * CL_Viewpos_Completion_f are `static` in Quake/cl_main.c (lines 1178, 1185,
 * 1190, 1214, 1225, 1345, 1350), so they have no external linkage and
 * c_ref_prelude.h -- which only renames non-static symbols -- gives them no
 * c_ref_ twin. There is no linkable oracle copy to compare against: the only
 * handle cl_main.c ever offers is the function pointer CL_Init hands to
 * Cmd_AddCommand2, and CL_Init is not runnable in this link.
 *
 * These seven are therefore driven on the Rust side ONLY, with assertions
 * against expected values read off cl_main.c rather than against a live
 * oracle. That is a real and deliberate coverage gap, recorded here so it is
 * not mistaken for a passing differential.
 */

static void ctest_clmain_invoke_rust_legacy_color (void *p)
{
	(void)p;
	CL_LegacyColor_f ();
}

int ctest_clmain_rust_legacy_color (void)
{
	return Host_Guard (ctest_clmain_invoke_rust_legacy_color, NULL);
}

static void ctest_clmain_invoke_rust_serverext (void *p)
{
	switch (*(int *)p)
	{
	case 0:
		CL_ServerExtension_FullServerinfo_f ();
		break;
	case 1:
		CL_ServerExtension_ServerinfoUpdate_f ();
		break;
	case 2:
		CL_ServerExtension_FullUserinfo_f ();
		break;
	case 3:
		CL_ServerExtension_UserinfoUpdate_f ();
		break;
	default:
		CL_ServerExtension_Ignore_f ();
		break;
	}
}

int ctest_clmain_rust_serverext (int which)
{
	int w = which;
	return Host_Guard (ctest_clmain_invoke_rust_serverext, &w);
}

static void ctest_clmain_invoke_rust_viewpos_completion (void *p)
{
	CL_Viewpos_Completion_f ((const char *)p);
}

int ctest_clmain_rust_viewpos_completion (const char *partial)
{
	return Host_Guard (ctest_clmain_invoke_rust_viewpos_completion, (void *)partial);
}

/* ==========================================================================
 * 8. Whole-fixture reset. Publishes a non-degenerate starting state into BOTH
 * copies of everything: a 64-entry entity array with models attached, a live
 * cl_dlights table, a writable 8KB cls.message, a connected client at signon
 * 1 (so CL_SignonReply's reachable transitions are 1 -> 2 and 2 -> 3), and a
 * cl.time / mtime pair that puts CL_LerpPoint on its interpolating arm rather
 * than either early return.
 *
 * cl.statss[] and cl.particle_precache[].name hold q_strdup'd storage that
 * the client replaces in place, so both are freed here rather than merely
 * zeroed.
 */
void ctest_clmain_reset (void)
{
	int i, j;

	for (j = 0; j < 2; j++)
	{
		client_state_t *c = ctest_clmain_cl (j);
		for (i = 0; i < MAX_CL_STATS; i++)
		{
			Mem_Free (c->statss[i]);
			c->statss[i] = NULL;
		}
		for (i = 0; i < MAX_PARTICLETYPES; i++)
		{
			Mem_Free (c->particle_precache[i].name);
			c->particle_precache[i].name = NULL;
			c->particle_precache[i].index = 0;
			c->local_particle_precache[i].name = NULL;
			c->local_particle_precache[i].index = 0;
		}
	}

	/* qcvm is normalized out of every comparison and nothing in this link
	   loads client progs, so wiping the rest of the struct is safe. */
	memset (&cl.movemessages, 0, offsetof (client_state_t, qcvm) - offsetof (client_state_t, movemessages));
	memset (&c_ref_cl.movemessages, 0, offsetof (client_state_t, qcvm) - offsetof (client_state_t, movemessages));
	cl.zoom = c_ref_cl.zoom = 0.0f;
	cl.zoomdir = c_ref_cl.zoomdir = 0.0f;
	memset (cl.serverinfo, 0, sizeof (cl.serverinfo));
	memset (c_ref_cl.serverinfo, 0, sizeof (c_ref_cl.serverinfo));

	memset (&cls.spawnparms, 0, sizeof (cls.spawnparms));
	memset (&c_ref_cls.spawnparms, 0, sizeof (c_ref_cls.spawnparms));
	memset (cls.userinfo, 0, sizeof (cls.userinfo));
	memset (c_ref_cls.userinfo, 0, sizeof (c_ref_cls.userinfo));
	cls.demopaused = c_ref_cls.demopaused = false;
	cls.demoseeking = c_ref_cls.demoseeking = false;
	cls.seektime = c_ref_cls.seektime = 0.0f;
	cls.demospeed = c_ref_cls.demospeed = 0.0f;
	cls.demo_prespawn_end = c_ref_cls.demo_prespawn_end = 0;
	cls.forcetrack = c_ref_cls.forcetrack = 0;
	cls.demofile = c_ref_cls.demofile = NULL;
	cls.netcon = c_ref_cls.netcon = NULL;
	cls.td_lastframe = c_ref_cls.td_lastframe = 0;
	cls.td_startframe = c_ref_cls.td_startframe = 0;
	cls.td_starttime = c_ref_cls.td_starttime = 0.0f;

	/* Cvar_RegisterVariable never runs in this link, so the .value fields
	   the port and the oracle both branch on are seeded by hand. */
	cl_name.value = c_ref_cl_name.value = 0.0f;
	cl_topcolor.value = c_ref_cl_topcolor.value = 0.0f;
	cl_bottomcolor.value = c_ref_cl_bottomcolor.value = 0.0f;
	cl_nolerp.value = c_ref_cl_nolerp.value = 0.0f;
	cfg_unbindall.value = c_ref_cfg_unbindall.value = 1.0f;
	lookstrafe.value = c_ref_lookstrafe.value = 0.0f;
	sensitivity.value = c_ref_sensitivity.value = 3.0f;
	m_pitch.value = c_ref_m_pitch.value = 0.022f;
	m_yaw.value = c_ref_m_yaw.value = 0.022f;
	m_forward.value = c_ref_m_forward.value = 1.0f;
	m_side.value = c_ref_m_side.value = 0.8f;
	cl_startdemos.value = c_ref_cl_startdemos.value = 1.0f;
	cl_confirmquit.value = c_ref_cl_confirmquit.value = 0.0f;

	ctest_clmain_clip[0] = '\0';

	ctest_clmain_set_sv_active (0);
	ctest_clmain_attach_arrays (CTEST_CLMAIN_ENTITIES);
	ctest_clmain_attach_message ();
	ctest_clmain_seed_dlights ();
	ctest_clmain_set_time (1.5, 1.4, 1.55, 1.5);
	ctest_clmain_set_counts (4, 1, 8);
	ctest_clmain_set_conn ((int)ca_connected, 1, 0, 0);
	ctest_clmain_set_timedemo (0);
	ctest_clmain_set_demoloop (-1, 0, "ctest");
	ctest_clmain_set_paused (0, 0);

	for (i = 0; i < 8; i++)
		ctest_clmain_set_entity (i, i % CTEST_CLMAIN_MODELS, i, (float)i, (float)(i * 2), (float)(i * 3), (float)i, (float)(i * 10), (float)(i * 5));
}
