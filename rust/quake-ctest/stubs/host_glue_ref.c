/* Phase 7 M8 T8.2: the ctest-link mirror of Quake/host_glue.c's glue-owned half.
 *
 * Quake/host_glue.c is gated on the Meson -Duse_rust_host flip and is not in
 * build.rs's C_SOURCES, while Quake/host.c is composed into stubs/host_ref.c
 * under a per-TU c_ref_* rename block. So every object and seam host_glue.c
 * owns has no definition under its plain name in this link, and quake-capi's
 * `host` feature -- which is on for this crate -- references all of them. This
 * file supplies exactly that set, and nothing else.
 *
 * Three jobs, mirroring the shape stubs/cl_main_ref.c and stubs/sv_main_ref.c
 * established:
 *
 *  1. Define the plain (Rust-reading) twins of the data host_glue.c owns:
 *     twenty-three of host.c's file-scope objects plus the nine host_glue_*
 *     preprocessor constants. The list is exactly the data half of the link
 *     probe (`cargo test -p quake-ctest --no-run`); host.c's other nineteen
 *     objects -- host_parms, host_initialized, host_frametime, realtime,
 *     host_framecount, minimum_memory, host_client, host_abortserver,
 *     screen_error, host_netinterval, sv_speeds, teamplay, skill, developer,
 *     devstats, dev_stats, dev_peakstats, dev_overflows -- already have plain
 *     definitions in stubs.c, and max_edicts/deathmatch/coop come from
 *     host_ref.c (which deliberately leaves those three unrenamed). Defining
 *     any of them here would be a duplicate symbol.
 *
 *  2. Transcribe host_glue.c's Host_Glue_* seams, keeping the
 *     Host_Guard (invoke_fn, &arg) trampoline shape verbatim. Host_Guard,
 *     Host_Reraise, Host_Error and Host_EndGame are stubs.c's, with the
 *     harness's CTEST_GUARD_OK/HOST_ERROR/SYS_ERROR result set rather than
 *     the real HOST_GUARD_* one (stubs.c:1434-1467 documents the departure);
 *     they are not redefined here.
 *
 *  3. Section 4 (added at T8.2) drives the Rust port from the test binary:
 *     ctest_host_rs_* accessors over the plain, Rust-read state, paired one
 *     for one with host_ref.c's ctest_host_* accessors over the c_ref_* state.
 *
 * CALLEE SPELLING. Every callee in sections 1-3 is spelled exactly the way
 * Quake/host.c spells it, and no #undef appears anywhere above section 4.
 * c_ref_prelude.h is force-included here and into host_ref.c alike, so its
 * rename macros rewrite this file's call sites and the oracle host.c's call
 * sites identically: the port's Host_Glue_Cbuf_Execute and the oracle's host.c
 * both reach c_ref_Cbuf_Execute, the port's Host_Glue_Mod_ClearAll and the
 * oracle both reach host_ref.c's plain Mod_ClearAll double. Both sides of the
 * differential therefore land on the same function by construction rather than
 * by audit. sv_main_ref.c:16-18 warns that relying on the prelude lets a stray
 * #undef silently redirect an oracle call -- that hazard is contained by
 * confining every #undef to section 4, which defines no seam and is textually
 * after all of them.
 *
 * MSG_Write* IS THE ONE EXCEPTION, and it follows cl_demo_ref.c:31 and
 * cl_main_ref.c:59-64 rather than the rule above. In the shipping -Duse_rust
 * build the writers are quake-capi's quake_rs_msg_write_* status cores wrapped
 * by Quake/net_msg_glue.c (ADR-009: SZ_GetSpace can Host_Error), so that is
 * what host_glue.c's MSG_WriteByte resolves to there. Host_Glue_WriteBatch and
 * Host_Glue_BroadcastDisconnect drive those cores directly and Host_Reraise a
 * non-zero status from inside the guard, exactly as net_msg_glue.c would; the
 * oracle keeps net_msg.c's c_ref_MSG_Write*, which is the comparison the
 * differential wants.
 *
 * Host_Glue_FrameInner IS A DEVIATION, stated so it is not mistaken for a
 * transcription. host_glue.c:912-918 is a setjmp on host_abortserver that
 * swallows a caught raise as _Host_Frame's early return did. This link has no
 * host_abortserver -- stubs.c's Host_Guard installs the harness's own
 * Host_Error/Sys_Error traps instead -- so the shell is expressed as a
 * Host_Guard whose result is DISCARDED rather than re-raised. That is the same
 * observable: a raise inside quake_rs_host_frame_core unwinds to this pure C
 * frame and the function returns, and a clean run returns normally.
 *
 * NO LINK DOUBLE IS DEFINED HERE, and that is a finding rather than an
 * omission: a link probe over the seam set found every callee already present
 * -- as host_ref.c's plain counting/aborting doubles, as an oracle source's
 * c_ref_* symbol, as a stubs.c symbol, or (BGM_Init / BGM_Update /
 * BGM_Shutdown) as quake-capi's own ported bgmusic.rs. Nothing needed a
 * silently-no-op stand-in, which is the defect class the earlier milestones
 * logged. PScript_InitParticles / PR_TraceInit / PR_TraceShutdown do have
 * return-HOST_GUARD_OK arms, but those are host_glue.c:487-508's own #ifdef
 * arms transcribed verbatim: PSET_SCRIPT is defined by the prelude (:544) so
 * the real guard is compiled, PR_TRACE is not (no -Dtrace=true) so host.c's
 * own call sites (host.c:1379, :1429) are compiled out too and the seam is
 * unreachable by construction, not by stubbing.
 */

#include <string.h>

/* stubs.c's raise machinery. The real engine declares these in host.h, which
 * c_ref_prelude.h does not pull in (cl_main_ref.c:95-98 does the same). */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* quake-capi/src/host.rs's status cores. host_glue.c gets these prototypes
 * from the generated quake_rs.h, which this link has no counterpart for. Only
 * the two this file re-raises are declared; the other entry points
 * (Host_Frame, Host_Init, SV_DropClient, ...) already have plain definitions
 * in stubs.c and are not this file's to own. */
extern int quake_rs_host_version_f (void);
extern int quake_rs_host_frame_core (double time);

/* quake-capi/src/net_msg.rs, as cl_demo_ref.c:74-79 declares them. */
extern int quake_rs_msg_write_byte (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_short (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_string (sizebuf_t *sb, const char *s);

/* Quake/bgmusic.h:30-35. The header is not reachable from this TU (the prelude
 * pre-empts the quakedef.h chain), so the three prototypes come by hand. The
 * DEFINITIONS are quake-capi's bgmusic.rs (an already-ported module in this
 * link), which is also what the shipping -Duse_rust build's host_glue.c calls,
 * so no double is needed or wanted here -- defining one would shadow a real
 * ported implementation. */
qboolean BGM_Init (void);
void	 BGM_Shutdown (void);
void	 BGM_Update (void);

/*
 * 1. Storage. Quake/host_glue.c:95-188, restricted to the objects that have no
 * other definition in this link. Names, defaults and flag sets are copied
 * exactly; the source line of each group is noted.
 */

/* host_glue.c:113-124. */
byte  *host_colormap;
cvar_t host_framerate = {"host_framerate", "0", CVAR_NONE}; // set for slow motion
cvar_t host_speeds = {"host_speeds", "0", CVAR_NONE};		// set for running times
cvar_t host_maxfps = {"host_maxfps", "200", CVAR_ARCHIVE};	// johnfitz

cvar_t host_phys_max_ticrate = {"host_phys_max_ticrate", "0", CVAR_NONE}; // vso = [0 = disabled; MAX_PHYSICS_FREQ]

cvar_t host_timescale = {"host_timescale", "0", CVAR_NONE}; // johnfitz
cvar_t cl_nocsqc = {"cl_nocsqc", "0", CVAR_NONE};			// spike -- blocks the loading of any csqc modules

/* host_glue.c:100-103. host_frametime and realtime are stubs.c's. */
double host_rawframetime; // unscaled and unbounded
double oldrealtime;		  // last frame run

/* host_glue.c:126-152. */
cvar_t sys_ticrate = {"sys_ticrate", "0.025", CVAR_NONE}; // dedicated server
cvar_t serverprofile = {"serverprofile", "0", CVAR_NONE};

cvar_t fraglimit = {"fraglimit", "0", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t timelimit = {"timelimit", "0", CVAR_NOTIFY | CVAR_SERVERINFO};
cvar_t samelevel = {"samelevel", "0", CVAR_NONE};
cvar_t noexit = {"noexit", "0", CVAR_NOTIFY | CVAR_SERVERINFO};

cvar_t pausable = {"pausable", "1", CVAR_NONE};

cvar_t autoload = {"autoload", "1", CVAR_ARCHIVE};
cvar_t autofastload = {"autofastload", "0", CVAR_ARCHIVE};

/* host_glue.c:145 -- pr_engine lost host.c:98's `static` in the glue, because
 * Host_InitLocal (its only address-taker) moved to Rust. */
cvar_t pr_engine = {"pr_engine", ENGINE_NAME_AND_VER, CVAR_NONE};
cvar_t temp1 = {"temp1", "0", CVAR_NONE};

cvar_t campaign = {"campaign", "0", CVAR_NONE};	  // for the 2021 rerelease
cvar_t horde = {"horde", "0", CVAR_NONE};		  // for the 2021 rerelease
cvar_t sv_cheats = {"sv_cheats", "0", CVAR_NONE}; // for the 2021 rerelease

/*
 * host_glue.c:164-173 -- the preprocessor-only values the glue owns on the
 * port's behalf. VERSION, QUAKESPASM_VER_STRING and ENGINE_NAME_AND_VER are
 * reachable here: c_ref_prelude.h:1217 includes the real Quake/quakever.h, so
 * these are the engine's own values rather than transcribed constants.
 * __TIME__/__DATE__ are stamped by the compiler as they are in the real build.
 */
const double	  host_glue_version = VERSION;
const char *const host_glue_quakespasm_ver = QUAKESPASM_VER_STRING;
const char *const host_glue_engine_ver = ENGINE_NAME_AND_VER;
const char *const host_glue_build_time = __TIME__;
const char *const host_glue_build_date = __DATE__;
#if defined(DEBUG) || defined(_DEBUG)
const char *const host_glue_build_suffix = "(DEBUG build)";
#else
const char *const host_glue_build_suffix = "";
#endif

/*
 * host_glue.c:180-188 -- Tests_Init's registration table (host.c:1277-1284).
 * The ctest link is a release build (no _DEBUG), so the count is zero and the
 * port's walk of the table is a no-trip loop, exactly as the release engine's
 * is. TestHashMap_f / GL_HeapTest_f / TestTasks_f are consequently not
 * referenced, which is why they are not in this link at all.
 */
#ifdef _DEBUG
const char *const host_glue_test_names[] = {"test_hash_map", "test_gl_heap", "test_tasks"};
xcommand_t		  host_glue_test_funcs[] = {TestHashMap_f, GL_HeapTest_f, TestTasks_f};
const int		  host_glue_num_tests = 3;
#else
const char *const host_glue_test_names[1] = {NULL};
xcommand_t		  host_glue_test_funcs[1] = {NULL};
const int		  host_glue_num_tests = 0;
#endif

/*
 * 2. Guarded seams, transcribed from host_glue.c:373-532. The macros collapse
 * only the boilerplate; the enumeration is spelled out exactly as the glue
 * spells it so the Pattern A symbol audit stays meaningful.
 */

#define HOST_GUARD_VOID(name)                         \
	static void Host_Invoke_##name (void *p)          \
	{                                                 \
		(void)p;                                      \
		name ();                                      \
	}                                                 \
	int Host_Glue_##name (void)                       \
	{                                                 \
		return Host_Guard (Host_Invoke_##name, NULL); \
	}

#define HOST_GUARD_PTR(name, type)                 \
	static void Host_Invoke_##name (void *p)       \
	{                                              \
		name ((type)p);                            \
	}                                              \
	int Host_Glue_##name (void *a)                 \
	{                                              \
		return Host_Guard (Host_Invoke_##name, a); \
	}

#define HOST_GUARD_INT(name, type)                  \
	static void Host_Invoke_##name (void *p)        \
	{                                               \
		name ((type) * (int *)p);                   \
	}                                               \
	int Host_Glue_##name (int a)                    \
	{                                               \
		return Host_Guard (Host_Invoke_##name, &a); \
	}

/* host.c:746-751, :1005 -- world/model/sky/sound teardown and the CSQC world. */
HOST_GUARD_VOID (Mod_ClearAll)
HOST_GUARD_VOID (Sky_ClearAll)
HOST_GUARD_VOID (S_ClearAll)
HOST_GUARD_VOID (SV_ClearWorld)

/* host.c:858-898 -- the server frame entry points, all of which reach progs. */
HOST_GUARD_VOID (SV_ClearDatagram)
HOST_GUARD_VOID (SV_CheckForNewClients)
HOST_GUARD_VOID (SV_RunClients)
HOST_GUARD_VOID (SV_Physics)
HOST_GUARD_VOID (SV_SendClientMessages)

/* host.c:1110-1231 -- the _Host_Frame body's own calls. */
HOST_GUARD_VOID (Key_UpdateForDest)
HOST_GUARD_VOID (IN_UpdateInputMode)
HOST_GUARD_VOID (IN_Commands)
HOST_GUARD_VOID (Con_UpdateMouseState)
HOST_GUARD_VOID (Cbuf_Execute)
HOST_GUARD_VOID (NET_Poll)
HOST_GUARD_VOID (CL_AccumulateCmd)
HOST_GUARD_VOID (M_UpdateMouse)
HOST_GUARD_VOID (CL_SendCmd)
HOST_GUARD_VOID (CL_ReadFromServer)
HOST_GUARD_VOID (CL_RunParticles)
HOST_GUARD_VOID (CL_DecayLights)
HOST_GUARD_VOID (BGM_Update)
HOST_GUARD_VOID (CDAudio_Update)
HOST_GUARD_VOID (Harness_Frame)

/* host.c:676, :757 -- client teardown reached from the server shutdown path. */
HOST_GUARD_VOID (CL_Disconnect)
HOST_GUARD_VOID (CL_FreeState)

/* host.c:431 -- host_cmd.c's command registration. */
HOST_GUARD_VOID (Host_InitCommands)

/* host.c:1296-1404 -- Host_Init's one-shot subsystem bring-up. */
HOST_GUARD_VOID (Mem_Init)
HOST_GUARD_VOID (Tasks_Init)
HOST_GUARD_VOID (Cbuf_Init)
HOST_GUARD_VOID (Cmd_Init)
HOST_GUARD_VOID (Cvar_Init)
HOST_GUARD_VOID (COM_Init)
HOST_GUARD_VOID (COM_InitFilesystem)
HOST_GUARD_VOID (W_LoadWadFile)
HOST_GUARD_VOID (Key_Init)
HOST_GUARD_VOID (Con_Init)
HOST_GUARD_VOID (PR_Init)
HOST_GUARD_VOID (Mod_Init)
HOST_GUARD_VOID (NET_Init)
HOST_GUARD_VOID (SV_Init)
HOST_GUARD_VOID (V_Init)
HOST_GUARD_VOID (Chase_Init)
HOST_GUARD_VOID (M_Init)
HOST_GUARD_VOID (ExtraMaps_Init)
HOST_GUARD_VOID (M_CheckMods)
HOST_GUARD_VOID (Modlist_Init)
HOST_GUARD_VOID (DemoList_Init)
HOST_GUARD_VOID (SaveList_Init)
HOST_GUARD_VOID (VID_Init)
HOST_GUARD_VOID (IN_Init)
HOST_GUARD_VOID (TexMgr_Init)
HOST_GUARD_VOID (Draw_Init)
HOST_GUARD_VOID (SCR_Init)
HOST_GUARD_VOID (R_Init)
HOST_GUARD_VOID (S_Init)
HOST_GUARD_VOID (CDAudio_Init)
HOST_GUARD_VOID (BGM_Init)
HOST_GUARD_VOID (Sbar_Init)
HOST_GUARD_VOID (R_InitParticles)
HOST_GUARD_VOID (LOC_Init)
HOST_GUARD_VOID (Harness_Init)
HOST_GUARD_VOID (COM_WriteSelectedBaseDir)
HOST_GUARD_VOID (CL_Init)
/* host_glue.c:483-508 -- the two optional subsystems keep an unconditional
 * Host_Glue_* ABI so the Rust port links the same symbol set in every
 * configuration. PSET_SCRIPT is on (c_ref_prelude.h:544); PR_TRACE is not, and
 * host.c's own call sites vanish with it. */
#ifdef PSET_SCRIPT
HOST_GUARD_VOID (PScript_InitParticles)
#else
int Host_Glue_PScript_InitParticles (void)
{
	return 0; /* CTEST_GUARD_OK; stubs.c:1465 */
}
#endif
#ifdef PR_TRACE
HOST_GUARD_VOID (PR_TraceInit)
HOST_GUARD_VOID (PR_TraceShutdown)
#else
int Host_Glue_PR_TraceInit (void)
{
	return 0; /* CTEST_GUARD_OK; stubs.c:1465 */
}

int Host_Glue_PR_TraceShutdown (void)
{
	return 0; /* CTEST_GUARD_OK; stubs.c:1465 */
}
#endif

/* host.c:1427-1455 -- Host_Shutdown's teardown. */
HOST_GUARD_VOID (Harness_Shutdown)
HOST_GUARD_VOID (NET_Shutdown)
HOST_GUARD_VOID (History_Shutdown)
HOST_GUARD_VOID (ExtraMaps_ShutDown)
HOST_GUARD_VOID (BGM_Shutdown)
HOST_GUARD_VOID (CDAudio_Shutdown)
HOST_GUARD_VOID (S_Shutdown)
HOST_GUARD_VOID (IN_Shutdown)
HOST_GUARD_VOID (VID_Shutdown)
HOST_GUARD_VOID (Steam_Shutdown)
HOST_GUARD_VOID (LOG_Close)
HOST_GUARD_VOID (LOC_Shutdown)

/* Pointer- and int-operand seams, host_glue.c:524-532. */
HOST_GUARD_PTR (LOG_Init, quakeparms_t *)
HOST_GUARD_PTR (PR_ClearProgs, qcvm_t *)
HOST_GUARD_PTR (Key_WriteBindings, FILE *)
HOST_GUARD_PTR (Cvar_WriteVariables, FILE *)
HOST_GUARD_PTR (SVFTE_DestroyFrames, client_t *)
HOST_GUARD_PTR (NET_Close, struct qsocket_s *)
HOST_GUARD_INT (SCR_UpdateScreen, qboolean)
HOST_GUARD_INT (PR_ExecuteProgram, func_t)

/* Value-returning seams, host_glue.c:538-715. The result is written through an
 * out-parameter so the int return stays the guard status; on a caught raise the
 * out-parameter is left untouched and the Rust core returns before observing
 * it. */

typedef struct
{
	const char	*s;
	const char	*t;
	void		*p;
	void		*q;
	double		 d;
	int			 i;
	unsigned int u;
	size_t		 n;
	int			*out_i;
	void	   **out_p;
} host_arg_t;

static void Host_InvokeCvarSetQuick (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	Cvar_SetQuick ((cvar_t *)a->p, a->s);
}
int Host_Glue_CvarSetQuick (void *var, const char *value)
{
	host_arg_t a = {0};
	a.p = var;
	a.s = value;
	return Host_Guard (Host_InvokeCvarSetQuick, &a);
}

static void Host_InvokeCbufAddText (void *p)
{
	Cbuf_AddText (((host_arg_t *)p)->s);
}
int Host_Glue_CbufAddText (const char *text)
{
	host_arg_t a = {0};
	a.s = text;
	return Host_Guard (Host_InvokeCbufAddText, &a);
}

static void Host_InvokeCbufInsertText (void *p)
{
	Cbuf_InsertText (((host_arg_t *)p)->s);
}
int Host_Glue_CbufInsertText (const char *text)
{
	host_arg_t a = {0};
	a.s = text;
	return Host_Guard (Host_InvokeCbufInsertText, &a);
}

/* host.c:959-961 -- PR_LoadProgs always runs with fatal = false here. */
static void Host_InvokeLoadProgs (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = PR_LoadProgs (a->s, false, a->u, (const builtin_t *)a->p, a->n);
}
int Host_Glue_PRLoadProgs (const char *filename, unsigned int needcrc, const void *builtins, size_t numbuiltins, int *out)
{
	host_arg_t a = {0};
	a.s = filename;
	a.u = needcrc;
	a.p = (void *)builtins;
	a.n = numbuiltins;
	a.out_i = out;
	return Host_Guard (Host_InvokeLoadProgs, &a);
}

static void Host_InvokeSetEngineString (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = PR_SetEngineString (a->s);
}
int Host_Glue_PRSetEngineString (const char *s, int *out)
{
	host_arg_t a = {0};
	a.s = s;
	a.out_i = out;
	return Host_Guard (Host_InvokeSetEngineString, &a);
}

static void Host_InvokeEdictNum (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_p = EDICT_NUM (a->i);
}
int Host_Glue_EdictNum (int n, void **out)
{
	host_arg_t a = {0};
	a.i = n;
	a.out_p = out;
	return Host_Guard (Host_InvokeEdictNum, &a);
}

static void Host_InvokeEdictToProg (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = EDICT_TO_PROG ((edict_t *)a->p);
}
int Host_Glue_EdictToProg (void *e, int *out)
{
	host_arg_t a = {0};
	a.p = e;
	a.out_i = out;
	return Host_Guard (Host_InvokeEdictToProg, &a);
}

static void Host_InvokeNetCanSendMessage (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = NET_CanSendMessage ((struct qsocket_s *)a->p);
}
int Host_Glue_NetCanSendMessage (void *sock, int *out)
{
	host_arg_t a = {0};
	a.p = sock;
	a.out_i = out;
	return Host_Guard (Host_InvokeNetCanSendMessage, &a);
}

static void Host_InvokeNetSendMessage (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = NET_SendMessage ((struct qsocket_s *)a->p, (sizebuf_t *)a->q);
}
int Host_Glue_NetSendMessage (void *sock, void *data, int *out)
{
	host_arg_t a = {0};
	a.p = sock;
	a.q = data;
	a.out_i = out;
	return Host_Guard (Host_InvokeNetSendMessage, &a);
}

static void Host_InvokeNetGetMessage (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_i = NET_GetMessage ((struct qsocket_s *)a->p);
}
int Host_Glue_NetGetMessage (void *sock, int *out)
{
	host_arg_t a = {0};
	a.p = sock;
	a.out_i = out;
	return Host_Guard (Host_InvokeNetGetMessage, &a);
}

static void Host_InvokeComLoadFile (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	*a->out_p = COM_LoadFile (a->s, NULL);
}
int Host_Glue_ComLoadFile (const char *path, void **out)
{
	host_arg_t a = {0};
	a.s = path;
	a.out_p = out;
	return Host_Guard (Host_InvokeComLoadFile, &a);
}

/* host.c:1212, :1216 -- S_Update takes four vec3_t. */
typedef struct
{
	const float *o, *f, *r, *u;
} host_supdate_arg_t;

static void Host_InvokeSUpdate (void *p)
{
	host_supdate_arg_t *a = (host_supdate_arg_t *)p;
	S_Update ((float *)a->o, (float *)a->f, (float *)a->r, (float *)a->u);
}
int Host_Glue_SUpdate (const float *origin, const float *forward, const float *right, const float *up)
{
	host_supdate_arg_t a;
	a.o = origin;
	a.f = forward;
	a.r = right;
	a.u = up;
	return Host_Guard (Host_InvokeSUpdate, &a);
}

/* Batched, guarded sizebuf writers, host_glue.c:717-772. Ops replay in
 * insertion order, so the byte stream is identical for any batch size. The
 * writers are quake-capi's cores rather than MSG_Write* -- see the header
 * comment. */

typedef struct
{
	int			kind;
	int			i;
	const void *p;
} host_write_t;

typedef struct
{
	sizebuf_t		   *sb;
	const host_write_t *ops;
	int					count;
} host_writebatch_arg_t;

static void Host_InvokeWriteBatch (void *p)
{
	host_writebatch_arg_t *a = (host_writebatch_arg_t *)p;
	int					   k;

	for (k = 0; k < a->count; k++)
	{
		const host_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			Host_Reraise (quake_rs_msg_write_byte (a->sb, op->i));
			break;
		case 1:
			Host_Reraise (quake_rs_msg_write_short (a->sb, op->i));
			break;
		case 2:
			Host_Reraise (quake_rs_msg_write_string (a->sb, (const char *)op->p));
			break;
		default:
			Sys_Error ("Host_InvokeWriteBatch: unknown op %i", op->kind);
			break;
		}
	}
}

int Host_Glue_WriteBatch (void *sb, const host_write_t *ops, int count)
{
	host_writebatch_arg_t arg;
	arg.sb = (sizebuf_t *)sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (Host_InvokeWriteBatch, &arg);
}

/* host.c:702-710 -- the disconnect broadcast writes into a stack-local
 * sizebuf_t backed by a four-byte array; keeping the whole step in C preserves
 * that storage exactly. */
static void Host_InvokeBroadcastDisconnect (void *p)
{
	host_arg_t *a = (host_arg_t *)p;
	sizebuf_t	buf;
	byte		message[4];

	buf.data = message;
	buf.maxsize = 4;
	buf.cursize = 0;
	Host_Reraise (quake_rs_msg_write_byte (&buf, svc_disconnect));
	*a->out_i = NET_SendToAll (&buf, 5.0);
}
int Host_Glue_BroadcastDisconnect (int *out_count)
{
	host_arg_t a = {0};
	a.out_i = out_count;
	return Host_Guard (Host_InvokeBroadcastDisconnect, &a);
}

/*
 * 3. Entry points. host_glue.c:802-933 defines fifteen of these; fourteen
 * already have plain definitions in stubs.c, so only the one the link probe
 * reported is here. host.c's own Host_Version_f is c_ref_Host_Version_f inside
 * host_ref.c, and the Rust port registers this plain wrapper with
 * Cmd_AddCommand so a raise inside it unwinds through a pure C frame.
 */

void Host_Version_f (void)
{
	Host_Reraise (quake_rs_host_version_f ());
}

/*
 * host_glue.c:912-918 -- _Host_Frame's setjmp shell, expressed as a discarded
 * Host_Guard because this link has no host_abortserver (see the header). A
 * raise inside quake_rs_host_frame_core is caught here and swallowed, which is
 * the early `return` host.c took.
 */
static void Host_InvokeFrameCore (void *p)
{
	Host_Reraise (quake_rs_host_frame_core (*(double *)p));
}

void Host_Glue_FrameInner (double time)
{
	(void)Host_Guard (Host_InvokeFrameCore, &time);
}

/*
 * 4. T8.2 differential drivers -- the Rust half of tests/host_differential.rs.
 *
 * host_ref.c renames host.c's file-scope objects to c_ref_*, and c_ref_prelude.h
 * renames sv/svs/cl/cls the same way, so the oracle's timing state, cvars,
 * client array, host_client, host_initialized and host_parms are all distinct
 * objects from the ones quake-capi/src/host.rs reads. That separation IS the
 * differential: the two implementations hold independent state in one process,
 * so each subject can be driven twice from identical inputs and the outputs
 * compared. Every accessor below is the plain-state twin of a ctest_host_*
 * accessor at the bottom of host_ref.c.
 *
 * THE #undefs ARE SCOPED TO THIS SECTION, which defines no Host_Glue_* seam.
 * `sv`, `svs`, `cls` and `com_gamedir` reach the prelude's c_ref_* spelling
 * everywhere above; here they must reach the plain objects quake-capi's
 * sv_main.rs/cl_main.rs/fs.rs define as #[no_mangle] statics, which is the
 * shape sv_main_ref.c:104-113 established. Nothing else is undefined -- in
 * particular Cvar_SetQuick keeps its c_ref_ spelling, because both sides share
 * one cvar.c.
 */

#undef sv
#undef svs
#undef cls
#undef com_gamedir

extern server_t		   sv;
extern server_static_t svs;
extern client_static_t cls;
extern char			   com_gamedir[MAX_OSPATH];

/* The plain twins of host.c's timing/gate objects. `oldrealtime` and
 * `host_rawframetime` are this file's own (section 1); `realtime`
 * (c_ref_prelude.h:1220), `host_frametime` (:1025), `host_initialized` (:376),
 * `host_parms` (:782), `host_client` (server.h) and `deathmatch` (server.h) are
 * declared already and defined by stubs.c or host_ref.c. */

/* quake-capi/src/host.rs. qboolean is `bool` (q_types.h:122), one byte -- an
 * `int` return type here would be an ABI trap. The Raise-returning exports
 * yield a Host_Guard status: 0 = HOST_GUARD_OK, 1 = Host_Error/Host_EndGame,
 * 2 = screen_error. */
extern qboolean quake_rs_host_filter_time (float time);
extern int		quake_rs_host_sv_client_printf (const char *string);
extern int		quake_rs_host_sv_broadcast_printf (const char *string);
extern int		quake_rs_host_client_commands (const char *string);
extern int		quake_rs_host_callback_notify (cvar_t *var);
extern int		quake_rs_host_find_max_clients (void);
extern int		quake_rs_host_write_configuration (void);

/* ---- Host_FilterTime (host.rs:1089) ---- */

void ctest_host_rs_reset_time (double realtime_in, double oldrealtime_in)
{
	realtime = realtime_in;
	oldrealtime = oldrealtime_in;
	host_frametime = 0.0;
	host_rawframetime = 0.0;
}

void ctest_host_rs_set_maxfps (float value)
{
	host_maxfps.value = value;
}

void ctest_host_rs_set_timescale (float value)
{
	host_timescale.value = value;
}

void ctest_host_rs_set_framerate (float value)
{
	host_framerate.value = value;
}

void ctest_host_rs_set_demo (int demoplayback, float demospeed, int timedemo)
{
	cls.demoplayback = demoplayback ? true : false;
	cls.demospeed = demospeed;
	cls.timedemo = timedemo ? true : false;
}

int ctest_host_rs_filter_time (float time)
{
	return quake_rs_host_filter_time (time) ? 1 : 0;
}

double ctest_host_rs_get_realtime (void)
{
	return realtime;
}

double ctest_host_rs_get_oldrealtime (void)
{
	return oldrealtime;
}

double ctest_host_rs_get_frametime (void)
{
	return host_frametime;
}

double ctest_host_rs_get_rawframetime (void)
{
	return host_rawframetime;
}

/* ---- the three variadic senders (host.rs:768, :804, :812) ----
 * The same fixture shape host_ref.c:473-495 publishes into c_ref_svs, published
 * into the plain svs instead. The va_list half stays in Quake/host_glue.c, so
 * each core takes the already-formatted string -- which is exactly what
 * host_ref.c's drivers hand the oracle through their "%s" wrapper. */
#define CTEST_HOST_RS_CLIENTS 4
#define CTEST_HOST_RS_MSGMAX  1024

static client_t ctest_host_rs_clients[CTEST_HOST_RS_CLIENTS];
static byte		ctest_host_rs_msgbuf[CTEST_HOST_RS_CLIENTS][CTEST_HOST_RS_MSGMAX];

void ctest_host_rs_reset_clients (int maxclients)
{
	int i;

	memset (ctest_host_rs_clients, 0, sizeof (ctest_host_rs_clients));
	memset (ctest_host_rs_msgbuf, 0, sizeof (ctest_host_rs_msgbuf));
	for (i = 0; i < CTEST_HOST_RS_CLIENTS; i++)
	{
		ctest_host_rs_clients[i].message.data = ctest_host_rs_msgbuf[i];
		ctest_host_rs_clients[i].message.maxsize = CTEST_HOST_RS_MSGMAX;
		ctest_host_rs_clients[i].active = true;
		ctest_host_rs_clients[i].spawned = true;
		q_snprintf (ctest_host_rs_clients[i].name, sizeof (ctest_host_rs_clients[i].name), "player%i", i);
	}

	svs.clients = ctest_host_rs_clients;
	svs.maxclients = maxclients;
	svs.maxclientslimit = CTEST_HOST_RS_CLIENTS;
	host_client = &ctest_host_rs_clients[0];
}

void ctest_host_rs_set_client_state (int index, int active, int spawned)
{
	ctest_host_rs_clients[index].active = active ? true : false;
	ctest_host_rs_clients[index].spawned = spawned ? true : false;
}

int ctest_host_rs_client_msg_len (int index)
{
	return ctest_host_rs_clients[index].message.cursize;
}

int ctest_host_rs_client_msg_byte (int index, int offset)
{
	return ctest_host_rs_msgbuf[index][offset];
}

void ctest_host_rs_set_host_client (int index)
{
	host_client = ctest_host_rs_clients + index;
}

int ctest_host_rs_sv_client_printf (const char *text)
{
	return quake_rs_host_sv_client_printf (text);
}

int ctest_host_rs_sv_broadcast_printf (const char *text)
{
	return quake_rs_host_sv_broadcast_printf (text);
}

int ctest_host_rs_client_commands (const char *text)
{
	return quake_rs_host_client_commands (text);
}

/* ---- Host_Callback_Notify (host.rs:623) ---- */

void ctest_host_rs_set_sv_active (int value)
{
	sv.active = value ? true : false;
}

int ctest_host_rs_callback_notify (cvar_t *var)
{
	return quake_rs_host_callback_notify (var);
}

/* ---- Host_FindMaxClients (host.rs:520) ----
 * com_argc/com_argv, COM_CheckParm, Mem_Alloc and the `deathmatch` cvar are NOT
 * renamed by the prelude and `deathmatch` is deliberately left plain by
 * host_ref.c:60-67, so both sides read one command line and write one cvar. The
 * setters below exist so a test can plant a sentinel in each output before the
 * Rust run: without that, a port that wrote nothing would inherit the oracle's
 * answer and the comparison would be vacuous. */

void ctest_host_rs_set_maxclients (int value)
{
	svs.maxclients = value;
}

int ctest_host_rs_get_maxclients (void)
{
	return svs.maxclients;
}

void ctest_host_rs_set_cls_state (int value)
{
	cls.state = (cactive_t)value;
}

int ctest_host_rs_get_cls_state (void)
{
	return (int)cls.state;
}

void ctest_host_rs_set_deathmatch (const char *value)
{
	Cvar_SetQuick (&deathmatch, value);
}

int ctest_host_rs_find_max_clients (void)
{
	return quake_rs_host_find_max_clients ();
}

/* ---- Host_WriteConfiguration (host.rs:722) ----
 * host_initialized, host_parms, isDedicated and Key_WriteBindings (the counting
 * double at host_ref.c:201) are unrenamed, so both sides share them; the setters
 * below exist so the driver never has to reach across into the oracle's half.
 * stubs.c:759 hands out one quakeparms_t whose address the filesystem compares
 * by identity, so errstate is poked in place rather than the pointer repointed.
 *
 * The output path is NOT shared. The oracle reaches c_ref_COM_FOpenPrefFile and
 * c_ref_com_gamedir (common_fs.c); the port reaches quake-capi's own
 * COM_FOpenPrefFile (fs.rs:801), which reads the plain com_gamedir. Two gamedirs
 * means two config files, so an empty-bodied port writes nothing and cannot be
 * mistaken for the oracle's file. */

void ctest_host_rs_set_gamedir (const char *dir)
{
	q_strlcpy (com_gamedir, dir, sizeof (com_gamedir));
}

void ctest_host_rs_set_initialized (int value)
{
	host_initialized = value ? true : false;
}

void ctest_host_rs_set_parms (int errstate)
{
	host_parms->errstate = errstate;
}

int ctest_host_rs_write_configuration (void)
{
	return quake_rs_host_write_configuration ();
}
