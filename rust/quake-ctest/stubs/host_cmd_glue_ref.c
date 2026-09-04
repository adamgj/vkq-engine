/* Phase 7 M8 T8.3: the ctest-link mirror of Quake/host_cmd_glue.c's
 * glue-owned half.
 *
 * Quake/host_cmd_glue.c is gated on the Meson -Duse_rust_host flip and is not
 * in build.rs's C_SOURCES, while Quake/host_cmd.c is composed into
 * stubs/host_cmd_ref.c under a per-TU c_ref_* rename block. So every object
 * and seam host_cmd_glue.c owns has no definition under its plain name in
 * this link, and quake-capi's `host` feature -- which is on for this crate --
 * references all of them. A link probe (`cargo test -p quake-ctest --no-run`)
 * named exactly sixty such symbols; this file supplies that set, and nothing
 * else.
 *
 * Four jobs, mirroring stubs/host_glue_ref.c:
 *
 *  1. Define the plain (Rust-reading) twins of the data host_cmd_glue.c owns:
 *     the five filelist heads plus extralevels_sorted and the engine version
 *     line. current_skill (stubs.c:6912) and noclip_anglehack (stubs.c:7248)
 *     already have plain definitions and are NOT redefined here.
 *
 *  2. Transcribe host_cmd_glue.c's forty-nine HostCmd_Glue_* seams, keeping
 *     the Host_Guard (invoke_fn, &arg) trampoline shape and the four
 *     HOSTCMD_GUARD_* macros verbatim. Host_Guard, Host_Reraise, Host_Error
 *     and Host_EndGame are stubs.c's, with the harness's CTEST_GUARD_*
 *     result set rather than the real HOST_GUARD_* one (stubs.c:1434-1467
 *     documents the departure); they are not redefined here.
 *
 *  3. Transcribe section 3, HostCmd_Raise. It is not part of the flip half:
 *     the two negative raise codes it re-issues (savegame version, first
 *     token brace) are observable behaviour of the port, and the loadgame
 *     differential pins the exact Host_Error text.
 *
 *  4. Section 4 (this file's own) drives the Rust port from the test binary:
 *     ctest_hostcmd_rs_* accessors over the plain, Rust-read state, paired
 *     one for one with host_cmd_ref.c's ctest_hostcmd_* accessors over the
 *     c_ref_* state.
 *
 * CALLEE SPELLING, as host_glue_ref.c:36-48 states it. Every callee in
 * sections 1-3 is spelled exactly the way Quake/host_cmd.c spells it, and no
 * #undef appears anywhere above section 4. c_ref_prelude.h is force-included
 * here and into host_cmd_ref.c alike, so its rename macros rewrite this
 * file's call sites and the oracle host_cmd.c's call sites identically: both
 * sides of the differential land on the same function by construction rather
 * than by audit.
 *
 * SV_ClientPrintf / SV_BroadcastPrintf ARE RENAMED PER-TU, for the reason
 * host_cmd_ref.c:81-96 gives: the prelude does not rename them and this link
 * has no plain definition (host.c's copies are host_ref.c's, behind its own
 * per-TU rename). The two #defines below are host_cmd_ref.c's, verbatim, so
 * this file's two SV_ClientPrintf seams and the oracle's call sites reach the
 * same c_ref_SV_ClientPrintf. That function writes host_ref.c's c_ref_ state,
 * which is a fixture-plumbing seam rather than a behavioural one; no test in
 * host_cmd_differential.rs reads it today.
 *
 * MSG_Write* IS THE ONE EXCEPTION to the callee-spelling rule, and it follows
 * host_glue_ref.c:50-59. In the shipping -Duse_rust build the writers are
 * quake-capi's quake_rs_msg_write_* status cores wrapped by
 * Quake/net_msg_glue.c (ADR-009: SZ_GetSpace can Host_Error), so that is what
 * host_cmd_glue.c's MSG_WriteByte resolves to there. HostCmd_InvokeWriteBatch
 * drives those cores directly and Host_Reraises a non-zero status from inside
 * the guard, exactly as net_msg_glue.c would; the oracle keeps net_msg.c's
 * c_ref_MSG_Write*, which is the comparison the differential wants.
 *
 * THE FLIP HALF IS DELIBERATELY ABSENT. host_cmd_glue.c's sections 4a/4b/4c
 * -- the fifteen C-visible wrappers, the forty-one static command thunks and
 * Host_InitCommands -- exist to hand the C engine a Rust implementation under
 * the old name. Nothing in this link calls them (the oracle supplies its own
 * c_ref_ copies), and defining Host_Quit_f or ExtraMaps_Init here would
 * collide with host_cmd_ref.c's. host_glue_ref.c draws the same boundary.
 * SaveList_Rebuild and Host_Reconnect_f are the two exceptions: two seams in
 * section 2b call them by name, so section 4 defines them as the same
 * one-line forwarders host_cmd_glue.c:765 and :864 use.
 */

#include <stddef.h>
#include <stdlib.h>
#include <string.h>

/* stubs.c's raise machinery. The real engine declares these in host.h, which
 * c_ref_prelude.h does not pull in (host_glue_ref.c:84-87 does the same). */
extern int	Host_Guard (void (*fn) (void *), void *arg);
extern void Host_Reraise (int guard_result);

/* quake-capi/src/host_cmd.rs's status cores and detail accessor.
 * host_cmd_glue.c gets these prototypes from the generated quake_rs.h, which
 * this link has no counterpart for. Only the ones this file actually calls
 * are declared. */
extern int quake_rs_hostcmd_raise_detail (void);
extern void quake_rs_hostcmd_save_list_rebuild (void);
extern int	quake_rs_hostcmd_reconnect_f (void);
extern void quake_rs_hostcmd_savegame_comment (char *text);
extern int	quake_rs_hostcmd_savegame_f (void);
extern int	quake_rs_hostcmd_loadgame_f (void);

/* quake-capi/src/net.rs, as host_glue_ref.c:96-99 declares its subset. */
extern int quake_rs_msg_write_byte (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_short (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_long (sizebuf_t *sb, int v);
extern int quake_rs_msg_write_float (sizebuf_t *sb, float v);
extern int quake_rs_msg_write_string (sizebuf_t *sb, const char *s);
extern int quake_rs_msg_write_angle (sizebuf_t *sb, float f, unsigned int flags);

/* host_cmd_ref.c:95-96, verbatim: see SV_ClientPrintf above. */
#define SV_ClientPrintf	   c_ref_SV_ClientPrintf
#define SV_BroadcastPrintf c_ref_SV_BroadcastPrintf

void SV_ClientPrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);
void SV_BroadcastPrintf (const char *fmt, ...) FUNC_PRINTF (1, 2);

// ---------------------------------------------------------------------------
// 1. C-visible storage.
//
// Quake/host_cmd_glue.c:49-80, restricted to the objects that have no other
// definition in this link. current_skill (stubs.c:6912) and noclip_anglehack
// (stubs.c:7248) already have plain definitions and are deliberately omitted;
// defining either here would be a duplicate symbol. Mod_Print is omitted too:
// only Host_InitCommands (the flip half, not transcribed) names it.

filelist_item_t	 *extralevels;		  // host_cmd.c:193
filelist_item_t **extralevels_sorted; // host_cmd.c:194
filelist_item_t	 *modlist;			  // host_cmd.c:567
filelist_item_t	 *demolist;			  // host_cmd.c:815
filelist_item_t	 *savelist;			  // host_cmd.c:837

/* host_cmd.c:931 -- print_fn ("version: " ENGINE_NAME_AND_VER "\n"). Kept as
 * C data rather than a Rust literal because ENGINE_NAME_AND_VER
 * (quakever.h:59-61) expands a build-date macro that has no fixed Rust
 * spelling. Used verbatim by the port: it carries no format specifiers. */
const char *const HostCmd_EngineVersionLine = "version: " ENGINE_NAME_AND_VER "\n";

/* host_cmd.c:1510. File-local there, so it is redefined here for the
 * HOSTCMD_RAISE_SAVEGAME_VERSION re-issue below. */
#define SAVEGAME_VERSION 5

// ---------------------------------------------------------------------------
// 2a. The map-description parsing thread and its two atomics
// (host_cmd.c:197-198, :377, :398-406, :460-461, :473).

static qthread_t	  *extralevels_parsing_thread;
static atomic_uint32_t extralevels_cancel_parsing;

unsigned int HostCmd_Glue_AtomicLoadU32 (void *atomic)
{
	return Atomic_LoadUInt32 ((atomic_uint32_t *)atomic);
}

void HostCmd_Glue_AtomicStoreU32 (void *atomic, unsigned int desired)
{
	Atomic_StoreUInt32 ((atomic_uint32_t *)atomic, desired);
}

void *HostCmd_Glue_AtomicLoadPtr (void *atomic)
{
	return Atomic_LoadPtr ((atomic_ptr_t *)atomic);
}

void HostCmd_Glue_AtomicStorePtr (void *atomic, void *desired)
{
	Atomic_StorePtr ((atomic_ptr_t *)atomic, desired);
}

/* host_cmd.c:461. Only the QThread_Create half: the port issues the
 * cancel-flag clear at :460 through HostCmd_Glue_SetCancelParsing, keeping
 * the two writes in the original order. */
void HostCmd_Glue_StartParsingThread (qthread_func_t func)
{
	extralevels_parsing_thread = QThread_Create (func, "Map parser", NULL);
}

/* host_cmd.c:398-406 -- ExtraMaps_WaitForParsingThread, verbatim. */
void HostCmd_Glue_WaitForParsingThread (void)
{
	if (extralevels_parsing_thread)
	{
		QThread_Wait (extralevels_parsing_thread);
		extralevels_parsing_thread = NULL;
		Atomic_StoreUInt32 (&extralevels_cancel_parsing, 0);
	}
}

void HostCmd_Glue_SetCancelParsing (unsigned int value)
{
	Atomic_StoreUInt32 (&extralevels_cancel_parsing, value);
}

unsigned int HostCmd_Glue_GetCancelParsing (void)
{
	return Atomic_LoadUInt32 (&extralevels_cancel_parsing);
}

// ---------------------------------------------------------------------------
// 2b. Guarded seams (ADR-009 rule 3). Each is enumerated by name rather than
// reached through a generic function-pointer guard: naming them keeps the
// Pattern A symbol audit meaningful and lets the compiler type-check every
// call. The macros collapse only the boilerplate, not the enumeration.

typedef struct
{
	const char	 *s;
	const char	 *t;
	void		 *p;
	void		 *q;
	void		**outp;
	const char	**outs;
	int			 *outi;
	unsigned int *outu;
	int			  i;
	unsigned int  u;
	float		  f;
	qboolean	  b;
} hostcmd_arg_t;

#define HOSTCMD_GUARD_VOID(name, call)                  \
	static void HostCmd_Invoke##name (void *p)          \
	{                                                   \
		(void)p;                                        \
		call;                                           \
	}                                                   \
	int HostCmd_Glue_##name (void)                      \
	{                                                   \
		return Host_Guard (HostCmd_Invoke##name, NULL); \
	}

#define HOSTCMD_GUARD_STR(name, call)                   \
	static void HostCmd_Invoke##name (void *p)          \
	{                                                   \
		hostcmd_arg_t *a = (hostcmd_arg_t *)p;          \
		call;                                           \
	}                                                   \
	int HostCmd_Glue_##name (const char *s)             \
	{                                                   \
		hostcmd_arg_t arg = {0};                        \
		arg.s = s;                                      \
		return Host_Guard (HostCmd_Invoke##name, &arg); \
	}

#define HOSTCMD_GUARD_PTR(name, call)                   \
	static void HostCmd_Invoke##name (void *p)          \
	{                                                   \
		hostcmd_arg_t *a = (hostcmd_arg_t *)p;          \
		call;                                           \
	}                                                   \
	int HostCmd_Glue_##name (void *ptr)                 \
	{                                                   \
		hostcmd_arg_t arg = {0};                        \
		arg.p = ptr;                                    \
		return Host_Guard (HostCmd_Invoke##name, &arg); \
	}

#define HOSTCMD_GUARD_BOOL(name, call)                  \
	static void HostCmd_Invoke##name (void *p)          \
	{                                                   \
		hostcmd_arg_t *a = (hostcmd_arg_t *)p;          \
		call;                                           \
	}                                                   \
	int HostCmd_Glue_##name (qboolean flag)             \
	{                                                   \
		hostcmd_arg_t arg = {0};                        \
		arg.b = flag;                                   \
		return Host_Guard (HostCmd_Invoke##name, &arg); \
	}

/* Defined further down as the C-visible entry points the port drives; the
 * guards below wrap those wrappers, so a Host_Reraise issued from inside one
 * of them is caught here and returned as a status like any other raise. */
void		SaveList_Rebuild (void);
static void Host_Reconnect_f (void);

/* menu.h:76. menu.h is not in this slice's include set. */
void M_Menu_Quit_f (void);

// -- chunk A (host_cmd.c:24-898) --------------------------------------------

/* host_cmd.c:56 -- M_Menu_Quit_f (), which reaches Cbuf_AddText. */
HOSTCMD_GUARD_VOID (M_Menu_Quit_f, M_Menu_Quit_f ())
/* host_cmd.c:59. */
HOSTCMD_GUARD_VOID (CL_Disconnect, CL_Disconnect ())
/* host_cmd.c:62 -- Sys_Quit () runs Host_Shutdown before exit (0). */
HOSTCMD_GUARD_VOID (Sys_Quit, Sys_Quit ())

/* host_cmd.c:60 -- Host_ShutdownServer (false). qboolean crosses as the int
 * the HOST_GUARD_INT shape uses (host_glue.c:531 precedent); the seam narrows
 * it back. */
static void HostCmd_InvokeHost_ShutdownServer (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Host_ShutdownServer (a->i != 0);
}

int HostCmd_Glue_Host_ShutdownServer (int crash)
{
	hostcmd_arg_t arg = {0};

	arg.i = crash;
	return Host_Guard (HostCmd_InvokeHost_ShutdownServer, &arg);
}

/* host_cmd.c:795 -- COM_LoadFile ("mapdb.json", &path_id). Both results leave
 * through out-parameters so the int return stays the guard status
 * (host_glue.c:683 precedent). */
static void HostCmd_InvokeComLoadFile (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outp = COM_LoadFile (a->s, a->outu);
}

int HostCmd_Glue_ComLoadFile (const char *path, unsigned int *path_id, void **out)
{
	hostcmd_arg_t arg = {0};

	arg.s = path;
	arg.outu = path_id;
	arg.outp = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeComLoadFile, &arg);
}

// -- chunk B (host_cmd.c:899-1509) ------------------------------------------

/* host_cmd.c:1341 -- Cmd_ExecuteString ("connect local", src_command) in
 * Host_Map_f. Cmd_ExecuteString cannot itself longjmp, but the handler it
 * dispatches to transitively can. The qboolean result is discarded, matching
 * host_cmd.c's own ignored return value. */
static void HostCmd_InvokeCmdExecuteString (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Cmd_ExecuteString (a->s, (cmd_source_t)a->u);
}

int HostCmd_Glue_CmdExecuteString (const char *text, unsigned int src)
{
	hostcmd_arg_t arg = {0};

	arg.s = text;
	arg.u = src;
	return Host_Guard (HostCmd_InvokeCmdExecuteString, &arg);
}

/* host_cmd.c:1497 (Host_Connect_f) and :2148 (Host_Reconnect_f). */
HOSTCMD_GUARD_STR (CLEstablishConnection, CL_EstablishConnection (a->s))

/* host_cmd.c:1413 -- Host_Error ("cannot find map %s", level). */
HOSTCMD_GUARD_STR (ErrorCannotFindMap, Host_Error ("cannot find map %s", a->s))
/* host_cmd.c:1426 -- Host_Error ("cannot run map %s", level). */
HOSTCMD_GUARD_STR (ErrorCannotRunMap, Host_Error ("cannot run map %s", a->s))
/* host_cmd.c:1458 -- Host_Error ("cannot restart map %s", mapname). */
HOSTCMD_GUARD_STR (ErrorCannotRestartMap, Host_Error ("cannot restart map %s", a->s))

// -- chunk C (host_cmd.c:1510-2156) -----------------------------------------

/* EDICT_NUM (host_cmd.c:1652, :1779, :2075, :2112). The macro Host_Errors on
 * an out-of-range index (pr_edict_arena.c). */
static void HostCmd_InvokeEdictNum (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outp = EDICT_NUM (a->i);
}

int HostCmd_Glue_EdictNum (int n, void **out)
{
	hostcmd_arg_t arg = {0};

	arg.i = n;
	arg.outp = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeEdictNum, &arg);
}

/* ED_WriteGlobals (f) -- host_cmd.c:1649. */
HOSTCMD_GUARD_PTR (EDWriteGlobals, ED_WriteGlobals ((FILE *)a->p))
/* ED_CheckFreeList () -- host_cmd.c:1703. */
HOSTCMD_GUARD_VOID (EDCheckFreeList, ED_CheckFreeList ())
/* SaveList_Rebuild () -- host_cmd.c:1708; the wrapper defined below. */
HOSTCMD_GUARD_VOID (SaveListRebuild, SaveList_Rebuild ())
/* ED_Free (EDICT_NUM (i)) -- host_cmd.c:2112. */
HOSTCMD_GUARD_PTR (EDFree, ED_Free ((edict_t *)a->p))
/* ED_RebuildFreeList (true) -- host_cmd.c:2119. */
HOSTCMD_GUARD_BOOL (EDRebuildFreeList, ED_RebuildFreeList (a->b))
/* CL_Disconnect_f () -- host_cmd.c:1914, :3123. */
HOSTCMD_GUARD_VOID (CLDisconnect_f, CL_Disconnect_f ())
/* CL_Stop_f () -- host_cmd.c:1916. */
HOSTCMD_GUARD_VOID (CLStop_f, CL_Stop_f ())
/* SV_SpawnServer (mapname) -- host_cmd.c:1921. */
HOSTCMD_GUARD_STR (SVSpawnServer, SV_SpawnServer (a->s))
/* CL_Resume_Record (fastload) -- host_cmd.c:1941. */
HOSTCMD_GUARD_BOOL (CLResumeRecord, CL_Resume_Record (a->b))
/* Sky_LoadSkyBox (com_token) -- host_cmd.c:2049; guarded conservatively, it
 * reaches the image loaders. */
HOSTCMD_GUARD_STR (SkyLoadSkyBox, Sky_LoadSkyBox (a->s))
/* Host_Reconnect_f () -- host_cmd.c:2149; the thunk defined below. */
HOSTCMD_GUARD_VOID (HostReconnect_f, Host_Reconnect_f ())

/* ED_Write (f, ed) -- host_cmd.c:1652. */
static void HostCmd_InvokeEDWrite (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	ED_Write ((FILE *)a->p, (edict_t *)a->q);
}

int HostCmd_Glue_EDWrite (void *f, void *ed)
{
	hostcmd_arg_t arg = {0};

	arg.p = f;
	arg.q = ed;
	return Host_Guard (HostCmd_InvokeEDWrite, &arg);
}

/* One buffered MSG_Write* call. Mirrored in Rust as HostCmdWriteOp
 * (rust/quake-c-sys/src/host_cmd.rs); the kind values are the switch below. */
typedef struct
{
	int			 kind;
	int			 i;
	float		 f;
	unsigned int u;
	const void	*p;
} hostcmd_write_t;

typedef struct
{
	sizebuf_t			  *sb;
	const hostcmd_write_t *ops;
	int					   count;
} hostcmd_writebatch_arg_t;

/* Replays a run of MSG_Write* calls against one sizebuf_t inside a single
 * Host_Guard frame (host_cmd.c:1723-1789). Every writer reaches SZ_GetSpace,
 * which Host_Errors on overflow (net_msg.c:488). */
static void HostCmd_InvokeWriteBatch (void *p)
{
	hostcmd_writebatch_arg_t *a = (hostcmd_writebatch_arg_t *)p;
	int						  k;

	for (k = 0; k < a->count; k++)
	{
		const hostcmd_write_t *op = &a->ops[k];
		switch (op->kind)
		{
		case 0:
			Host_Reraise (quake_rs_msg_write_byte (a->sb, op->i));
			break;
		case 1:
			Host_Reraise (quake_rs_msg_write_short (a->sb, op->i));
			break;
		case 2:
			Host_Reraise (quake_rs_msg_write_long (a->sb, op->i));
			break;
		case 3:
			Host_Reraise (quake_rs_msg_write_float (a->sb, op->f));
			break;
		case 4:
			Host_Reraise (quake_rs_msg_write_string (a->sb, (const char *)op->p));
			break;
		case 5:
			Host_Reraise (quake_rs_msg_write_angle (a->sb, op->f, op->u));
			break;
		default:
			Sys_Error ("HostCmd_InvokeWriteBatch: unknown op %i", op->kind);
			break;
		}
	}
}

int HostCmd_Glue_WriteBatch (void *sb, const hostcmd_write_t *ops, int count)
{
	hostcmd_writebatch_arg_t arg;

	arg.sb = (sizebuf_t *)sb;
	arg.ops = ops;
	arg.count = count;
	return Host_Guard (HostCmd_InvokeWriteBatch, &arg);
}

/* SV_WriteClientdataToMessage (c, &c->message) -- host_cmd.c:1789. */
static void HostCmd_InvokeSVWriteClientdataToMessage (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	SV_WriteClientdataToMessage ((client_t *)a->p, (sizebuf_t *)a->q);
}

int HostCmd_Glue_SVWriteClientdataToMessage (void *client, void *sb)
{
	hostcmd_arg_t arg = {0};

	arg.p = client;
	arg.q = sb;
	return Host_Guard (HostCmd_InvokeSVWriteClientdataToMessage, &arg);
}

/* Cvar_SetValue ("skill", ...) -- host_cmd.c:1890; a cvar callback can
 * Host_Error. */
static void HostCmd_InvokeCvarSetValue (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Cvar_SetValue (a->s, a->f);
}

int HostCmd_Glue_CvarSetValue (const char *name, float value)
{
	hostcmd_arg_t arg = {0};

	arg.s = name;
	arg.f = value;
	return Host_Guard (HostCmd_InvokeCvarSetValue, &arg);
}

/* Cvar_SetValueQuick (&nomonsters, 0.f) -- host_cmd.c:1832. */
static void HostCmd_InvokeCvarSetValueQuick (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Cvar_SetValueQuick ((cvar_t *)a->p, a->f);
}

int HostCmd_Glue_CvarSetValueQuick (cvar_t *var, float value)
{
	hostcmd_arg_t arg = {0};

	arg.p = var;
	arg.f = value;
	return Host_Guard (HostCmd_InvokeCvarSetValueQuick, &arg);
}

/* Mod_ForName (name, crash) -- host_cmd.c:1991, :2907. crash=true Host_Errors
 * from gl_model.c:531; the model comes back in *out. */
static void HostCmd_InvokeModForName (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outp = Mod_ForName (a->s, a->b);
}

int HostCmd_Glue_ModForName (const char *name, qboolean crash, void **out)
{
	hostcmd_arg_t arg = {0};

	arg.s = name;
	arg.b = crash;
	arg.outp = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeModForName, &arg);
}

/* ED_ParseGlobals (data) -- host_cmd.c:2071; the advanced cursor in *out. */
static void HostCmd_InvokeEDParseGlobals (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outs = ED_ParseGlobals (a->s);
}

int HostCmd_Glue_EDParseGlobals (const char *data, const char **out)
{
	hostcmd_arg_t arg = {0};

	arg.s = data;
	arg.outs = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeEDParseGlobals, &arg);
}

/* ED_ParseEdict (data, ent) -- host_cmd.c:2097; cursor in *out. */
static void HostCmd_InvokeEDParseEdict (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outs = ED_ParseEdict (a->s, (edict_t *)a->p);
}

int HostCmd_Glue_EDParseEdict (const char *data, void *ent, const char **out)
{
	hostcmd_arg_t arg = {0};

	arg.s = data;
	arg.p = ent;
	arg.outs = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeEDParseEdict, &arg);
}

/* SV_LinkEdict (ent, false) -- host_cmd.c:2101. */
static void HostCmd_InvokeSVLinkEdict (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	SV_LinkEdict ((edict_t *)a->p, a->b);
}

int HostCmd_Glue_SVLinkEdict (void *ent, qboolean touch_triggers)
{
	hostcmd_arg_t arg = {0};

	arg.p = ent;
	arg.b = touch_triggers;
	return Host_Guard (HostCmd_InvokeSVLinkEdict, &arg);
}

// -- chunk D (host_cmd.c:2158-2649) -----------------------------------------

/* host_cmd.c:2185, :2372-2373 -- Cvar_Set (name, value), which can reach
 * Host_Error through Cvar_SetQuick -> Cvar_CallCallback. */
static void HostCmd_InvokeCvarSet (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	Cvar_Set (a->s, a->t);
}

int HostCmd_Glue_CvarSet (const char *name, const char *value)
{
	hostcmd_arg_t arg = {0};

	arg.s = name;
	arg.t = value;
	return Host_Guard (HostCmd_InvokeCvarSet, &arg);
}

/* Cmd_ForwardToServer () -- host_cmd.c:922, :984, :1029, :1075, :1131, :1188,
 * :2186, :2210, :2299, :2377, :2665, :3234. Reaches MSG_WriteByte/SZ_Print ->
 * SZ_GetSpace, which is Host_Error-capable on overflow. */
HOSTCMD_GUARD_VOID (CmdForwardToServer, Cmd_ForwardToServer ())

/* host_cmd.c:2438, :2440 -- PR_GetString (sv_player->v.netname), which
 * Host_Errors on an invalid string index (pr_edict_arena.c:307-325). */
static void HostCmd_InvokePRGetString (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outs = PR_GetString (a->i);
}

int HostCmd_Glue_PRGetString (int num, const char **out)
{
	hostcmd_arg_t arg = {0};

	arg.i = num;
	arg.outs = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokePRGetString, &arg);
}

/* host_cmd.c:2521-2527 -- the extended (17-64) spawn-parm write loop's
 * qcvm->globals[g->ofs] = host_client->spawn_parms[i], after ED_FindGlobal has
 * located g. Write-direction mirror of SvMain_Glue_SpawnParmGlobal
 * (sv_main_glue.c:429-435): ED_FindGlobal and a bounded global-array store
 * have no error path, so this is unguarded and void like its counterpart. */
void HostCmd_Glue_SetSpawnParmGlobal (int index, float value)
{
	ddef_t *g = ED_FindGlobal (va ("parm%i", index));
	if (g)
		qcvm->globals[g->ofs] = value;
}

// -- chunk E (host_cmd.c:2650-3298) -----------------------------------------

/* host_cmd.c:2971 (Host_Viewmodel_f): e->v.modelindex = SV_Precache_Model
 * (m->name). Unlike SvMain_Glue_PrecacheModel (whose only caller at
 * sv_main.c:1048 discards the index), this call site needs the slot. */
static void HostCmd_InvokePrecacheModel (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outi = SV_Precache_Model (a->s);
}

int HostCmd_Glue_PrecacheModel (const char *name, int *out)
{
	hostcmd_arg_t arg = {0};

	arg.s = name;
	arg.outi = out;
	*out = 0;
	return Host_Guard (HostCmd_InvokePrecacheModel, &arg);
}

/* host_cmd.c:3007 (PrintFrameName): Mod_Extradata (m) ->
 * Mod_Extradata_CheckSkin -> Mod_LoadModel (mod, true), which reloads from
 * disk when the cache slot was dropped and can Host_Error. */
static void HostCmd_InvokeModExtradata (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	*a->outp = Mod_Extradata ((qmodel_t *)a->p);
}

int HostCmd_Glue_ModExtradata (void *model, void **out)
{
	hostcmd_arg_t arg = {0};

	arg.p = model;
	arg.outp = out;
	*out = NULL;
	return Host_Guard (HostCmd_InvokeModExtradata, &arg);
}

/* host_cmd.c:3109, :3126 (Host_Startdemos_f, Host_Demos_f). */
HOSTCMD_GUARD_VOID (CLNextDemo, CL_NextDemo ())

/* host_cmd.c:3189, :3222 (Host_Serverinfo_f, Host_Setinfo_f). */
static void HostCmd_InvokeSVUpdateInfo (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	SV_UpdateInfo (a->i, a->s, a->t);
}

int HostCmd_Glue_SVUpdateInfo (int edict, const char *keyname, const char *value)
{
	hostcmd_arg_t arg = {0};

	arg.i = edict;
	arg.s = keyname;
	arg.t = value;
	return Host_Guard (HostCmd_InvokeSVUpdateInfo, &arg);
}

/* host_cmd.c:3163 (Info_ClientPrint_Callback): SV_ClientPrintf ("%20s: %s\n",
 * key, val). The Rust callback is handed to Info_Enumerate as a plain
 * void (*) (void *, const char *, const char *) and cannot return a status, so
 * it writes this seam's result through Info_Enumerate's own cbctx pointer,
 * into a Raise that Host_Serverinfo_f / Host_Setinfo_f own on their stack and
 * drain right after the enumeration returns. */
static void HostCmd_InvokeSVClientPrintfKV (void *p)
{
	hostcmd_arg_t *a = (hostcmd_arg_t *)p;
	SV_ClientPrintf ("%20s: %s\n", a->s, a->t);
}

int HostCmd_Glue_SVClientPrintfKV (const char *key, const char *val)
{
	hostcmd_arg_t arg = {0};

	arg.s = key;
	arg.t = val;
	return Host_Guard (HostCmd_InvokeSVClientPrintfKV, &arg);
}

/* host_cmd.c:3203 (Host_Setinfo_f): SV_ClientPrintf ("Your Serverside User
 * Info:\n") -- a fixed literal with no format arguments, kept as its own
 * hardcoded seam rather than routed through a generic "%s" wrapper so the
 * format string byte-for-byte matches the original call site. */
HOSTCMD_GUARD_VOID (SVClientPrintfUserInfoHeader, SV_ClientPrintf ("Your Serverside User Info:\n"))

// ---------------------------------------------------------------------------
// 3. Re-raising from a pure C frame (ADR-009 rule 4).
//
// Two Host_Error sites of host_cmd.c are issued by the port itself rather than
// by a guarded callee, so they cross as negative raise codes and are re-issued
// here. The other three (cannot find/run/restart map) fire inside their own
// HostCmd_Glue_ErrorCannot* guard and arrive as ordinary statuses.

#define HOSTCMD_RAISE_SAVEGAME_VERSION	(-101)
#define HOSTCMD_RAISE_FIRST_TOKEN_BRACE (-102)

static void HostCmd_Raise (int r)
{
	switch (r)
	{
	case HOSTCMD_RAISE_SAVEGAME_VERSION:
		/* host_cmd.c:1879 */
		Host_Error ("Savegame is version %i, not %i", quake_rs_hostcmd_raise_detail (), SAVEGAME_VERSION);
		break;
	case HOSTCMD_RAISE_FIRST_TOKEN_BRACE:
		/* host_cmd.c:2066 */
		Host_Error ("First token isn't a brace");
		break;
	default:
		Host_Reraise (r);
		break;
	}
}

/*
 * 4. T8.3 differential drivers -- the Rust half of
 * tests/host_cmd_differential.rs.
 *
 * host_cmd_ref.c renames host_cmd.c's file-scope objects to c_ref_*, and
 * c_ref_prelude.h renames sv/svs/cl/cls/com_gamedir/cmd_source the same way,
 * so the oracle's server, client and gamedir are distinct objects from the
 * ones quake-capi/src/host_cmd.rs reads. That separation IS the differential:
 * the two implementations hold independent state in one process, so each
 * subject can be driven twice from identical inputs and the outputs compared.
 * Every accessor below is the plain-state twin of a ctest_hostcmd_* accessor
 * at the bottom of host_cmd_ref.c, and the savegame fixture is that file's
 * fixture re-expressed against the plain objects.
 *
 * WHAT IS NOT DUPLICATED, because the prelude does not rename it and both
 * sides therefore already share one object: qcvm and PR_SwitchQCVM (ADR-008's
 * ambient VM pointer, stubs.c:3092), host_client (stubs.c:808), current_skill
 * (stubs.c:6912), and stubs.c's con_log / fog / sky recorders. Only the
 * renamed state is twinned.
 *
 * THE #undefs ARE SCOPED TO THIS SECTION, which defines no HostCmd_Glue_*
 * seam and is textually after all of them, per host_glue_ref.c:654-661.
 */

#undef sv
#undef svs
#undef cl
#undef cls
#undef com_gamedir
#undef cmd_source
#undef Cmd_TokenizeString
#undef PR_ClearEdictStrings
#undef ipv4Available
#undef ipv6Available
#undef my_ipv4_address
#undef my_ipv6_address

extern server_t		   sv;
extern server_static_t svs;
extern client_state_t  cl;
extern client_static_t cls;
extern char			   com_gamedir[MAX_OSPATH];
extern cmd_source_t	   cmd_source;
extern void			   Cmd_TokenizeString (const char *text);

/* net_main.c:47-50. The prelude renames all four (c_ref_prelude.h:173-176)
 * and stubs.c:2978-2981 defines them under the renamed spelling, so the
 * plain names quake-capi's host_cmd.rs reads (Host_Status_f, host_cmd.c:947)
 * have no definition in this link. These are the plain twins, in the
 * #undef + plain-twin shape chase_ref.c:50-94 established. */
char	 my_ipv4_address[NET_NAMELEN];
char	 my_ipv6_address[NET_NAMELEN];
qboolean ipv4Available;
qboolean ipv6Available;

/* pr_edict_arena.c:416, which IS in build.rs's C_SOURCES -- but the prelude
 * renames it (c_ref_prelude.h:619), so the plain name host_cmd.rs:3365 calls
 * is unresolved. A forwarder rather than a second implementation: port and
 * oracle then land on the same arena code by construction. */
extern void c_ref_PR_ClearEdictStrings (void);

void PR_ClearEdictStrings (void)
{
	c_ref_PR_ClearEdictStrings ();
}

/* host_cmd_glue.c:765 and :864. Two section 2b seams call these by name
 * (HOSTCMD_GUARD_VOID (SaveListRebuild, ...) and (HostReconnect_f, ...)), so
 * they are the only members of the flip half this file defines. Both are the
 * one-line forwarders host_cmd_glue.c uses, verbatim. */
void SaveList_Rebuild (void)
{
	quake_rs_hostcmd_save_list_rebuild ();
}

static void Host_Reconnect_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_reconnect_f ());
}

/* ---- driver entry points, paired with host_cmd_ref.c:112-126 ---- */

void ctest_hostcmd_rs_savegame_comment (char *out)
{
	quake_rs_hostcmd_savegame_comment (out);
}

void ctest_hostcmd_rs_savegame_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_savegame_f ());
}

void ctest_hostcmd_rs_loadgame_f (void)
{
	HostCmd_Raise (quake_rs_hostcmd_loadgame_f ());
}

/* ---- savegame fixture, host_cmd_ref.c:137-282 against the plain objects ---
 * The commentary there explains every value; it is not repeated. The only
 * differences are the spellings (plain sv/svs/cl/com_gamedir/cmd_source) and
 * the storage, which is this file's own so the two fixtures cannot alias.
 */

#define CTEST_SAVE_EDICTS 2
#define CTEST_SAVE_EDSIZE ((int)sizeof (edict_t) + 256)

static char		   ctest_rs_save_strings[] = "\0gsave\0health\0";
static ddef_t	   ctest_rs_save_globaldefs[2];
static ddef_t	   ctest_rs_save_fielddefs[2];
static dprograms_t ctest_rs_save_progs;
static float	   ctest_rs_save_globals[1024];
static byte		  *ctest_rs_save_edicts;
static client_t	   ctest_rs_save_client;

static edict_t *ctest_rs_save_edict (int n)
{
	return (edict_t *)(ctest_rs_save_edicts + (size_t)n * (size_t)CTEST_SAVE_EDSIZE);
}

void ctest_hostcmd_rs_setup_savegame (
	const char *gamedir, const char *levelname, const char *mapname, int monsters, int totalmonsters, int skill_value, float qctime)
{
	qcvm_t *vm = &sv.qcvm;
	int		i;

	if (!ctest_rs_save_edicts)
		ctest_rs_save_edicts = (byte *)malloc ((size_t)CTEST_SAVE_EDICTS * (size_t)CTEST_SAVE_EDSIZE);

	memset (&sv, 0, sizeof (sv));
	memset (&cl, 0, sizeof (cl));
	memset (&ctest_rs_save_client, 0, sizeof (ctest_rs_save_client));
	memset (ctest_rs_save_edicts, 0, (size_t)CTEST_SAVE_EDICTS * (size_t)CTEST_SAVE_EDSIZE);
	memset (ctest_rs_save_globals, 0, sizeof (ctest_rs_save_globals));

	q_strlcpy (com_gamedir, gamedir, MAX_OSPATH);
	q_strlcpy (cl.levelname, levelname, sizeof (cl.levelname));
	cl.stats[STAT_MONSTERS] = monsters;
	cl.stats[STAT_TOTALMONSTERS] = totalmonsters;
	cl.intermission = 0;

	sv.active = true;
	sv.nomonsters = false;
	q_strlcpy (sv.name, mapname, sizeof (sv.name));
	sv.lightstyles[0] = "a";
	sv.lightstyles[2] = "mmnn";
	sv.model_precache[1] = "maps/ctest.bsp";
	sv.model_precache[2] = "progs/player.mdl";
	sv.sound_precache[1] = "weapons/r_exp3.wav";
	sv.particle_precache[1] = "tr_rocket";

	ctest_rs_save_globaldefs[0].type = ev_float | DEF_SAVEGLOBAL;
	ctest_rs_save_globaldefs[0].ofs = 30;
	ctest_rs_save_globaldefs[0].s_name = 1; /* "gsave" */
	ctest_rs_save_globaldefs[1].type = ev_float;
	ctest_rs_save_globaldefs[1].ofs = 31;
	ctest_rs_save_globaldefs[1].s_name = 1;
	ctest_rs_save_globals[30] = 3.5f;
	ctest_rs_save_globals[31] = 9.0f;

	ctest_rs_save_fielddefs[0].type = ev_void;
	ctest_rs_save_fielddefs[0].ofs = 0;
	ctest_rs_save_fielddefs[0].s_name = 0;
	ctest_rs_save_fielddefs[1].type = ev_float;
	ctest_rs_save_fielddefs[1].ofs = (unsigned short)(offsetof (entvars_t, health) / 4);
	ctest_rs_save_fielddefs[1].s_name = 7; /* "health" */

	memset (&ctest_rs_save_progs, 0, sizeof (ctest_rs_save_progs));
	ctest_rs_save_progs.numglobaldefs = 2;
	ctest_rs_save_progs.numfielddefs = 2;
	ctest_rs_save_progs.entityfields = (int)(sizeof (entvars_t) / 4);

	memset (vm, 0, sizeof (*vm));
	vm->progs = &ctest_rs_save_progs;
	vm->globaldefs = ctest_rs_save_globaldefs;
	vm->fielddefs = ctest_rs_save_fielddefs;
	vm->globals = ctest_rs_save_globals;
	vm->strings = ctest_rs_save_strings;
	vm->stringssize = (int)sizeof (ctest_rs_save_strings);
	vm->edicts = (edict_t *)ctest_rs_save_edicts;
	vm->edict_size = CTEST_SAVE_EDSIZE;
	vm->num_edicts = CTEST_SAVE_EDICTS;
	vm->max_edicts = CTEST_SAVE_EDICTS;
	vm->time = qctime;
	vm->extfields.alpha = 0;

	for (i = 0; i < CTEST_SAVE_EDICTS; i++)
		ctest_rs_save_edict (i)->free = false;
	ctest_rs_save_edict (1)->v.health = 42.0f;

	ctest_rs_save_client.active = true;
	ctest_rs_save_client.spawned = true;
	ctest_rs_save_client.edict = ctest_rs_save_edict (1);
	for (i = 0; i < NUM_BASIC_SPAWN_PARMS; i++)
		ctest_rs_save_client.spawn_parms[i] = (float)i;
	ctest_rs_save_client.spawn_parms[NUM_BASIC_SPAWN_PARMS] = 7.5f;

	svs.clients = &ctest_rs_save_client;
	svs.maxclients = 1;
	svs.maxclientslimit = 1;
	svs.serverflags = 3;
	host_client = &ctest_rs_save_client;

	current_skill = skill_value;
	cmd_source = src_command;
}

void ctest_hostcmd_rs_set_intermission (int value)
{
	cl.intermission = value;
}

void ctest_hostcmd_rs_set_nomonsters (int value)
{
	sv.nomonsters = value ? true : false;
}

void ctest_hostcmd_rs_set_sv_active (int value)
{
	sv.active = value ? true : false;
}

void ctest_hostcmd_rs_set_maxclients (int value)
{
	svs.maxclients = value;
}

void ctest_hostcmd_rs_set_player_health (float value)
{
	ctest_rs_save_edict (1)->v.health = value;
}

void ctest_hostcmd_rs_set_cmd_source (int value)
{
	cmd_source = (cmd_source_t)value;
}

void ctest_hostcmd_rs_tokenize (const char *text)
{
	Cmd_TokenizeString (text);
}

const char *ctest_hostcmd_rs_get_lastsave (void)
{
	return sv.lastsave;
}

void ctest_hostcmd_rs_clear_qcvm (void)
{
	if (qcvm)
		PR_SwitchQCVM (NULL);
}

int ctest_hostcmd_rs_get_current_skill (void)
{
	return current_skill;
}

void ctest_hostcmd_rs_set_current_skill (int value)
{
	current_skill = value;
}
